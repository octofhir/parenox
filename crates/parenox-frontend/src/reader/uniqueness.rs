//! Detect reader-level duplicates whose equality is known from inert syntax.
use std::collections::BTreeSet;

use super::{Form, Kind, ReadError};

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Atom<'a> {
    Nil,
    Bool(bool),
    Character(char),
    Number(&'a str),
    String(&'a str),
    Symbol(&'a str),
    Keyword(&'a str),
}

pub(super) fn check_set(values: &[Form], source: &str) -> Result<(), ReadError> {
    if values.len() < 2 {
        return Ok(());
    }
    let mut seen = BTreeSet::new();
    for value in values {
        let value = value.without_metadata();
        let atom = match &value.kind {
            Kind::Nil => Atom::Nil,
            Kind::Bool(value) => Atom::Bool(*value),
            Kind::Character(value) => Atom::Character(*value),
            Kind::Number => Atom::Number(&source[value.span.start.offset..value.span.end.offset]),
            Kind::String(value) => Atom::String(value),
            Kind::Symbol(value) | Kind::AnonymousArgument(value) => Atom::Symbol(value),
            Kind::Keyword(value) => Atom::Keyword(value),
            _ => continue,
        };
        if !seen.insert(atom) {
            return Err(ReadError {
                span: value.span,
                message: "Duplicate element in set literal".into(),
            });
        }
    }
    Ok(())
}
