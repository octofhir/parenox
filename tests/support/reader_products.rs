//! Exercise the syntax and semantic entry points together for reader fixtures.
use std::ops::Deref;

pub use parenox::reader::*;

#[derive(Debug)]
pub struct ParsedForms {
    pub forms: Vec<Form>,
    syntax: Parse,
}

impl Deref for ParsedForms {
    type Target = Parse;

    fn deref(&self) -> &Parse {
        &self.syntax
    }
}

pub fn parse(source: &str) -> ParsedForms {
    parse_with(source, Conditionals::Preserve)
}

pub fn parse_with(source: &str, conditionals: Conditionals<'_>) -> ParsedForms {
    let syntax = parenox::reader::parse_with(source, conditionals);
    let (forms, error) = read_recovering(source, conditionals);
    assert_eq!(syntax.errors.first(), error.as_ref(), "{source}");
    ParsedForms { forms, syntax }
}

pub fn parse_edn(source: &str) -> ParsedForms {
    let syntax = parenox::reader::parse_edn(source);
    let (forms, error) = read_edn_recovering(source);
    assert_eq!(syntax.errors.first(), error.as_ref(), "{source}");
    ParsedForms { forms, syntax }
}
