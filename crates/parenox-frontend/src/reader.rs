mod edn;
mod numbers;
mod uniqueness;

pub use edn::validate_edn;

use crate::syntax::{Builder, SyntaxKind, SyntaxNode};

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
    Regex,
    Character(char),
    Number,
    SymbolicNumber(SymbolicNumber),
    Bool(bool),
    Nil,
    List(Vec<Form>),
    AnonymousFunction(AnonymousFunction),
    Vector(Vec<Form>),
    Map(Vec<Form>),
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

    pub fn unquoted(&self) -> &Form {
        match &self.without_metadata().kind {
            Kind::Quote(inner) => inner.without_metadata(),
            _ => self.without_metadata(),
        }
    }
}

#[derive(Debug)]
pub struct ReadError {
    pub span: Span,
    pub message: String,
}

/// Reader conditionals are forbidden in ordinary .clj/EDN input, preserved for
/// syntax consumers, or selected for an explicit platform feature such as :clj.
#[derive(Clone, Copy, Debug, Default)]
pub enum Conditionals<'a> {
    #[default]
    Reject,
    Preserve,
    Select(&'a str),
}

/// Fast semantic forms without capturing a CST. Rejects reader conditionals.
pub fn read(source: &str) -> Result<Vec<Form>, ReadError> {
    read_with(source, Conditionals::Reject)
}

/// The same reader with an explicit conditional policy; no project code is executed.
pub fn read_with(source: &str, conditionals: Conditionals<'_>) -> Result<Vec<Form>, ReadError> {
    Reader::new(source, false, conditionals).read_all()
}

#[doc(hidden)]
pub struct CheckRead {
    pub forms: Vec<Form>,
    pub auto_keywords: Vec<Span>,
    pub comments: Vec<LineComment>,
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
    let mut reader = Reader::new(source, false, conditionals);
    reader.auto_keywords = Some(Vec::new());
    reader.comment_capture = comment_marker.map(|marker| CommentCapture {
        marker,
        comments: Vec::new(),
    });
    let (forms, error) = reader.read_prefix();
    (
        CheckRead {
            forms,
            auto_keywords: reader.auto_keywords.unwrap(),
            comments: reader
                .comment_capture
                .map_or_else(Vec::new, |capture| capture.comments),
        },
        error,
    )
}

#[derive(Debug)]
pub struct Parse {
    green: rowan::GreenNode,
    pub forms: Vec<Form>,
    pub errors: Vec<ReadError>,
}

impl Parse {
    /// Cheap syntax view over the immutable green tree, including trivia and errors.
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

/// Retains all source bytes, including invalid and unsupported reader syntax.
/// Forms contain only successfully read top-level values; errors must be checked before analysis.
pub fn parse(source: &str) -> Parse {
    parse_with(source, Conditionals::Preserve)
}

/// Captures the whole CST and applies the requested policy to semantic forms in one pass.
pub fn parse_with(source: &str, conditionals: Conditionals<'_>) -> Parse {
    let mut reader = Reader::new(source, true, conditionals);

    let mut forms = Vec::new();
    let mut errors = Vec::new();
    loop {
        let start = reader.pos;
        match reader.next_form() {
            Ok(Some(form)) => forms.push(form),
            Ok(None) => break,
            Err(error) => {
                errors.push(error);
                // Every recovery step must consume input, even at an unsupported construct.
                if reader.pos.offset == start.offset {
                    if reader.bump().is_none() {
                        break;
                    }
                    reader.token(SyntaxKind::ErrorToken, start);
                }
            }
        }
    }
    Parse {
        green: reader.tree.unwrap().build(),
        forms,
        errors,
    }
}

struct Reader<'a> {
    source: &'a str,
    pos: Position,
    depth: usize,
    arguments: Option<AnonymousFunction>,
    auto_keywords: Option<Vec<Span>>,
    comment_capture: Option<CommentCapture>,
    tree: Option<Builder<'a>>,
    conditionals: Conditionals<'a>,
    edn: bool,
}

enum Item {
    Form(Form),
    Omit,
    Splice { forms: Vec<Form>, span: Span },
}

impl<'a> Reader<'a> {
    fn new(source: &'a str, capture: bool, conditionals: Conditionals<'a>) -> Self {
        Self {
            source,
            pos: Position {
                offset: 0,
                line: 1,
                column: 1,
            },
            depth: 0,
            arguments: None,
            auto_keywords: None,
            comment_capture: None,
            tree: capture.then(|| Builder::new(source)),
            conditionals,
            edn: false,
        }
    }

    fn read_all(&mut self) -> Result<Vec<Form>, ReadError> {
        let (forms, error) = self.read_prefix();
        error.map_or(Ok(forms), Err)
    }

    fn read_prefix(&mut self) -> (Vec<Form>, Option<ReadError>) {
        let mut forms = Vec::new();
        loop {
            match self.next_form() {
                Ok(Some(form)) => forms.push(form),
                Ok(None) => return (forms, None),
                Err(error) => return (forms, Some(error)),
            }
        }
    }

    fn next_form(&mut self) -> Result<Option<Form>, ReadError> {
        loop {
            self.skip()?;
            if self.peek().is_none() {
                return Ok(None);
            }
            match self.item()? {
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
        if let Some(tree) = &mut self.tree {
            tree.start(start.offset);
        }
    }

    fn finish_node(&mut self, kind: SyntaxKind) {
        if let Some(tree) = &mut self.tree {
            tree.finish(kind, self.pos.offset);
        }
    }

    fn token(&mut self, kind: SyntaxKind, start: Position) {
        if let Some(tree) = &mut self.tree {
            tree.token(kind, start.offset..self.pos.offset);
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.pos.offset..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos.offset += ch.len_utf8();
        if ch == '\n' {
            self.pos.line += 1;
            self.pos.column = 1;
        } else {
            self.pos.column += 1;
        }
        Some(ch)
    }

    fn error(&self, start: Position, message: impl Into<String>) -> ReadError {
        ReadError {
            span: Span {
                start,
                end: self.pos,
            },
            message: message.into(),
        }
    }

    fn nesting_error(&self, start: Position) -> ReadError {
        self.error(
            start,
            format!("Reader nesting limit ({MAX_NESTING_DEPTH}) exceeded"),
        )
    }

    // Discards remain syntax in a CST and disappear from semantic forms.
    fn skip(&mut self) -> Result<(), ReadError> {
        let first_comment = self
            .comment_capture
            .as_ref()
            .map_or(0, |capture| capture.comments.len());
        loop {
            let start = self.pos;
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    while self.peek().is_some_and(char::is_whitespace) {
                        self.bump();
                    }
                    self.token(SyntaxKind::Whitespace, start);
                }
                Some(',') => {
                    self.bump();
                    self.token(SyntaxKind::Comma, start);
                }
                Some(';') => {
                    while self.peek().is_some_and(|c| c != '\n') {
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
                Some('#') if self.source[self.pos.offset..].starts_with("#_") => {
                    self.start_node(start);
                    self.bump();
                    self.bump();
                    self.token(SyntaxKind::DiscardMark, start);
                    let result = self.without_comments(|reader| {
                        if reader.depth >= MAX_NESTING_DEPTH {
                            return Err(reader.nesting_error(start));
                        }
                        reader.depth += 1;
                        let skipped = reader.skip();
                        reader.depth -= 1;
                        skipped?;
                        if reader.peek().is_none()
                            || reader.peek().is_some_and(|c| ")]}".contains(c))
                        {
                            return Err(reader.error(start, "Expected a form after #_"));
                        }
                        reader.form()?;
                        Ok(())
                    });
                    self.finish_node(if result.is_ok() {
                        SyntaxKind::Discard
                    } else {
                        SyntaxKind::Error
                    });
                    result?;
                }
                _ => {
                    if let Some(capture) = &mut self.comment_capture {
                        for comment in &mut capture.comments[first_comment..] {
                            comment.following_offset = self.pos.offset;
                        }
                    }
                    return Ok(());
                }
            }
        }
    }

    fn without_comments<T>(&mut self, read: impl FnOnce(&mut Self) -> T) -> T {
        let capture = self.comment_capture.take();
        let result = read(self);
        self.comment_capture = capture;
        result
    }

    fn form(&mut self) -> Result<Form, ReadError> {
        loop {
            match self.item()? {
                Item::Form(form) => return Ok(form),
                Item::Omit => {}
                Item::Splice { span, .. } => {
                    return Err(self.error(
                        span.start,
                        "Reader conditional splicing requires a surrounding collection",
                    ));
                }
            }
        }
    }

    fn item(&mut self) -> Result<Item, ReadError> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(self.nesting_error(self.pos));
        }
        self.depth += 1;
        let result = self.skip().and_then(|()| {
            let start = self.pos;
            self.start_node(start);
            let result = if self.edn && self.edn_dispatch_prefix() {
                self.edn_dispatch(start)
            } else {
                self.form_inner()
            };
            if let Some(tree) = &mut self.tree {
                let spelling = &self.source[start.offset..self.pos.offset];
                let conditional = spelling.starts_with("#?");
                let kind = result.as_ref().map_or(SyntaxKind::Error, |item| {
                    if conditional {
                        SyntaxKind::Conditional
                    } else {
                        match item {
                            Item::Form(form) => match form.kind {
                                Kind::List(_) if spelling.starts_with('@') => SyntaxKind::Deref,
                                Kind::List(_) if spelling.starts_with("#'") => SyntaxKind::VarQuote,
                                Kind::List(_) => SyntaxKind::List,
                                Kind::AnonymousFunction(_) => SyntaxKind::AnonymousFunction,
                                Kind::Vector(_) => SyntaxKind::Vector,
                                Kind::Map(_) => SyntaxKind::Map,
                                Kind::Set(_) => SyntaxKind::Set,
                                Kind::Quote(_) => SyntaxKind::Quote,
                                Kind::SyntaxQuote(_) => SyntaxKind::SyntaxQuote,
                                Kind::Unquote {
                                    splicing: false, ..
                                } => SyntaxKind::Unquote,
                                Kind::Unquote { splicing: true, .. } => SyntaxKind::UnquoteSplicing,
                                Kind::Symbol(_) | Kind::AnonymousArgument(_) => SyntaxKind::Symbol,
                                Kind::Keyword(_) => SyntaxKind::Keyword,
                                Kind::String(_) | Kind::Utf16String(_) => SyntaxKind::String,
                                Kind::Regex => SyntaxKind::Regex,
                                Kind::Character(_) => SyntaxKind::Character,
                                Kind::Metadata(_, _) => SyntaxKind::Metadata,
                                Kind::Tagged(_, _) => SyntaxKind::Tagged,
                                Kind::Conditional { .. } => SyntaxKind::Conditional,
                                Kind::Number => SyntaxKind::Number,
                                Kind::SymbolicNumber(_) => SyntaxKind::SymbolicNumber,
                                Kind::Bool(_) => SyntaxKind::Boolean,
                                Kind::Nil => SyntaxKind::Nil,
                            },
                            Item::Omit | Item::Splice { .. } => SyntaxKind::Conditional,
                        }
                    }
                });
                tree.finish(kind, self.pos.offset);
            }
            result
        });
        self.depth -= 1;
        result
    }

    fn form_inner(&mut self) -> Result<Item, ReadError> {
        let start = self.pos;
        let kind = match self.bump() {
            Some('(') => {
                self.token(SyntaxKind::OpenParen, start);
                Kind::List(self.collection(')', start)?)
            }
            Some('[') => {
                self.token(SyntaxKind::OpenBracket, start);
                Kind::Vector(self.collection(']', start)?)
            }
            Some('{') => self.map(start)?,
            Some('"') => self.string(start)?,
            Some('\\') => Kind::Character(self.character(start)?),
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
            Some('#') if self.peek() == Some('#') => self.symbolic_number(start)?,
            Some('#') if self.peek() == Some('(') => self.anonymous_function(start)?,
            Some('#') if self.peek() == Some('"') => {
                self.bump();
                loop {
                    match self.bump() {
                        Some('"') => break,
                        Some('\\') if self.bump().is_some() => {}
                        Some('\\') | None => {
                            return Err(self.error(start, "Unterminated regex literal"));
                        }
                        Some(_) => {}
                    }
                }
                Kind::Regex
            }
            Some('#') if self.peek() == Some('{') => {
                return self.set(start);
            }
            Some('#') if self.peek() == Some('?') => return self.conditional(start),
            Some('#') if self.peek().is_some_and(char::is_alphabetic) => {
                self.token(SyntaxKind::DispatchMark, start);
                let tag = self.without_comments(Self::form)?;
                let Some(name) = tag.symbol() else {
                    return Err(self.error(start, "Reader tag must be a symbol"));
                };
                if name.split('/').count() > 2 || name.split('/').any(str::is_empty) {
                    return Err(self.error(start, "Invalid reader tag"));
                }
                let value = self.without_comments(Self::form)?;
                Kind::Tagged(Box::new(tag), Box::new(value))
            }
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

    fn set(&mut self, start: Position) -> Result<Item, ReadError> {
        self.bump(); // {
        self.token(SyntaxKind::OpenSet, start);
        let values = self.collection('}', start)?;
        uniqueness::check_set(&values, self.source)?;
        Ok(self.form_item(start, Kind::Set(values)))
    }

    fn map(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.token(SyntaxKind::OpenBrace, start);
        let entries = self.collection('}', start)?;
        let unresolved = matches!(self.conditionals, Conditionals::Preserve)
            && entries
                .iter()
                .any(|form| matches!(form.kind, Kind::Conditional { .. }));
        if !unresolved {
            if !entries.len().is_multiple_of(2) {
                return Err(self.error(start, "Map literal requires an even number of forms"));
            }
            uniqueness::check_map(&entries, self.source)?;
        }
        Ok(Kind::Map(entries))
    }

    fn symbolic_number(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.bump(); // #
        self.token(SyntaxKind::DispatchMark, start);
        let value = self.without_comments(Self::form)?;
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
        let kind = match token {
            arg if self.arguments.is_some() && arg.starts_with('%') => {
                self.anonymous_argument(arg, start)?
            }
            "nil" => Kind::Nil,
            "true" => Kind::Bool(true),
            "false" => Kind::Bool(false),
            s if s.starts_with(':') => {
                if self.edn && s.starts_with("::") {
                    return Err(self.error(start, "Auto-resolved keywords are unavailable in EDN"));
                }
                if s.starts_with("::")
                    && let Some(spans) = &mut self.auto_keywords
                {
                    spans.push(Span {
                        start,
                        end: self.pos,
                    });
                }
                Kind::Keyword(s.to_string())
            }
            s if numeric => {
                if !numbers::valid_number(s) {
                    return Err(self.error(start, "Invalid numeric literal"));
                }
                Kind::Number
            }
            s => Kind::Symbol(s.to_string()),
        };
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
        if matches!(
            form.without_metadata().kind,
            Kind::Unquote { splicing: true, .. }
        ) {
            return Err(self.error(
                form.span.start,
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
        let function = self.arguments.as_mut().unwrap();
        if token == "%&" {
            function.variadic = true;
            return Ok(Kind::AnonymousArgument(token.into()));
        }
        let index = if token == "%" {
            Some(1)
        } else {
            let digits = &token[1..];
            (!digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit()))
                .then(|| digits.parse::<usize>().ok())
                .flatten()
        };
        let Some(index) = index else {
            return Err(self.error(start, "Argument literal must be %, %& or %integer"));
        };
        function.required = function.required.max(index);
        // % and %1 are two spellings of one binding inside this reader scope.
        Ok(Kind::AnonymousArgument(
            if token == "%" { "%1" } else { token }.into(),
        ))
    }

    fn token_boundary(c: char) -> bool {
        // Quote, # and % terminate only at the start of a form in Clojure.
        c.is_whitespace() || "()[]{}\";,`~@^\\".contains(c)
    }

    fn metadata_form(&mut self, start: Position) -> Result<Kind, ReadError> {
        let mut metadata = self.without_comments(Self::form)?;
        let shorthand = match &metadata.without_metadata().kind {
            Kind::Map(_) => None,
            Kind::Conditional { .. } if matches!(self.conditionals, Conditionals::Preserve) => None,
            Kind::Keyword(key) => Some((key.clone(), true)),
            Kind::Symbol(_)
            | Kind::AnonymousArgument(_)
            | Kind::String(_)
            | Kind::Utf16String(_) => Some((":tag".into(), false)),
            Kind::Vector(_) if !self.edn => Some((":param-tags".into(), false)),
            _ => {
                return Err(self.error(
                    start,
                    "Metadata requires a map, keyword, symbol, string or vector",
                ));
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
        if !matches!(
            target.without_metadata().kind,
            Kind::Symbol(_)
                | Kind::AnonymousArgument(_)
                | Kind::AnonymousFunction(_)
                | Kind::List(_)
                | Kind::Vector(_)
                | Kind::Map(_)
                | Kind::Set(_)
                | Kind::Quote(_)
                | Kind::SyntaxQuote(_)
                | Kind::Unquote { .. }
                | Kind::Tagged(_, _)
                | Kind::Conditional { .. }
        ) {
            return Err(self.error(
                start,
                "Metadata requires a symbol, collection or metadata-capable tagged value",
            ));
        }
        Ok(Kind::Metadata(Box::new(metadata), Box::new(target)))
    }

    fn character(&mut self, start: Position) -> Result<char, ReadError> {
        let token_start = self.pos.offset;
        self.bump()
            .ok_or_else(|| self.error(start, "Expected a character after backslash"))?;
        while self.peek().is_some_and(|c| !Self::token_boundary(c)) {
            self.bump();
        }
        let token = &self.source[token_start..self.pos.offset];
        let mut chars = token.chars();
        let first = chars.next().unwrap();
        let value = if chars.next().is_none() {
            (u32::from(first) <= 0xffff).then_some(first)
        } else {
            match token {
                "newline" => Some('\n'),
                "space" => Some(' '),
                "tab" => Some('\t'),
                "return" => Some('\r'),
                "backspace" => Some('\u{0008}'),
                "formfeed" => Some('\u{000c}'),
                _ => {
                    let decoded = if let Some(digits) = token
                        .strip_prefix('u')
                        .filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                    {
                        u32::from_str_radix(digits, 16).ok()
                    } else if let Some(digits) = token.strip_prefix('o').filter(|s| {
                        (1..=3).contains(&s.len()) && s.bytes().all(|b| (b'0'..=b'7').contains(&b))
                    }) {
                        u32::from_str_radix(digits, 8).ok().filter(|&n| n <= 0xff)
                    } else {
                        None
                    };
                    decoded.and_then(char::from_u32)
                }
            }
        };
        value.ok_or_else(|| self.error(start, "Invalid JVM character literal"))
    }

    fn collection(&mut self, end: char, start: Position) -> Result<Vec<Form>, ReadError> {
        let mut forms = Vec::new();
        while let Some(item) = self.collection_item(end, start)? {
            match item {
                Item::Form(form) => forms.push(form),
                Item::Omit => {}
                Item::Splice { forms: values, .. } => forms.extend(values),
            }
        }
        Ok(forms)
    }

    fn collection_item(&mut self, end: char, start: Position) -> Result<Option<Item>, ReadError> {
        self.skip()?;
        match self.peek() {
            Some(c) if c == end => {
                let close = self.pos;
                self.bump();
                self.token(
                    match end {
                        ')' => SyntaxKind::CloseParen,
                        ']' => SyntaxKind::CloseBracket,
                        '}' => SyntaxKind::CloseBrace,
                        _ => unreachable!(),
                    },
                    close,
                );
                Ok(None)
            }
            None => Err(self.error(start, format!("Unclosed form; expected {end}"))),
            _ => self.item().map(Some),
        }
    }

    fn conditional(&mut self, start: Position) -> Result<Item, ReadError> {
        self.bump(); // ?
        let splicing = self.peek() == Some('@');
        if splicing {
            self.bump();
        }
        self.token(SyntaxKind::ConditionalMark, start);
        self.skip()?;
        if self.peek() != Some('(') {
            return Err(self.error(start, "Reader conditional requires a list"));
        }
        let feature = match self.conditionals {
            Conditionals::Reject => {
                return Err(self.error(
                    start,
                    "Reader conditionals require .cljc input or an explicit reader policy",
                ));
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
            Conditionals::Select(feature) => feature,
        };
        let open = self.pos;
        self.start_node(open);
        self.bump(); // (
        self.token(SyntaxKind::OpenParen, open);
        let result = self.select_conditional(start, feature, splicing);
        self.finish_node(if result.is_ok() {
            SyntaxKind::List
        } else {
            SyntaxKind::Error
        });
        result
    }

    fn select_conditional(
        &mut self,
        start: Position,
        feature: &str,
        splicing: bool,
    ) -> Result<Item, ReadError> {
        loop {
            let Some(key) = self.without_comments(|reader| reader.conditional_form(start))? else {
                return Ok(Item::Omit);
            };
            let Some(key_name) = key.keyword() else {
                return Err(self.error(
                    key.span.start,
                    "Reader conditional feature must be a keyword",
                ));
            };
            if matches!(key_name, ":else" | ":none") {
                return Err(self.error(key.span.start, "Reader conditional feature is reserved"));
            }
            let selected = key_name == feature || key_name == ":default";
            let value = if selected {
                self.conditional_form(start)?
            } else {
                self.without_comments(|reader| reader.conditional_form(start))?
            };
            let Some(mut value) = value else {
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
                // Still read the tail for syntax errors. Nested conditionals
                // keep their selection semantics even in inactive branches.
                self.without_comments(|reader| {
                    while reader.collection_item(')', start)?.is_some() {}
                    Ok(())
                })?;
                if !splicing {
                    return Ok(Item::Form(value));
                }
                loop {
                    match value.kind {
                        Kind::Metadata(_, target) => value = *target,
                        Kind::List(values) | Kind::Vector(values) => {
                            return Ok(Item::Splice {
                                forms: values,
                                span: Span {
                                    start,
                                    end: self.pos,
                                },
                            });
                        }
                        _ => {
                            return Err(self.error(
                                value.span.start,
                                "Splicing reader conditional requires a list or vector",
                            ));
                        }
                    }
                }
            }
        }
    }

    fn conditional_form(&mut self, start: Position) -> Result<Option<Form>, ReadError> {
        loop {
            match self.collection_item(')', start)? {
                Some(Item::Form(form)) => return Ok(Some(form)),
                Some(Item::Omit) => {}
                Some(Item::Splice { span, .. }) => {
                    return Err(self.error(
                        span.start,
                        "Splicing is not supported in reader conditional feature/value pairs",
                    ));
                }
                None => return Ok(None),
            }
        }
    }

    fn string(&mut self, start: Position) -> Result<Kind, ReadError> {
        let mut text = String::new();
        let mut utf16 = Vec::new();
        loop {
            let ch = match self.bump() {
                Some('"') if utf16.is_empty() => return Ok(Kind::String(text)),
                Some('"') => {
                    return Ok(String::from_utf16(&utf16)
                        .map(Kind::String)
                        .unwrap_or_else(|_| Kind::Utf16String(utf16)));
                }
                Some('\\') => match self.bump() {
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
                .bump()
                .ok_or_else(|| self.error(start, "Incomplete Unicode escape"))?;
            let digit = ch
                .to_digit(16)
                .ok_or_else(|| self.error(start, "Invalid Unicode escape"))?;
            unit = unit * 16 + digit as u16;
        }
        Ok(unit)
    }
}
