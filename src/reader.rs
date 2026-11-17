use serde::Serialize;

use crate::syntax::{Builder, SyntaxKind, SyntaxNode};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Position {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
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
    Keyword(String),
    String(String),
    Number,
    Bool(bool),
    Nil,
    List(Vec<Form>),
    Vector(Vec<Form>),
    Map(Vec<Form>),
    Set(Vec<Form>),
    Quote(Box<Form>),
}

impl Form {
    pub fn symbol(&self) -> Option<&str> {
        match &self.kind {
            Kind::Symbol(s) => Some(s),
            _ => None,
        }
    }

    pub fn keyword(&self) -> Option<&str> {
        match &self.kind {
            Kind::Keyword(s) => Some(s),
            _ => None,
        }
    }

    pub fn vector(&self) -> Option<&[Form]> {
        match &self.kind {
            Kind::Vector(xs) => Some(xs),
            _ => None,
        }
    }

    pub fn unquoted(&self) -> &Form {
        match &self.kind {
            Kind::Quote(inner) => inner,
            _ => self,
        }
    }
}

#[derive(Debug)]
pub struct ReadError {
    pub span: Span,
    pub message: String,
}

/// Fast semantic forms for checking. Uses the same reader without capturing a CST.
pub fn read(source: &str) -> Result<Vec<Form>, ReadError> {
    let mut reader = Reader::new(source, false);
    let mut forms = Vec::new();
    while let Some(form) = reader.next_form()? {
        forms.push(form);
    }
    Ok(forms)
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
    let mut reader = Reader::new(source, true);
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
    tree: Option<Builder<'a>>,
}

impl<'a> Reader<'a> {
    fn new(source: &'a str, capture: bool) -> Self {
        Self {
            source,
            pos: Position {
                offset: 0,
                line: 1,
                column: 1,
            },
            depth: 0,
            tree: capture.then(|| Builder::new(source)),
        }
    }

    fn next_form(&mut self) -> Result<Option<Form>, ReadError> {
        self.skip()?;
        if self.peek().is_none() {
            Ok(None)
        } else {
            self.form().map(Some)
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

    // Discards remain syntax in a CST and disappear from semantic forms.
    fn skip(&mut self) -> Result<(), ReadError> {
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
                }
                Some('#') if self.source[self.pos.offset..].starts_with("#_") => {
                    self.start_node(start);
                    self.bump();
                    self.bump();
                    self.token(SyntaxKind::DiscardMark, start);
                    let result = (|| {
                        if self.depth >= 128 {
                            return Err(self.error(start, "Reader nesting limit (128) exceeded"));
                        }
                        self.depth += 1;
                        let skipped = self.skip();
                        self.depth -= 1;
                        skipped?;
                        if self.peek().is_none() || self.peek().is_some_and(|c| ")]}".contains(c)) {
                            return Err(self.error(start, "Expected a form after #_"));
                        }
                        self.form()?;
                        Ok(())
                    })();
                    self.finish_node(if result.is_ok() {
                        SyntaxKind::Discard
                    } else {
                        SyntaxKind::Error
                    });
                    result?;
                }
                _ => return Ok(()),
            }
        }
    }

    fn form(&mut self) -> Result<Form, ReadError> {
        if self.depth >= 128 {
            return Err(self.error(self.pos, "Reader nesting limit (128) exceeded"));
        }
        self.depth += 1;
        let result = self.skip().and_then(|()| {
            self.start_node(self.pos);
            let result = self.form_inner();
            if let Some(tree) = &mut self.tree {
                let kind = result
                    .as_ref()
                    .map_or(SyntaxKind::Error, |form| match form.kind {
                        Kind::List(_) => SyntaxKind::List,
                        Kind::Vector(_) => SyntaxKind::Vector,
                        Kind::Map(_) => SyntaxKind::Map,
                        Kind::Set(_) => SyntaxKind::Set,
                        Kind::Quote(_) => SyntaxKind::Quote,
                        Kind::Symbol(_) => SyntaxKind::Symbol,
                        Kind::Keyword(_) => SyntaxKind::Keyword,
                        Kind::String(_) => SyntaxKind::String,
                        Kind::Number => SyntaxKind::Number,
                        Kind::Bool(_) => SyntaxKind::Boolean,
                        Kind::Nil => SyntaxKind::Nil,
                    });
                tree.finish(kind, self.pos.offset);
            }
            result
        });
        self.depth -= 1;
        result
    }

    fn form_inner(&mut self) -> Result<Form, ReadError> {
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
            Some('{') => {
                self.token(SyntaxKind::OpenBrace, start);
                let xs = self.collection('}', start)?;
                if xs.len() % 2 != 0 {
                    return Err(self.error(start, "Map literal requires an even number of forms"));
                }
                Kind::Map(xs)
            }
            Some('"') => Kind::String(self.string(start)?),
            Some('\'') => {
                self.token(SyntaxKind::QuoteMark, start);
                Kind::Quote(Box::new(self.form()?))
            }
            Some('#') if self.peek() == Some('{') => {
                self.bump();
                self.token(SyntaxKind::OpenSet, start);
                Kind::Set(self.collection('}', start)?)
            }
            Some(c @ (')' | ']' | '}')) => {
                return Err(self.error(start, format!("Unexpected closing delimiter {c}")));
            }
            Some(c @ ('#' | '`' | '~' | '@' | '^' | '\\')) => {
                return Err(self.error(start, format!("Reader syntax {c} is not supported yet")));
            }
            Some(_) => {
                while self
                    .peek()
                    .is_some_and(|c| !c.is_whitespace() && !"()[]{}\";,\'`~@^\\".contains(c))
                {
                    self.bump();
                }
                let token = &self.source[start.offset..self.pos.offset];
                match token {
                    "nil" => Kind::Nil,
                    "true" => Kind::Bool(true),
                    "false" => Kind::Bool(false),
                    s if s.starts_with(':') => Kind::Keyword(s.to_string()),
                    s if s.chars().next().is_some_and(|c| c.is_ascii_digit())
                        || s.strip_prefix(['+', '-']).is_some_and(|s| {
                            s.chars().next().is_some_and(|c| c.is_ascii_digit())
                        }) =>
                    {
                        if s.parse::<f64>().is_ok() {
                            Kind::Number
                        } else {
                            return Err(self.error(start, "Unsupported numeric literal"));
                        }
                    }
                    s => Kind::Symbol(s.to_string()),
                }
            }
            None => return Err(self.error(start, "Unexpected end of input")),
        };
        Ok(Form {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        })
    }

    fn collection(&mut self, end: char, start: Position) -> Result<Vec<Form>, ReadError> {
        let mut xs = Vec::new();
        loop {
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
                    return Ok(xs);
                }
                None => return Err(self.error(start, format!("Unclosed form; expected {end}"))),
                _ => xs.push(self.form()?),
            }
        }
    }

    fn string(&mut self, start: Position) -> Result<String, ReadError> {
        let mut value = String::new();
        loop {
            match self.bump() {
                Some('"') => return Ok(value),
                Some('\\') => {
                    match self.bump() {
                        Some('n') => value.push('\n'),
                        Some('r') => value.push('\r'),
                        Some('t') => value.push('\t'),
                        Some('b') => value.push('\u{0008}'),
                        Some('f') => value.push('\u{000c}'),
                        Some('"') => value.push('"'),
                        Some('\\') => value.push('\\'),
                        Some('u') => {
                            let mut digits = String::new();
                            for _ in 0..4 {
                                digits.push(self.bump().ok_or_else(|| {
                                    self.error(start, "Incomplete Unicode escape")
                                })?);
                            }
                            let ch = u32::from_str_radix(&digits, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| self.error(start, "Invalid Unicode escape"))?;
                            value.push(ch);
                        }
                        _ => return Err(self.error(start, "Invalid string escape")),
                    }
                }
                Some(c) => value.push(c),
                None => return Err(self.error(start, "Unterminated string")),
            }
        }
    }
}
