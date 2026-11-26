mod edn;
mod names;
mod namespace_map;
mod numbers;
mod regex;
mod uniqueness;

pub use edn::{read_edn, read_edn_recovering, validate_edn};
pub use names::array_class_component;
#[doc(hidden)]
pub use namespace_map::{qualify_namespaced_entries, qualify_namespaced_entries_at};
pub use numbers::{
    IntegerLiteralClass, NumericLiteral, classify_number, classify_number_with_class, decimal_digit,
};
#[doc(hidden)]
pub use uniqueness::{check_resolved_collection, check_resolved_collection_with_namespaces};

use crate::syntax::{Builder, SyntaxKind, SyntaxNode};
use std::collections::{BTreeMap, VecDeque};

#[doc(hidden)]
pub const MAX_NESTING_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Position {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Span {
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug)]
pub struct Form {
    pub kind: Kind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Symbol(String),
    AnonymousArgument(String),
    Keyword(String),
    String(String),
    /// A JVM string with at least one unpaired UTF-16 surrogate.
    Utf16String(Vec<u16>),
    Regex {
        /// The exact group count for regex syntax understood by the reader.
        captures: Option<u16>,
        /// Bits for capturing groups that participate in every successful match.
        required_captures: u32,
    },
    Character(char),
    Number,
    SymbolicNumber(SymbolicNumber),
    Bool(bool),
    Nil,
    List(Vec<Form>),
    AnonymousFunction(AnonymousFunction),
    Vector(Vec<Form>),
    Map(Vec<Form>),
    AutoNamespacedMap {
        alias: Option<String>,
        entries: Vec<Form>,
    },
    Set(Vec<Form>),
    Quote(Box<Form>),
    SyntaxQuote(Box<Form>),
    Unquote {
        splicing: bool,
        form: Box<Form>,
    },
    Metadata(Box<Form>, Box<Form>),
    Tagged(Box<Form>, Box<Form>),
    Conditional {
        splicing: bool,
        form: Box<Form>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SymbolicNumber {
    PositiveInfinity,
    NegativeInfinity,
    NaN,
}

impl SymbolicNumber {
    pub fn spelling(self) -> &'static str {
        match self {
            Self::PositiveInfinity => "##Inf",
            Self::NegativeInfinity => "##-Inf",
            Self::NaN => "##NaN",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AnonymousFunction {
    pub body: Vec<Form>,
    pub required: usize,
    pub variadic: bool,
}

impl Form {
    pub fn without_metadata(&self) -> &Form {
        let mut form = self;
        while let Kind::Metadata(_, target) = &form.kind {
            form = target;
        }
        form
    }

    /// Reader metadata is normalized to maps. Outer entries override inner entries.
    pub fn metadata(&self, key: &str) -> Option<&Form> {
        let mut form = self;
        while let Kind::Metadata(metadata, target) = &form.kind {
            if let Kind::Map(entries) = &metadata.without_metadata().kind
                && let Some(pair) = entries
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .rev()
                    .find(|pair| pair[0].keyword() == Some(key))
            {
                return Some(&pair[1]);
            }
            form = target;
        }
        None
    }

    pub fn symbol(&self) -> Option<&str> {
        match &self.without_metadata().kind {
            Kind::Symbol(s) | Kind::AnonymousArgument(s) => Some(s),
            _ => None,
        }
    }

    pub fn keyword(&self) -> Option<&str> {
        match &self.without_metadata().kind {
            Kind::Keyword(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_string(&self) -> bool {
        matches!(self.kind, Kind::String(_) | Kind::Utf16String(_))
    }

    pub fn vector(&self) -> Option<&[Form]> {
        match &self.without_metadata().kind {
            Kind::Vector(xs) => Some(xs),
            _ => None,
        }
    }

    pub fn map(&self) -> Option<&[Form]> {
        match &self.without_metadata().kind {
            Kind::Map(xs) => Some(xs),
            _ => None,
        }
    }

    /// The head and operand of a quote or unquote reader prefix.
    pub fn prefixed_list(&self) -> Option<(&'static str, &Form)> {
        Some(match &self.without_metadata().kind {
            Kind::Quote(form) => ("quote", form),
            Kind::Unquote { splicing, form } => (
                if *splicing {
                    "clojure.core/unquote-splicing"
                } else {
                    "clojure.core/unquote"
                },
                form,
            ),
            _ => return None,
        })
    }

    /// The operand of a quote prefix or an explicit unqualified quote.
    pub fn quote_operand(&self) -> Option<&Form> {
        match &self.without_metadata().kind {
            Kind::Quote(inner) => Some(inner),
            Kind::List(forms) => match forms.as_slice() {
                [head, inner] if head.symbol() == Some("quote") => Some(inner),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn unquoted(&self) -> &Form {
        self.quote_operand().unwrap_or(self).without_metadata()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadErrorKind {
    Invalid,
    Unchecked,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReadError {
    pub kind: ReadErrorKind,
    pub span: Span,
    pub message: String,
}

impl ReadError {
    pub fn invalid(span: Span, message: impl Into<String>) -> Self {
        Self {
            kind: ReadErrorKind::Invalid,
            span,
            message: message.into(),
        }
    }

    pub fn unchecked(span: Span, message: impl Into<String>) -> Self {
        Self {
            kind: ReadErrorKind::Unchecked,
            span,
            message: message.into(),
        }
    }
}

/// Reader conditionals are forbidden in ordinary .clj/EDN input, preserved for
/// syntax consumers, or selected for an explicit platform feature such as :clj.
#[derive(Clone, Copy, Debug, Default)]
pub enum Conditionals<'a> {
    #[default]
    Reject,
    Preserve,
    Select(&'a str),
    /// Select the first clause whose feature is active, in source order.
    SelectFeatures(&'a [&'a str]),
}

/// Reads semantic forms and rejects reader conditionals.
/// Physical line endings in literals follow the JVM source loader.
pub fn read(source: &str) -> Result<Vec<Form>, ReadError> {
    read_with(source, Conditionals::Reject)
}

/// The same reader with an explicit conditional policy; no project code is executed.
pub fn read_with(source: &str, conditionals: Conditionals<'_>) -> Result<Vec<Form>, ReadError> {
    Reader::new(source, conditionals).read_all()
}

/// Read semantic forms before the first error. A consumer can act on each
/// complete top-level form before reporting the failure.
pub fn read_prefix(source: &str, conditionals: Conditionals<'_>) -> (Vec<Form>, Option<ReadError>) {
    Reader::new(source, conditionals).read_with_first_error(false)
}

/// Reads the semantic forms that survived recovery and the first reader error.
pub fn read_recovering(
    source: &str,
    conditionals: Conditionals<'_>,
) -> (Vec<Form>, Option<ReadError>) {
    Reader::new(source, conditionals).read_with_first_error(true)
}

#[doc(hidden)]
pub struct CheckRead {
    pub forms: Vec<Form>,
    /// Reader data omitted from evaluation that may still need namespace validation.
    pub inactive_namespace_forms: Vec<Form>,
    pub namespace_tokens: Vec<NamespaceToken>,
    /// Resolved map namespaces needed to compare projected keys in enclosing collections.
    pub projected_key_namespaces: BTreeMap<usize, String>,
    pub unchecked_dispatches: Option<UncheckedDispatches>,
    pub comments: Vec<LineComment>,
    pub first_error_read_end: Option<usize>,
    form_read_ends: Vec<(usize, usize)>,
}

impl CheckRead {
    /// The input position after reading a top-level value. A selected reader
    /// conditional can return a span that ends before the enclosing read.
    pub fn end_of_read(&self, form_end: usize) -> usize {
        self.form_read_ends
            .binary_search_by_key(&form_end, |&(end, _)| end)
            .map_or(form_end, |index| self.form_read_ends[index].1)
    }

    pub fn completed_before(&self, offset: usize) -> &[Form] {
        let count = self
            .forms
            .partition_point(|form| self.end_of_read(form.span.end.offset) <= offset);
        &self.forms[..count]
    }
}

#[doc(hidden)]
pub enum NamespaceToken {
    Keyword(Span),
    Map { marker: Span, alias: Option<Span> },
}

#[doc(hidden)]
#[derive(Clone, Default)]
pub struct UncheckedDispatches {
    /// The first possible reader-time call to a tag handler. The handler can
    /// run only after reading the complete payload.
    pub first_tag: Option<Span>,
    /// Top-level forms whose reads finished before the last possible tag call.
    pub completed_before_last_tag: usize,
    /// The earliest dispatch requiring an explicit coverage finding.
    pub first_omitted: Option<OmittedDispatch>,
    /// A regex offset, or the start of its enclosing top-level value if one
    /// was returned. JVM validation can happen anywhere in that read.
    pub regexes: Vec<usize>,
}

#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum OmittedDispatch {
    Tag(Span),
    Regex(Span),
    MetadataTag(Span),
    MetadataRegex(Span),
}

impl OmittedDispatch {
    fn span(self) -> Span {
        match self {
            Self::Tag(span)
            | Self::Regex(span)
            | Self::MetadataTag(span)
            | Self::MetadataRegex(span) => span,
        }
    }

    fn in_metadata(self) -> Self {
        match self {
            Self::Tag(span) => Self::MetadataTag(span),
            Self::Regex(span) => Self::MetadataRegex(span),
            other => other,
        }
    }
}

impl UncheckedDispatches {
    fn omit(&mut self, dispatch: OmittedDispatch) {
        if self
            .first_omitted
            .is_none_or(|earlier| dispatch.span().end.offset < earlier.span().end.offset)
        {
            self.first_omitted = Some(dispatch);
        }
    }
}

#[doc(hidden)]
pub struct LineComment {
    pub span: Span,
    pub following_offset: usize,
}

struct CommentCapture {
    marker: &'static str,
    comments: Vec<LineComment>,
}

#[doc(hidden)]
pub fn read_for_check(
    source: &str,
    conditionals: Conditionals<'_>,
    comment_marker: Option<&'static str>,
) -> (CheckRead, Option<ReadError>) {
    read_for_check_inner(Reader::new(source, conditionals), comment_marker, false)
}

#[doc(hidden)]
pub fn read_externs_for_check(source: &str) -> (CheckRead, Option<ReadError>) {
    let mut reader = Reader::new(source, Conditionals::Reject);
    reader.preserve_literal_newlines = true;
    read_for_check_inner(reader, None, false)
}

/// Continue reading semantic forms for navigation when the project check encounters an error.
/// Return the first error so the caller can keep the check itself strict.
#[doc(hidden)]
pub fn read_for_check_recovering(
    source: &str,
    conditionals: Conditionals<'_>,
    comment_marker: Option<&'static str>,
) -> (CheckRead, Option<ReadError>) {
    read_for_check_inner(Reader::new(source, conditionals), comment_marker, true)
}

fn read_for_check_inner(
    mut reader: Reader<'_>,
    comment_marker: Option<&'static str>,
    recover: bool,
) -> (CheckRead, Option<ReadError>) {
    reader.namespace_tokens = Some(Vec::new());
    reader.unchecked_dispatches = Some(UncheckedDispatches::default());
    reader.comment_capture = comment_marker.map(|marker| CommentCapture {
        marker,
        comments: Vec::new(),
    });
    let (forms, error) = reader.read_with_first_error(recover);
    let mut read = CheckRead {
        forms,
        inactive_namespace_forms: reader.inactive_namespace_forms,
        namespace_tokens: reader.namespace_tokens.unwrap(),
        projected_key_namespaces: reader.projected_key_namespaces,
        unchecked_dispatches: reader
            .unchecked_dispatches
            .filter(|dispatches| dispatches.first_tag.is_some() || !dispatches.regexes.is_empty()),
        form_read_ends: reader.form_read_ends,
        first_error_read_end: reader.first_error_read_end,
        comments: reader
            .comment_capture
            .map_or_else(Vec::new, |capture| capture.comments),
    };
    if let Some(offset) = reader.last_tag_end.checked_sub(1) {
        // If reading the current form ends with the tag, its evaluation
        // starts after the handler has run.
        let completed = read.completed_before(offset).len();
        read.unchecked_dispatches
            .as_mut()
            .expect("read-for-check records active tags")
            .completed_before_last_tag = completed;
    }
    (read, error)
}

#[derive(Debug)]
pub struct Parse {
    green: rowan::GreenNode,
    pub errors: Vec<ReadError>,
}

impl Parse {
    /// Cheap syntax view over the immutable green tree, including trivia and errors.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

/// Retains all source bytes, including invalid and unsupported reader syntax.
pub fn parse(source: &str) -> Parse {
    parse_with(source, Conditionals::Preserve)
}

/// Parses the whole CST with the requested reader conditional policy.
pub fn parse_with(source: &str, conditionals: Conditionals<'_>) -> Parse {
    parse_visiting(source, conditionals, drop)
}

/// Visits recovered top-level forms while building the same lossless tree.
pub fn parse_visiting(
    source: &str,
    conditionals: Conditionals<'_>,
    visit: impl FnMut(Form),
) -> Parse {
    parse_tree(Reader::new(source, conditionals), visit)
}

/// Captures inert EDN syntax, preserving trivia just like the source reader.
pub fn parse_edn(source: &str) -> Parse {
    parse_tree(edn::data_reader(source), drop)
}

fn parse_tree(mut reader: Reader<'_>, visit: impl FnMut(Form)) -> Parse {
    let mut errors = Vec::new();
    reader.visit_forms(true, visit, |error| errors.push(error));
    Parse {
        green: reader.tree.build(),
        errors,
    }
}

struct Reader<'a> {
    source: &'a str,
    pos: Position,
    depth: usize,
    arguments: Option<AnonymousFunction>,
    pending: VecDeque<Form>,
    namespace_tokens: Option<Vec<NamespaceToken>>,
    projected_key_namespaces: BTreeMap<usize, String>,
    unchecked_dispatches: Option<UncheckedDispatches>,
    // None outside a descriptor, Some(None) until a descriptor reads a dispatch.
    metadata_dispatch: Option<Option<OmittedDispatch>>,
    inactive_branch: bool,
    reading_discard: bool,
    last_tag_end: usize,
    first_error_read_end: Option<usize>,
    inactive_namespace_forms: Vec<Form>,
    form_read_ends: Vec<(usize, usize)>,
    comment_capture: Option<CommentCapture>,
    tree: Builder<'a>,
    conditionals: Conditionals<'a>,
    edn: bool,
    preserve_literal_newlines: bool,
    recover: bool,
    recovery_abandoned: bool,
}

enum Item {
    Form(Form),
    Omit,
    Splice { forms: Vec<Form>, span: Span },
}

enum ImmediateSyntaxQuote<'a> {
    Form(&'a Form),
    Nil,
}

struct Delimited {
    start: Position,
    close: char,
    closed: bool,
}

impl Delimited {
    fn new(start: Position, close: char) -> Self {
        Self {
            start,
            close,
            closed: false,
        }
    }
}

impl<'a> Reader<'a> {
    fn new(source: &'a str, conditionals: Conditionals<'a>) -> Self {
        Self {
            source,
            pos: Position {
                offset: 0,
                line: 1,
                column: 1,
            },
            depth: 0,
            arguments: None,
            pending: VecDeque::new(),
            namespace_tokens: None,
            projected_key_namespaces: BTreeMap::new(),
            unchecked_dispatches: None,
            metadata_dispatch: None,
            inactive_branch: false,
            reading_discard: false,
            last_tag_end: 0,
            first_error_read_end: None,
            inactive_namespace_forms: Vec::new(),
            form_read_ends: Vec::new(),
            comment_capture: None,
            tree: Builder::new(source),
            conditionals,
            edn: false,
            preserve_literal_newlines: false,
            recover: false,
            recovery_abandoned: false,
        }
    }

    fn read_all(&mut self) -> Result<Vec<Form>, ReadError> {
        let (forms, error) = self.read_with_first_error(false);
        error.map_or(Ok(forms), Err)
    }

    fn read_with_first_error(&mut self, recover: bool) -> (Vec<Form>, Option<ReadError>) {
        let mut forms = Vec::new();
        let mut first_error = None;
        self.visit_forms(
            recover,
            |form| forms.push(form),
            |error| {
                first_error.get_or_insert(error);
            },
        );
        (forms, first_error)
    }

    fn visit_forms(
        &mut self,
        recover: bool,
        mut emit_form: impl FnMut(Form),
        mut record_error: impl FnMut(ReadError),
    ) {
        self.recover = recover;
        loop {
            let start = self.pos;
            let regexes_before = self
                .unchecked_dispatches
                .as_ref()
                .map_or(0, |dispatches| dispatches.regexes.len());
            match self.next_form() {
                Ok(Some(form)) => {
                    let form_end = form.span.end.offset;
                    if form_end != self.pos.offset && self.namespace_tokens.is_some() {
                        // Successive top-level values come from disjoint reads.
                        self.form_read_ends.push((form_end, self.pos.offset));
                    }
                    if let Some(dispatches) = &mut self.unchecked_dispatches
                        && dispatches.regexes.len() != regexes_before
                    {
                        dispatches.regexes.truncate(regexes_before);
                        dispatches.regexes.push(form.span.start.offset);
                    }
                    emit_form(form);
                }
                Ok(None) => break,
                Err(error) => {
                    self.record_read_failure();
                    record_error(error);
                    if !recover {
                        break;
                    }
                    if self.recovery_abandoned {
                        self.retain_invalid_tail();
                        break;
                    }
                    // Every recovery step must consume input, even at an unsupported construct.
                    if self.pos.offset == start.offset {
                        if self.bump().is_none() {
                            break;
                        }
                        self.token(SyntaxKind::ErrorToken, start);
                    }
                }
            }
        }
    }

    #[cold]
    fn record_read_failure(&mut self) {
        if self.namespace_tokens.is_some() {
            self.first_error_read_end.get_or_insert(self.pos.offset);
        }
    }

    fn next_form(&mut self) -> Result<Option<Form>, ReadError> {
        // A top-level read owns no pending forms from a preceding read.
        self.pending.clear();
        loop {
            self.skip()?;
            if self.peek().is_none() {
                return Ok(None);
            }
            match self.item(false)? {
                Item::Form(form) => return Ok(Some(form)),
                Item::Omit => {}
                Item::Splice { span, .. } => {
                    return Err(self.error(
                        span.start,
                        "Reader conditional splicing is not allowed at top level",
                    ));
                }
            }
        }
    }

    fn start_node(&mut self, start: Position) {
        self.tree.start(start.offset);
    }

    fn finish_node(&mut self, kind: SyntaxKind) {
        self.tree.finish(kind, self.pos.offset);
    }

    fn token(&mut self, kind: SyntaxKind, start: Position) {
        self.tree.token(kind, start.offset..self.pos.offset);
    }

    fn peek(&self) -> Option<char> {
        self.source[self.pos.offset..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos.offset += ch.len_utf8();
        if ch == '\n' || (ch == '\r' && self.peek() != Some('\n')) {
            self.pos.line += 1;
            self.pos.column = 1;
        } else {
            self.pos.column += 1;
        }
        Some(ch)
    }

    fn bump_literal(&mut self) -> Option<char> {
        let ch = self.bump()?;
        // LineNumberingPushbackReader normalizes physical line endings when
        // loading Clojure source. Keep the offset and Rowan token byte-exact.
        if ch != '\r' || self.edn || self.preserve_literal_newlines {
            return Some(ch);
        }
        if self.peek() == Some('\n') {
            self.bump();
        }
        Some('\n')
    }

    fn error(&self, start: Position, message: impl Into<String>) -> ReadError {
        ReadError::invalid(
            Span {
                start,
                end: self.pos,
            },
            message,
        )
    }

    #[cold]
    fn nesting_error(&mut self, start: Position) -> ReadError {
        // Let the recursive reader unwind before keeping the opaque CST tail.
        self.recovery_abandoned = true;
        ReadError::unchecked(
            Span {
                start,
                end: self.pos,
            },
            format!("Reader nesting limit ({MAX_NESTING_DEPTH}) exceeded"),
        )
    }

    fn reader_whitespace(ch: char) -> bool {
        // Both JVM readers use Character.isWhitespace. Commas have a separate token.
        matches!(
            ch,
            '\u{0009}'..='\u{000d}' | '\u{001c}'..=' ' | '\u{1680}'
                | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200a}'
                | '\u{2028}' | '\u{2029}' | '\u{205f}' | '\u{3000}'
        )
    }

    fn skip_space(&mut self) -> bool {
        let start = self.pos;
        match self.peek() {
            Some(c) if Self::reader_whitespace(c) => {
                while self.peek().is_some_and(Self::reader_whitespace) {
                    self.bump();
                }
                self.token(SyntaxKind::Whitespace, start);
            }
            Some(',') => {
                self.bump();
                self.token(SyntaxKind::Comma, start);
            }
            _ => return false,
        }
        true
    }

    // Discards remain syntax in a CST and disappear from semantic forms.
    fn skip(&mut self) -> Result<(), ReadError> {
        let first_comment = self
            .comment_capture
            .as_ref()
            .map_or(0, |capture| capture.comments.len());
        while self.pending.is_empty() {
            if self.skip_space() {
                continue;
            }
            let start = self.pos;
            match self.peek() {
                Some(';') => {
                    while self.peek().is_some_and(|c| !matches!(c, '\r' | '\n')) {
                        self.bump();
                    }
                    self.token(SyntaxKind::Comment, start);
                    if let Some(capture) = &mut self.comment_capture
                        && self.source[start.offset..self.pos.offset]
                            .trim_start_matches(';')
                            .split_whitespace()
                            .next()
                            == Some(capture.marker)
                    {
                        capture.comments.push(LineComment {
                            span: Span {
                                start,
                                end: self.pos,
                            },
                            following_offset: 0,
                        });
                    }
                }
                Some('#') if !self.edn && self.source[start.offset..].starts_with("#!") => {
                    while self.peek().is_some_and(|c| !matches!(c, '\r' | '\n')) {
                        self.bump();
                    }
                    self.token(SyntaxKind::Comment, start);
                }
                Some('#') if self.source[self.pos.offset..].starts_with("#_") => {
                    let top_level = self.depth == 0;
                    self.start_node(start);
                    self.bump();
                    self.bump();
                    self.token(SyntaxKind::DiscardMark, start);
                    let was_discarded = std::mem::replace(&mut self.reading_discard, true);
                    let result = self.without_comments(|reader| {
                        if reader.depth >= MAX_NESTING_DEPTH {
                            return Err(reader.nesting_error(start));
                        }
                        reader.depth += 1;
                        let skipped = reader.skip();
                        reader.depth -= 1;
                        skipped?;
                        if reader.pending.is_empty()
                            && reader.peek().is_none_or(|c| ")]}".contains(c))
                        {
                            return Err(reader.error(start, "Expected a form after #_"));
                        }
                        let form = reader.form()?;
                        reader.retain_inactive_namespace_form(form);
                        Ok(())
                    });
                    self.reading_discard = was_discarded;
                    if top_level {
                        self.pending.clear();
                    }
                    self.finish_node(if result.is_ok() {
                        SyntaxKind::Discard
                    } else {
                        SyntaxKind::Error
                    });
                    result?;
                }
                _ => break,
            }
        }
        if let Some(capture) = &mut self.comment_capture {
            let offset = self
                .pending
                .front()
                .map_or(self.pos.offset, |form| form.span.start.offset);
            for comment in &mut capture.comments[first_comment..] {
                comment.following_offset = offset;
            }
        }
        Ok(())
    }

    fn without_comments<T>(&mut self, read: impl FnOnce(&mut Self) -> T) -> T {
        let capture = self.comment_capture.take();
        let result = read(self);
        self.comment_capture = capture;
        result
    }

    fn retain_inactive_namespace_form(&mut self, form: Form) {
        if self.namespace_tokens.is_some()
            && !matches!(&form.kind, Kind::Keyword(_) | Kind::Symbol(_))
            && self.source[form.span.start.offset..form.span.end.offset].contains("::")
        {
            self.inactive_namespace_forms.push(form);
        }
    }

    fn form(&mut self) -> Result<Form, ReadError> {
        loop {
            match self.item(true)? {
                Item::Form(form) => return Ok(form),
                Item::Omit => {}
                Item::Splice { forms, .. } => self.queue_splice(forms),
            }
        }
    }

    fn queue_splice(&mut self, forms: Vec<Form>) {
        for form in forms.into_iter().rev() {
            self.pending.push_front(form);
        }
    }

    fn item(&mut self, required: bool) -> Result<Item, ReadError> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(self.nesting_error(self.pos));
        }
        self.depth += 1;
        // Top-level and collection reads already skipped trivia and discards.
        // A required operand starts immediately after its prefix.
        if required && let Err(error) = self.skip() {
            self.depth -= 1;
            return Err(error);
        }
        if let Some(form) = self.pending.pop_front() {
            self.depth -= 1;
            return Ok(Item::Form(form));
        }
        let start = self.pos;
        // A missing prefixed operand must leave the enclosing delimiter available.
        if self.recover
            && required
            && let Some(close @ (')' | ']' | '}')) = self.peek()
        {
            self.depth -= 1;
            return Err(Self::closing_delimiter_error(start, close));
        }
        self.start_node(start);
        let result = if self.edn && self.edn_dispatch_prefix() {
            self.edn_dispatch(start)
        } else {
            match self.bump() {
                // A conditional recursively reads its feature and branch forms.
                // Keep the full form dispatch frame off that recursive path.
                Some('#') if self.peek() == Some('?') => self.conditional(start),
                first => self.form_inner(start, first),
            }
        };
        self.depth -= 1;
        self.tree.finish_item(result.is_ok(), self.pos.offset);
        result
    }

    #[cold]
    fn closing_delimiter_error(start: Position, close: char) -> ReadError {
        ReadError::invalid(
            Span {
                start,
                end: Position {
                    offset: start.offset + 1,
                    column: start.column + 1,
                    ..start
                },
            },
            format!("Unexpected closing delimiter {close}"),
        )
    }

    fn form_inner(&mut self, start: Position, first: Option<char>) -> Result<Item, ReadError> {
        let kind = match first {
            Some('(') => {
                self.token(SyntaxKind::OpenParen, start);
                Kind::List(self.collection(')', start)?)
            }
            Some('[') => {
                self.token(SyntaxKind::OpenBracket, start);
                Kind::Vector(self.collection(']', start)?)
            }
            Some('{') => Kind::Map(self.map(start)?),
            Some('"') => match self.string(start) {
                Ok(string) => {
                    self.token(SyntaxKind::String, start);
                    string
                }
                Err(error) => {
                    if self.recover {
                        self.record_read_failure();
                        self.finish_invalid_string(start);
                    }
                    return Err(error);
                }
            },
            Some('\\') => {
                let character = self.character(start)?;
                self.token(SyntaxKind::Character, start);
                Kind::Character(character)
            }
            Some('^') => {
                self.token(SyntaxKind::MetadataMark, start);
                self.metadata_form(start)?
            }
            Some('\'') if !self.edn => {
                self.token(SyntaxKind::QuoteMark, start);
                Kind::Quote(Box::new(self.without_comments(Self::form)?))
            }
            Some('`') if !self.edn => return self.syntax_quote(start),
            Some('~') if !self.edn => return self.unquote(start),
            Some('@') if !self.edn => {
                return self.prefix_call(start, SyntaxKind::DerefMark, "clojure.core/deref");
            }
            Some('#') if self.peek() == Some('\'') && !self.edn => {
                self.bump();
                return self.prefix_call(start, SyntaxKind::VarQuoteMark, "var");
            }
            Some('#') if self.peek() == Some('^') => {
                self.bump();
                self.token(SyntaxKind::MetadataMark, start);
                self.metadata_form(start)?
            }
            Some('#') if self.peek() == Some('#') => self.symbolic_number(start)?,
            Some('#') if self.peek() == Some(':') => self.namespaced_map(start)?,
            Some('#') if self.peek() == Some('(') => self.anonymous_function(start)?,
            Some('#') if self.peek() == Some('"') => {
                let regex = self.regex(start)?;
                self.token(SyntaxKind::Regex, start);
                regex
            }
            Some('#') if self.peek() == Some('{') => {
                return self.set(start);
            }
            Some('#') if !self.edn && self.peek() == Some('=') => {
                return self.reject_read_eval(start);
            }
            Some('#') if !self.edn && self.peek() == Some('<') => {
                self.bump();
                return Err(self.error(start, "Unreadable form"));
            }
            Some('#') if !self.edn => self.tagged(start)?,
            Some('#') if self.peek().is_some_and(char::is_alphabetic) => self.edn_tag(start)?,
            Some(c @ (')' | ']' | '}')) => {
                return Err(self.error(start, format!("Unexpected closing delimiter {c}")));
            }
            Some(c @ ('#' | '`' | '~' | '@')) => {
                return Err(self.error(start, format!("Reader syntax {c} is not supported yet")));
            }
            Some(_) => return self.atom(start),
            None => return Err(self.error(start, "Unexpected end of input")),
        };
        Ok(self.form_item(start, kind))
    }

    fn form_item(&self, start: Position, kind: Kind) -> Item {
        Item::Form(Form {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        })
    }

    fn tagged(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.tree.mark_dispatch(SyntaxKind::Tagged);
        self.token(SyntaxKind::DispatchMark, start);
        let tag = self
            .without_comments(Self::form)
            .map_err(|error| self.recover_prefix_operand(error))?;
        if tag.symbol().is_none() {
            let error = if let Some(span) = self.data_reader_result_span(&tag) {
                self.retain_inactive_namespace_form(tag);
                ReadError::unchecked(
                    span,
                    "Tag name depends on the value returned by a data reader",
                )
            } else {
                self.error(start, "Reader tag must be a symbol")
            };
            return Err(self.recover_prefix_operand(error));
        }
        let value = self.without_comments(Self::form)?;
        if !self.inactive_branch {
            self.last_tag_end = self.pos.offset;
            if let Some(dispatches) = &mut self.unchecked_dispatches {
                let span = Span {
                    start,
                    end: self.pos,
                };
                dispatches.first_tag.get_or_insert(span);
                self.observe_dispatch(OmittedDispatch::Tag(span), self.reading_discard);
            }
        }
        Ok(Kind::Tagged(Box::new(tag), Box::new(value)))
    }

    fn data_reader_result_span(&self, form: &Form) -> Option<Span> {
        let value = form.without_metadata();
        (!self.inactive_branch && matches!(value.kind, Kind::Tagged(..))).then_some(value.span)
    }

    fn observe_dispatch(&mut self, dispatch: OmittedDispatch, omitted: bool) {
        if let Some(descriptor) = &mut self.metadata_dispatch {
            descriptor.get_or_insert(dispatch);
        }
        if omitted && let Some(dispatches) = &mut self.unchecked_dispatches {
            dispatches.omit(dispatch);
        }
    }

    #[cold]
    fn reject_read_eval(&mut self, start: Position) -> Result<Item, ReadError> {
        self.bump(); // =
        self.token(SyntaxKind::DispatchMark, start);
        let error = ReadError::unchecked(
            Span {
                start,
                end: self.pos,
            },
            "Read-time evaluation #= is unavailable",
        );
        Err(self.recover_prefix_operand(error))
    }

    #[cold]
    fn recover_prefix_operand(&mut self, error: ReadError) -> ReadError {
        // A malformed prefix still owns its operand. Keep that operand in
        // the error node so recovery cannot publish it as a sibling form.
        if self.recover && !self.recovery_abandoned {
            self.record_read_failure();
            let namespace_tokens = self.namespace_tokens.take();
            let _ = self.without_comments(Self::form);
            self.namespace_tokens = namespace_tokens;
        }
        error
    }

    fn set(&mut self, start: Position) -> Result<Item, ReadError> {
        self.bump(); // {
        self.token(SyntaxKind::OpenSet, start);
        let values = self.collection('}', start)?;
        if let Err(error) = uniqueness::check_set(
            &values,
            self.source,
            self.last_tag_end <= start.offset,
            &self.projected_key_namespaces,
        ) {
            self.retain_rejected_literal(start, Kind::Set(values));
            return Err(error);
        }
        Ok(self.form_item(start, Kind::Set(values)))
    }

    fn map(&mut self, start: Position) -> Result<Vec<Form>, ReadError> {
        self.token(SyntaxKind::OpenBrace, start);
        let entries = self.collection('}', start)?;
        if !self.preserves_map_entries(&entries) {
            if !entries.len().is_multiple_of(2) {
                for form in entries {
                    self.retain_inactive_namespace_form(form);
                }
                return Err(self.error(start, "Map literal requires an even number of forms"));
            }
            if let Err(error) = uniqueness::check_map(
                &entries,
                self.source,
                self.last_tag_end <= start.offset,
                &self.projected_key_namespaces,
            ) {
                self.retain_rejected_literal(start, Kind::Map(entries));
                return Err(error);
            }
        }
        Ok(entries)
    }

    #[cold]
    fn retain_rejected_literal(&mut self, start: Position, kind: Kind) {
        self.retain_inactive_namespace_form(Form {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        });
    }

    fn preserves_map_entries(&self, entries: &[Form]) -> bool {
        matches!(self.conditionals, Conditionals::Preserve)
            && entries
                .iter()
                .any(|form| matches!(form.kind, Kind::Conditional { .. }))
    }

    fn symbolic_number(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.bump(); // #
        self.tree.mark_dispatch(SyntaxKind::SymbolicNumber);
        self.token(SyntaxKind::DispatchMark, start);
        let value = self.without_comments(Self::form)?;
        if let Some(span) = self.data_reader_result_span(&value) {
            self.retain_inactive_namespace_form(value);
            return Err(ReadError::unchecked(
                span,
                "Symbolic number depends on the value returned by a data reader",
            ));
        }
        let number = match value.symbol() {
            Some("Inf") => SymbolicNumber::PositiveInfinity,
            Some("-Inf") => SymbolicNumber::NegativeInfinity,
            Some("NaN") => SymbolicNumber::NaN,
            _ => return Err(self.error(start, "Unknown symbolic value")),
        };
        Ok(Kind::SymbolicNumber(number))
    }

    fn atom(&mut self, start: Position) -> Result<Item, ReadError> {
        let numeric = numbers::starts_number(&self.source[start.offset..]);
        while self
            .peek()
            .is_some_and(|c| !(Self::token_boundary(c) || numeric && matches!(c, '\'' | '#' | '%')))
        {
            self.bump();
        }
        let token = &self.source[start.offset..self.pos.offset];
        let (kind, syntax) = match token {
            arg if self.arguments.is_some() && arg.starts_with('%') => {
                (self.anonymous_argument(arg, start)?, SyntaxKind::Symbol)
            }
            "nil" => (Kind::Nil, SyntaxKind::Nil),
            "true" => (Kind::Bool(true), SyntaxKind::Boolean),
            "false" => (Kind::Bool(false), SyntaxKind::Boolean),
            s if s.starts_with(':') => {
                if !names::valid_name(s, self.edn) {
                    self.token(SyntaxKind::Keyword, start);
                    return Err(self.error(start, "Invalid keyword literal"));
                }
                if self.edn && s.starts_with("::") {
                    return Err(self.error(start, "Auto-resolved keywords are unavailable in EDN"));
                }
                if s.starts_with("::")
                    && let Some(tokens) = &mut self.namespace_tokens
                {
                    tokens.push(NamespaceToken::Keyword(Span {
                        start,
                        end: self.pos,
                    }));
                }
                (Kind::Keyword(s.to_string()), SyntaxKind::Keyword)
            }
            s if numeric => {
                if !numbers::valid_number(s) {
                    return Err(self.error(start, "Invalid numeric literal"));
                }
                (Kind::Number, SyntaxKind::Number)
            }
            s if names::valid_name(s, self.edn) => {
                (Kind::Symbol(s.to_string()), SyntaxKind::Symbol)
            }
            _ => {
                self.token(SyntaxKind::Symbol, start);
                return Err(self.error(start, "Invalid symbol literal"));
            }
        };
        self.token(syntax, start);
        Ok(self.form_item(start, kind))
    }

    fn prefix_call(
        &mut self,
        start: Position,
        syntax: SyntaxKind,
        operator: &str,
    ) -> Result<Item, ReadError> {
        self.token(syntax, start);
        let head = Form {
            kind: Kind::Symbol(operator.into()),
            span: Span {
                start,
                end: self.pos,
            },
        };
        let operand = self.without_comments(Self::form)?;
        Ok(self.form_item(start, Kind::List(vec![head, operand])))
    }

    fn syntax_quote(&mut self, start: Position) -> Result<Item, ReadError> {
        self.token(SyntaxKind::SyntaxQuoteMark, start);
        let form = self.without_comments(Self::form)?;
        let template = form.without_metadata();
        let value = match Self::immediate_syntax_quote_value(template) {
            Some(ImmediateSyntaxQuote::Form(value)) => value,
            _ => template,
        };
        let splicing = match &value.without_metadata().kind {
            Kind::Unquote { splicing, .. } => *splicing,
            Kind::List(items) => {
                items.first().and_then(Form::symbol) == Some("clojure.core/unquote-splicing")
            }
            _ => false,
        };
        if splicing {
            return Err(self.error(
                value.span.start,
                "Unquote splicing requires a surrounding collection",
            ));
        }
        Ok(self.form_item(start, Kind::SyntaxQuote(Box::new(form))))
    }

    fn unquote(&mut self, start: Position) -> Result<Item, ReadError> {
        let splicing = self.peek() == Some('@');
        let mark = if splicing {
            self.bump();
            SyntaxKind::UnquoteSplicingMark
        } else {
            SyntaxKind::UnquoteMark
        };
        self.token(mark, start);
        let form = Box::new(self.without_comments(Self::form)?);
        Ok(self.form_item(start, Kind::Unquote { splicing, form }))
    }

    fn anonymous_function(&mut self, start: Position) -> Result<Kind, ReadError> {
        if self.arguments.is_some() {
            return Err(self.error(start, "Nested #() forms are not allowed"));
        }
        self.bump(); // (
        self.token(SyntaxKind::OpenAnonymousFunction, start);
        self.arguments = Some(AnonymousFunction::default());
        let body = self.collection(')', start);
        let mut function = self.arguments.take().unwrap();
        function.body = body?;
        Ok(Kind::AnonymousFunction(function))
    }

    fn anonymous_argument(&mut self, token: &str, start: Position) -> Result<Kind, ReadError> {
        if token == "%&" {
            self.arguments.as_mut().unwrap().variadic = true;
            return Ok(Kind::AnonymousArgument(token.into()));
        }
        let index = if token == "%" {
            1
        } else {
            let digits = &token[1..];
            if digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(self.error(start, "Argument literal must be %, %& or %integer"));
            }
            digits.parse::<i32>().map_err(|_| {
                self.error(start, "Argument index exceeds the JVM signed 32-bit range")
            })? as usize
        };
        let function = self.arguments.as_mut().unwrap();
        function.required = function.required.max(index);
        // % and %1 are two spellings of one binding inside this reader scope.
        Ok(Kind::AnonymousArgument(
            if token == "%" { "%1" } else { token }.into(),
        ))
    }

    fn token_boundary(c: char) -> bool {
        // Quote, # and % terminate only at the start of a form in Clojure.
        Self::reader_whitespace(c) || "()[]{}\";,`~@^\\".contains(c)
    }

    fn metadata_form(&mut self, start: Position) -> Result<Kind, ReadError> {
        let parent_dispatch = self.metadata_dispatch.take();
        self.metadata_dispatch = self.unchecked_dispatches.as_ref().map(|_| None);
        let descriptor = self.without_comments(Self::form);
        let dispatch = self.metadata_dispatch.take().flatten();
        self.metadata_dispatch = parent_dispatch.map(|first| first.or(dispatch));
        let mut metadata = descriptor.map_err(|error| self.recover_prefix_operand(error))?;
        let projection = Self::immediate_syntax_quote_value(&metadata);
        if let Some(value) = projection {
            let kind = match value {
                ImmediateSyntaxQuote::Form(value) => value.kind.clone(),
                ImmediateSyntaxQuote::Nil => Kind::Nil,
            };
            // Explicit unquote ignores subsequent operands after reading them.
            // The raw descriptor can still contain contextual reader errors.
            let raw = std::mem::replace(&mut metadata.kind, kind);
            self.retain_inactive_namespace_form(Form {
                kind: raw,
                span: metadata.span,
            });
        }
        let tagged = self.data_reader_result_span(&metadata).is_some();
        if let Some(dispatch) = dispatch
            && let Some(dispatches) = &mut self.unchecked_dispatches
        {
            dispatches.omit(dispatch.in_metadata());
        }
        let shorthand = match &metadata.without_metadata().kind {
            Kind::Map(_) | Kind::AutoNamespacedMap { .. } => None,
            Kind::Tagged(..) if tagged => None,
            Kind::Conditional { .. } if matches!(self.conditionals, Conditionals::Preserve) => None,
            Kind::Keyword(key) => Some((key.clone(), true)),
            Kind::Symbol(_)
            | Kind::AnonymousArgument(_)
            | Kind::String(_)
            | Kind::Utf16String(_) => Some((":tag".into(), false)),
            Kind::Vector(_) if !self.edn => Some((":param-tags".into(), false)),
            _ => {
                let error = self.error(
                    start,
                    "Metadata requires a map, keyword, symbol, string or vector",
                );
                return Err(self.recover_prefix_operand(error));
            }
        };
        if let Some((key, flag)) = shorthand {
            let span = metadata.span;
            let value = if flag {
                Form {
                    kind: Kind::Bool(true),
                    span,
                }
            } else {
                metadata
            };
            metadata = Form {
                kind: Kind::Map(vec![
                    Form {
                        kind: Kind::Keyword(key),
                        span,
                    },
                    value,
                ]),
                span,
            };
        }
        let target = self.form()?;
        let raw_target = target.without_metadata();
        let value = match Self::immediate_syntax_quote_value(raw_target) {
            Some(ImmediateSyntaxQuote::Form(value)) => &value.without_metadata().kind,
            Some(ImmediateSyntaxQuote::Nil) => &Kind::Nil,
            None => &raw_target.kind,
        };
        if !matches!(
            value,
            Kind::Symbol(_)
                | Kind::AnonymousArgument(_)
                | Kind::AnonymousFunction(_)
                | Kind::List(_)
                | Kind::Vector(_)
                | Kind::Map(_)
                | Kind::AutoNamespacedMap { .. }
                | Kind::Set(_)
                | Kind::Quote(_)
                | Kind::SyntaxQuote(_)
                | Kind::Unquote { .. }
                | Kind::Conditional { .. }
        ) && (self.inactive_branch || !matches!(value, Kind::Tagged(..)))
        {
            return Err(self.error(
                start,
                "Metadata requires a symbol, collection or metadata-capable tagged value",
            ));
        }
        Ok(Kind::Metadata(Box::new(metadata), Box::new(target)))
    }

    // Syntax quote can return an operand verbatim. Metadata can use that
    // value without resolving symbols or expanding a template.
    fn immediate_syntax_quote_value(form: &Form) -> Option<ImmediateSyntaxQuote<'_>> {
        let Kind::SyntaxQuote(template) = &form.without_metadata().kind else {
            return None;
        };
        let value = match Self::immediate_syntax_quote_value(template) {
            Some(ImmediateSyntaxQuote::Form(value)) => value,
            // A surrounding syntax quote constructs (quote nil).
            Some(ImmediateSyntaxQuote::Nil) => return None,
            None => template,
        };
        match &value.without_metadata().kind {
            Kind::Number
            | Kind::SymbolicNumber(_)
            | Kind::Character(_)
            | Kind::String(_)
            | Kind::Utf16String(_)
            | Kind::Keyword(_) => Some(ImmediateSyntaxQuote::Form(value)),
            Kind::Unquote {
                form,
                splicing: false,
            } => Some(ImmediateSyntaxQuote::Form(form)),
            Kind::List(items)
                if items.first().and_then(Form::symbol) == Some("clojure.core/unquote") =>
            {
                Some(
                    items
                        .get(1)
                        .map_or(ImmediateSyntaxQuote::Nil, ImmediateSyntaxQuote::Form),
                )
            }
            _ => None,
        }
    }

    fn character(&mut self, start: Position) -> Result<char, ReadError> {
        let token_start = self.pos.offset;
        let first = self
            .bump_literal()
            .ok_or_else(|| self.error(start, "Expected a character after backslash"))?;
        let first_end = self.pos.offset;
        while self.peek().is_some_and(|c| !Self::token_boundary(c)) {
            self.bump();
        }
        let value = if first_end == self.pos.offset {
            (u32::from(first) <= 0xffff).then_some(first)
        } else {
            let token = &self.source[token_start..self.pos.offset];
            match token {
                "newline" => Some('\n'),
                "space" => Some(' '),
                "tab" => Some('\t'),
                "return" => Some('\r'),
                "backspace" => Some('\u{0008}'),
                "formfeed" => Some('\u{000c}'),
                _ => {
                    let decoded = if let Some(digits) = token.strip_prefix('u') {
                        Self::character_digits(digits, 16, 4..=4)
                    } else if let Some(digits) = token.strip_prefix('o') {
                        Self::character_digits(digits, 8, 1..=3).filter(|&n| n <= 0xff)
                    } else {
                        None
                    };
                    decoded.and_then(char::from_u32)
                }
            }
        };
        value.ok_or_else(|| self.error(start, "Invalid JVM character literal"))
    }

    fn character_digits(
        digits: &str,
        radix: u32,
        width: std::ops::RangeInclusive<usize>,
    ) -> Option<u32> {
        if !width.contains(&digits.chars().count()) {
            return None;
        }
        digits.chars().try_fold(0, |value, ch| {
            Some(value * radix + numbers::escape_digit(ch, radix)?)
        })
    }

    fn collection(&mut self, end: char, start: Position) -> Result<Vec<Form>, ReadError> {
        let mut bounds = Delimited::new(start, end);
        let mut forms = Vec::new();
        loop {
            match self.collection_item(&mut bounds) {
                Ok(Some(Item::Form(form))) => forms.push(form),
                Ok(Some(Item::Omit)) => {}
                Ok(Some(Item::Splice { forms: values, .. })) => forms.extend(values),
                Ok(None) => return Ok(forms),
                Err(error) => {
                    for form in forms {
                        self.retain_inactive_namespace_form(form);
                    }
                    if self.recover && !self.recovery_abandoned {
                        self.recover_delimited(&mut bounds);
                    }
                    return Err(error);
                }
            }
        }
    }

    #[cold]
    fn recover_delimited(&mut self, bounds: &mut Delimited) {
        self.record_read_failure();
        if !bounds.closed {
            self.without_comments(|reader| reader.recover_collection(bounds));
        }
    }

    fn recover_collection(&mut self, bounds: &mut Delimited) {
        while !self.recovery_abandoned && self.peek().is_some() {
            let start = self.pos;
            match self.collection_item(bounds) {
                Ok(None) => break,
                Err(_) if self.pos == start => {
                    self.retain_invalid_tail();
                    break;
                }
                _ => {}
            }
        }
    }

    fn retain_invalid_tail(&mut self) {
        let start = self.pos;
        while self.bump().is_some() {}
        if self.pos != start {
            self.token(SyntaxKind::ErrorToken, start);
        }
    }

    fn collection_item(&mut self, bounds: &mut Delimited) -> Result<Option<Item>, ReadError> {
        self.skip()?;
        if let Some(form) = self.pending.pop_front() {
            return Ok(Some(Item::Form(form)));
        }
        match self.peek() {
            Some(c) if c == bounds.close => {
                let close = self.pos;
                self.bump();
                bounds.closed = true;
                self.token(
                    match c {
                        ')' => SyntaxKind::CloseParen,
                        ']' => SyntaxKind::CloseBracket,
                        '}' => SyntaxKind::CloseBrace,
                        _ => unreachable!(),
                    },
                    close,
                );
                Ok(None)
            }
            None => Err(self.error(
                bounds.start,
                format!("Unclosed form; expected {}", bounds.close),
            )),
            _ => self.item(false).map(Some),
        }
    }

    fn conditional(&mut self, start: Position) -> Result<Item, ReadError> {
        self.bump(); // ?
        let splicing = self.peek() == Some('@');
        if splicing {
            self.bump();
        }
        self.token(SyntaxKind::ConditionalMark, start);
        // The dispatch marker only accepts reader whitespace before its list.
        while self.skip_space() {}
        if self.peek() != Some('(') {
            let error = self.error(start, "Reader conditional requires a list");
            return Err(self.recover_prefix_operand(error));
        }
        match self.conditionals {
            Conditionals::Reject => {
                let error = self.error(
                    start,
                    "Reader conditionals require .cljc input or an explicit reader policy",
                );
                return Err(self.recover_prefix_operand(error));
            }
            Conditionals::Preserve => {
                let form = Box::new(self.form()?);
                return Ok(Item::Form(Form {
                    kind: Kind::Conditional { splicing, form },
                    span: Span {
                        start,
                        end: self.pos,
                    },
                }));
            }
            Conditionals::Select(_) | Conditionals::SelectFeatures(_) => {}
        }
        let open = self.pos;
        self.start_node(open);
        self.bump(); // (
        self.token(SyntaxKind::OpenParen, open);
        let mut bounds = Delimited::new(start, ')');
        let result = self.select_conditional(&mut bounds, splicing);
        if result.is_err() && self.recover && !self.recovery_abandoned {
            self.recover_delimited(&mut bounds);
        }
        self.finish_node(if result.is_ok() {
            SyntaxKind::List
        } else {
            SyntaxKind::Error
        });
        result
    }

    fn select_conditional(
        &mut self,
        bounds: &mut Delimited,
        splicing: bool,
    ) -> Result<Item, ReadError> {
        let start = bounds.start;
        loop {
            let Some(key) = self.without_comments(|reader| reader.conditional_form(bounds))? else {
                return Ok(Item::Omit);
            };
            let Some(key_name) = key.keyword() else {
                if let Some(span) = self.data_reader_result_span(&key) {
                    self.retain_inactive_namespace_form(key);
                    return Err(ReadError::unchecked(
                        span,
                        "Reader conditional feature depends on the value returned by a data reader",
                    ));
                }
                return Err(self.error(
                    key.span.start,
                    "Reader conditional feature must be a keyword",
                ));
            };
            if matches!(key_name, ":else" | ":none") {
                return Err(self.error(key.span.start, "Reader conditional feature is reserved"));
            }
            let selected = key_name == ":default"
                || match self.conditionals {
                    Conditionals::Select(feature) => key_name == feature,
                    Conditionals::SelectFeatures(features) => features.contains(&key_name),
                    _ => false,
                };
            let value = if selected {
                self.conditional_form(bounds)?
            } else {
                self.in_inactive_branch(|reader| reader.conditional_form(bounds))?
            };
            let Some(value) = value else {
                return if selected {
                    Err(self.error(
                        start,
                        "Reader conditional requires a value for each feature",
                    ))
                } else {
                    Ok(Item::Omit)
                };
            };
            if selected {
                return self.finish_selected_conditional(bounds, value, splicing);
            }
            self.retain_inactive_namespace_form(value);
        }
    }

    fn finish_selected_conditional(
        &mut self,
        bounds: &mut Delimited,
        value: Form,
        splicing: bool,
    ) -> Result<Item, ReadError> {
        // Still read the tail for syntax errors. Nested conditionals keep
        // their selection semantics even in inactive branches.
        let tail = self.in_inactive_branch(|reader| {
            while let Some(item) = reader.collection_item(bounds)? {
                match item {
                    Item::Form(form) => reader.retain_inactive_namespace_form(form),
                    Item::Splice { forms, .. } => {
                        for form in forms {
                            reader.retain_inactive_namespace_form(form);
                        }
                    }
                    Item::Omit => {}
                }
            }
            Ok(())
        });
        if let Err(error) = tail {
            self.retain_inactive_namespace_form(value);
            return Err(error);
        }
        if !splicing {
            return Ok(Item::Form(value));
        }
        let span = Span {
            start: bounds.start,
            end: self.pos,
        };
        let forms = self.splice_conditional_value(value, span)?;
        Ok(Item::Splice { forms, span })
    }

    fn splice_conditional_value(
        &mut self,
        mut value: Form,
        conditional: Span,
    ) -> Result<Vec<Form>, ReadError> {
        let written = value.span;
        let invalid = || {
            ReadError::invalid(
                written,
                "Splicing reader conditional requires a list or vector",
            )
        };
        loop {
            match value.kind {
                Kind::Metadata(metadata, target) => {
                    self.retain_inactive_namespace_form(*metadata);
                    value = *target;
                }
                Kind::SyntaxQuote(template)
                    if matches!(
                        template.kind,
                        Kind::Unquote {
                            splicing: false,
                            ..
                        }
                    ) =>
                {
                    let Kind::Unquote { form, .. } = template.kind else {
                        unreachable!("direct unquote supplies its already-read operand");
                    };
                    value = *form;
                }
                Kind::SyntaxQuote(ref template) => {
                    if let Some(projected) = Self::immediate_syntax_quote_value(&value) {
                        let ImmediateSyntaxQuote::Form(projected) = projected else {
                            return Err(invalid());
                        };
                        let projected = projected.clone();
                        self.retain_inactive_namespace_form(value);
                        value = projected;
                    } else {
                        let start = value.span.start;
                        return match &template.kind {
                            Kind::Bool(_) | Kind::Nil => {
                                Ok(vec![Self::splice_head(start, 1, "quote"), template.as_ref().clone()])
                            }
                            Kind::List(items) if items.is_empty() => {
                                Ok(vec![Self::splice_head(start, 1, "clojure.core/list")])
                            }
                            _ => Err(self.unchecked_splice(
                                value,
                                conditional,
                                "Splicing reader conditional requires expansion of the syntax quote",
                            )),
                        };
                    }
                }
                _ => break,
            }
        }
        if let Some((head, _)) = value.prefixed_list() {
            let width = if matches!(value.kind, Kind::Unquote { splicing: true, .. }) {
                2
            } else {
                1
            };
            let operator = Self::splice_head(value.span.start, width, head);
            let (Kind::Quote(operand) | Kind::Unquote { form: operand, .. }) = value.kind else {
                unreachable!("reader prefix supplies an operand");
            };
            return Ok(vec![operator, *operand]);
        }
        match value.kind {
            Kind::List(forms) | Kind::Vector(forms) => Ok(forms),
            Kind::Tagged(..) if !self.inactive_branch => Err(self.unchecked_splice(
                value,
                conditional,
                "Splicing reader conditional depends on the value returned by a data reader",
            )),
            Kind::AnonymousFunction(_) => Err(self.unchecked_splice(
                value,
                conditional,
                "Splicing reader conditional requires expansion of the anonymous function",
            )),
            _ => {
                self.retain_inactive_namespace_form(value);
                Err(invalid())
            }
        }
    }

    fn splice_head(start: Position, width: usize, symbol: &str) -> Form {
        Form {
            kind: Kind::Symbol(symbol.into()),
            span: Span {
                start,
                end: Position {
                    offset: start.offset + width,
                    column: start.column + width,
                    ..start
                },
            },
        }
    }

    #[cold]
    fn unchecked_splice(&mut self, value: Form, conditional: Span, message: &str) -> ReadError {
        let span = value.span;
        self.retain_inactive_namespace_form(value);
        if self.depth == 1 && !self.reading_discard {
            ReadError::invalid(
                conditional,
                "Reader conditional splicing is not allowed at top level",
            )
        } else {
            ReadError::unchecked(span, message)
        }
    }

    fn conditional_form(&mut self, bounds: &mut Delimited) -> Result<Option<Form>, ReadError> {
        loop {
            match self.collection_item(bounds)? {
                Some(Item::Form(form)) => return Ok(Some(form)),
                Some(Item::Omit) => {}
                Some(Item::Splice { forms, .. }) => self.queue_splice(forms),
                None => return Ok(None),
            }
        }
    }

    fn in_inactive_branch<T>(&mut self, read: impl FnOnce(&mut Self) -> T) -> T {
        let was_inactive = std::mem::replace(&mut self.inactive_branch, true);
        let item = self.without_comments(read);
        self.inactive_branch = was_inactive;
        item
    }

    fn finish_invalid_string(&mut self, start: Position) {
        // A decoding error may occur before a delimiter inside the string or may
        // have consumed the closing quote itself. Find the lexical end first.
        let mut escaped = false;
        let end = self.source.as_bytes()[start.offset + 1..]
            .iter()
            .position(|&byte| {
                let closes = byte == b'"' && !escaped;
                escaped = byte == b'\\' && !escaped;
                closes
            })
            .map_or(self.source.len(), |index| start.offset + index + 2);
        while self.pos.offset < end {
            self.bump();
        }
    }

    fn string(&mut self, start: Position) -> Result<Kind, ReadError> {
        // A plain ASCII prefix can be copied and counted in one step.
        // Escape sequences, newlines and non-ASCII retain the regular decoder.
        let remaining = &self.source[self.pos.offset..];
        let plain = remaining
            .bytes()
            .position(|byte| matches!(byte, b'"' | b'\\' | b'\r' | b'\n' | 0x80..=0xff))
            .unwrap_or(remaining.len());
        let mut text = remaining[..plain].to_owned();
        self.pos.offset += plain;
        self.pos.column += plain;
        let mut utf16 = Vec::new();
        loop {
            let ch = match self.bump_literal() {
                Some('"') if utf16.is_empty() => return Ok(Kind::String(text)),
                Some('"') => {
                    return Ok(String::from_utf16(&utf16)
                        .map(Kind::String)
                        .unwrap_or_else(|_| Kind::Utf16String(utf16)));
                }
                Some('\\') => match self.bump_literal() {
                    Some('n') => '\n',
                    Some('r') => '\r',
                    Some('t') => '\t',
                    Some('b') => '\u{0008}',
                    Some('f') => '\u{000c}',
                    Some('"') => '"',
                    Some('\\') => '\\',
                    Some('u') => {
                        let unit = self.unicode_escape(start)?;
                        let Some(ch) = char::from_u32(unit.into()) else {
                            if utf16.is_empty() {
                                utf16.extend(text.encode_utf16());
                            }
                            utf16.push(unit);
                            continue;
                        };
                        ch
                    }
                    Some(ch) if numbers::decimal_digit(ch).is_some() => {
                        self.octal_escape(ch, start)?
                    }
                    _ => return Err(self.error(start, "Invalid string escape")),
                },
                Some(ch) => ch,
                None => return Err(self.error(start, "Unterminated string")),
            };
            if utf16.is_empty() {
                text.push(ch);
            } else {
                utf16.extend_from_slice(ch.encode_utf16(&mut [0; 2]));
            }
        }
    }

    fn unicode_escape(&mut self, start: Position) -> Result<u16, ReadError> {
        let mut unit = 0;
        for _ in 0..4 {
            let ch = self
                .bump_literal()
                .ok_or_else(|| self.error(start, "Incomplete Unicode escape"))?;
            let digit = numbers::escape_digit(ch, 16)
                .ok_or_else(|| self.error(start, "Invalid Unicode escape"))?;
            unit = unit * 16 + digit as u16;
        }
        Ok(unit)
    }

    fn octal_escape(&mut self, first: char, start: Position) -> Result<char, ReadError> {
        let digit = |ch: char| numbers::escape_digit(ch, 8);
        let mut value = digit(first).ok_or_else(|| self.error(start, "Invalid octal escape"))?;
        for _ in 1..3 {
            let Some(ch) = self.peek() else {
                break;
            };
            if Self::reader_whitespace(ch)
                || ",()[]{}\";\\^#".contains(ch)
                || (!self.edn && "'@`~%".contains(ch))
            {
                break;
            }
            let next = digit(ch).ok_or_else(|| self.error(start, "Invalid octal escape"))?;
            self.bump();
            value = value * 8 + next;
        }
        if value > 0xff {
            return Err(self.error(start, "Octal escape must be in range 0 to 377"));
        }
        Ok(char::from(value as u8))
    }
}
