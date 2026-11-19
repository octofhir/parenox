//! Layout-only formatting over the shared lossless reader.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

use parenox_frontend::reader::{self, ReadError};
use parenox_frontend::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentRule {
    Block,
    Align,
}

#[derive(Clone, Debug)]
pub struct FormatOptions {
    pub indent_width: u8,
    pub insert_final_newline: bool,
    pub indent_rules: BTreeMap<String, IndentRule>,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            indent_width: 2,
            insert_final_newline: true,
            indent_rules: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Clojure,
    Edn,
}

impl Language {
    pub fn for_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "clj" | "cljc" | "cljs" => Some(Self::Clojure),
            "edn" => Some(Self::Edn),
            _ => None,
        }
    }
}

/// Format valid source without evaluating, reordering or rewriting forms.
/// Reader errors leave the input untouched; physical line breaks are preserved.
pub fn format(
    source: &str,
    language: Language,
    options: &FormatOptions,
) -> Result<String, ReadError> {
    format_layout(source, language, options, None)
}

/// Format only the physical lines in a byte range. The range must start and
/// end at line boundaries or EOF. Invalid and empty ranges leave the text
/// unchanged. Preceding lines retain their original layout as context.
pub fn format_lines(
    source: &str,
    language: Language,
    options: &FormatOptions,
    range: Range<usize>,
) -> Result<String, ReadError> {
    if source.get(range.clone()).is_none()
        || range.is_empty()
        || !is_line_boundary(source, range.start)
        || !is_line_boundary(source, range.end)
    {
        return Ok(source.to_owned());
    }
    format_layout(source, language, options, Some(range))
}

fn is_line_boundary(source: &str, at: usize) -> bool {
    at == 0
        || at == source.len()
        || match source.as_bytes()[at - 1] {
            b'\n' => true,
            b'\r' => source.as_bytes()[at] != b'\n',
            _ => false,
        }
}

fn format_layout(
    source: &str,
    language: Language,
    options: &FormatOptions,
    selection: Option<Range<usize>>,
) -> Result<String, ReadError> {
    let mut parsed = match language {
        Language::Clojure => reader::parse(source),
        Language::Edn => reader::parse_edn(source),
    };
    if !parsed.errors.is_empty() {
        return Err(parsed.errors.remove(0));
    }
    let mut layout = Layout {
        source,
        options,
        out: String::with_capacity(source.len()),
        line: 0,
        column: 0,
        selection,
    };
    layout.node(&parsed.syntax(), language == Language::Edn);
    if layout.selection.is_none()
        && options.insert_final_newline
        && !layout.out.is_empty()
        && !layout.out.ends_with(['\r', '\n'])
    {
        layout.out.push_str(newline(source));
    }
    Ok(layout.out)
}

fn newline(source: &str) -> &str {
    match source.find(['\r', '\n']) {
        Some(offset) if source[offset..].starts_with("\r\n") => "\r\n",
        Some(offset) if source.as_bytes()[offset] == b'\r' => "\r",
        _ => "\n",
    }
}

struct Layout<'a> {
    source: &'a str,
    options: &'a FormatOptions,
    out: String,
    line: usize,
    column: usize,
    selection: Option<Range<usize>>,
}

impl<'a> Layout<'a> {
    fn node(&mut self, node: &SyntaxNode, data: bool) {
        match node.kind() {
            SyntaxKind::Root => self.sequence(node, false, data),
            SyntaxKind::List | SyntaxKind::AnonymousFunction => self.sequence(node, true, data),
            SyntaxKind::Vector | SyntaxKind::Map | SyntaxKind::Set => {
                self.sequence(node, false, data)
            }
            SyntaxKind::Discard => {
                let range = node.text_range();
                self.push(&self.source[usize::from(range.start())..usize::from(range.end())]);
            }
            kind => {
                // A prefix can require adjacency to its operand. Keep its own
                // tokens intact and only format collections inside the node.
                let data = data || matches!(kind, SyntaxKind::Quote | SyntaxKind::SyntaxQuote);
                for element in node.children_with_tokens() {
                    self.element(element, data);
                }
            }
        }
    }

    fn element(&mut self, element: SyntaxElement, data: bool) {
        match element {
            SyntaxElement::Node(node) => self.node(&node, data),
            SyntaxElement::Token(token) => self.push(token.text()),
        }
    }

    fn sequence(&mut self, node: &SyntaxNode, call: bool, data: bool) {
        use SyntaxKind::*;

        let opening_column = self.column;
        let mut indent = 0;
        let mut previous = None;
        let mut whitespace = None;
        let mut forms = 0;
        let mut align = None;
        for element in node.children_with_tokens() {
            let kind = element.kind();
            if kind == Whitespace {
                whitespace = Some(self.text(&element));
                continue;
            }
            let closing = matches!(kind, CloseParen | CloseBrace | CloseBracket);
            self.gap_at(
                whitespace.take().unwrap_or_default(),
                previous,
                Some(kind),
                if closing { opening_column } else { indent },
                usize::from(element.text_range().start()),
            );
            if matches!(
                kind,
                OpenParen | OpenAnonymousFunction | OpenBrace | OpenBracket | OpenSet
            ) {
                self.element(element, data);
                indent = self.column;
            } else {
                if !matches!(kind, Comment | Comma | Discard) && !closing {
                    if call && !data && forms == 0 && kind == Symbol {
                        indent = opening_column + usize::from(self.options.indent_width.max(1));
                        if self.rule(self.text(&element)) == IndentRule::Align {
                            align = Some(self.line);
                        }
                    } else if forms == 1 && align == Some(self.line) {
                        indent = self.column;
                    }
                    forms += 1;
                }
                self.element(element, data);
            }
            previous = Some(kind);
        }
        self.gap_at(
            whitespace.unwrap_or_default(),
            previous,
            None,
            0,
            usize::from(node.text_range().end()),
        );
    }

    fn text(&self, element: &SyntaxElement) -> &'a str {
        let range = element.text_range();
        &self.source[usize::from(range.start())..usize::from(range.end())]
    }

    fn gap_at(
        &mut self,
        whitespace: &str,
        previous: Option<SyntaxKind>,
        next: Option<SyntaxKind>,
        indent: usize,
        at: usize,
    ) {
        let Some(selection) = &self.selection else {
            self.gap(whitespace, previous, next, indent);
            return;
        };
        let start = at - whitespace.len();
        let selected_start = selection.start.clamp(start, at) - start;
        let selected_end = selection.end.clamp(start, at) - start;
        let next = next.filter(|_| selection.contains(&at));
        if selected_start == selected_end && next.is_none() {
            self.push(whitespace);
            return;
        }

        let prefix = &whitespace[..selected_start];
        let middle = &whitespace[selected_start..selected_end];
        let suffix = &whitespace[selected_end..];
        self.push(prefix);
        // Preserve the separator when a lone CR outside the selection is
        // followed by a separate LF inside it.
        if prefix.ends_with('\r')
            && middle
                .trim_start_matches(|ch: char| !matches!(ch, '\r' | '\n') && ch.is_whitespace())
                .starts_with('\n')
        {
            self.spaces(1);
        }
        if prefix.ends_with(['\r', '\n']) && !middle.contains(['\r', '\n']) {
            if next.is_some() {
                self.spaces(indent);
            }
        } else {
            self.gap(middle, previous, next, indent);
        }
        self.push(suffix);
    }

    fn gap(
        &mut self,
        whitespace: &str,
        previous: Option<SyntaxKind>,
        right: Option<SyntaxKind>,
        indent: usize,
    ) {
        use SyntaxKind::*;
        if whitespace.contains(['\r', '\n']) {
            let mut previous_cr = None;
            for (offset, byte) in whitespace
                .bytes()
                .enumerate()
                .filter(|&(_, byte)| matches!(byte, b'\r' | b'\n'))
            {
                // A lone CR and a lone LF separated by horizontal whitespace
                // are two line breaks. Don't merge them into a single CRLF.
                if byte == b'\n' && previous_cr.is_some_and(|cr| cr + 1 != offset) {
                    self.out.push(' ');
                }
                self.out.push(char::from(byte));
                previous_cr = (byte == b'\r').then_some(offset);
            }
            self.line += 1;
            self.column = 0;
            if right.is_some() {
                self.spaces(indent);
            }
        } else if self.column != 0
            && previous.is_some()
            && !matches!(
                previous,
                Some(OpenParen | OpenAnonymousFunction | OpenBrace | OpenBracket | OpenSet)
            )
            && !matches!(
                right,
                None | Some(CloseParen | CloseBrace | CloseBracket | Comma)
            )
        {
            self.spaces(1);
        }
    }

    fn spaces(&mut self, count: usize) {
        self.out.extend(std::iter::repeat_n(' ', count));
        self.column += count;
    }

    fn push(&mut self, text: &str) {
        self.out.push_str(text);
        // Only the last line's column matters. Track whether an alignment
        // anchor is still on that line without scanning content line by line.
        let tail = if let Some(offset) = text.rfind(['\r', '\n']) {
            self.line += 1;
            self.column = 0;
            &text[offset + 1..]
        } else {
            text
        };
        let tab_width = usize::from(self.options.indent_width.max(1));
        for ch in tail.chars() {
            self.column += if ch == '\t' {
                tab_width - self.column % tab_width
            } else {
                1
            };
        }
    }

    fn rule(&self, raw: &str) -> IndentRule {
        if let Some(&rule) = self.options.indent_rules.get(raw) {
            return rule;
        }
        let core = raw.strip_prefix("clojure.core/").unwrap_or(raw);
        let test = raw.strip_prefix("clojure.test/").unwrap_or(raw);
        if matches!(
            core,
            "ns" | "def"
                | "defn"
                | "defn-"
                | "defmacro"
                | "defmulti"
                | "defmethod"
                | "defprotocol"
                | "defrecord"
                | "deftype"
                | "defonce"
                | "fn"
                | "fn*"
                | "let"
                | "let*"
                | "letfn"
                | "loop"
                | "loop*"
                | "binding"
                | "with-open"
                | "with-redefs"
                | "if"
                | "if-not"
                | "if-let"
                | "if-some"
                | "when"
                | "when-not"
                | "when-let"
                | "when-some"
                | "when-first"
                | "cond"
                | "case"
                | "try"
                | "catch"
                | "finally"
                | "do"
                | "for"
                | "doseq"
                | "dotimes"
                | "comment"
                | "->"
                | "->>"
                | "some->"
                | "some->>"
                | "cond->"
                | "cond->>"
                | "as->"
        ) || matches!(test, "deftest" | "testing" | "are" | "use-fixtures")
        {
            IndentRule::Block
        } else {
            IndentRule::Align
        }
    }
}
