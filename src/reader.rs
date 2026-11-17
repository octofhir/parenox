use serde::Serialize;

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

pub fn read(source: &str) -> Result<Vec<Form>, ReadError> {
    let mut reader = Reader {
        source,
        depth: 0,
        pos: Position {
            offset: 0,
            line: 1,
            column: 1,
        },
    };
    let mut forms = Vec::new();
    loop {
        reader.skip()?;
        if reader.peek().is_none() {
            return Ok(forms);
        }
        forms.push(reader.form()?);
    }
}

struct Reader<'a> {
    source: &'a str,
    pos: Position,
    depth: usize,
}

impl Reader<'_> {
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

    // Discards are consumed as reader syntax, including nested #_ #_ forms.
    fn skip(&mut self) -> Result<(), ReadError> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() || c == ',' => {
                    self.bump();
                }
                Some(';') => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                Some('#') if self.source[self.pos.offset..].starts_with("#_") => {
                    let start = self.pos;
                    self.bump();
                    self.bump();
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
        let result = self.form_inner();
        self.depth -= 1;
        result
    }

    fn form_inner(&mut self) -> Result<Form, ReadError> {
        self.skip()?;
        let start = self.pos;
        let kind = match self.bump() {
            Some('(') => Kind::List(self.collection(')', start)?),
            Some('[') => Kind::Vector(self.collection(']', start)?),
            Some('{') => {
                let xs = self.collection('}', start)?;
                if xs.len() % 2 != 0 {
                    return Err(self.error(start, "Map literal requires an even number of forms"));
                }
                Kind::Map(xs)
            }
            Some('"') => Kind::String(self.string(start)?),
            Some('\'') => Kind::Quote(Box::new(self.form()?)),
            Some('#') if self.peek() == Some('{') => {
                self.bump();
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
                    self.bump();
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
