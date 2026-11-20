//! Detect reader-level duplicates whose equality is known from inert syntax.
use std::collections::BTreeSet;

use super::numbers::integer_key;
use super::{Form, Kind, ReadError, SymbolicNumber};

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Atom<'a> {
    Nil,
    Bool(bool),
    Character(char),
    Integer(bool, u128),
    Number(&'a str),
    SymbolicNumber(SymbolicNumber),
    String(&'a str),
    Utf16String(&'a [u16]),
    Symbol(&'a str),
    Keyword(&'a str),
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Literal<'a> {
    Atom(Atom<'a>),
    Sequence(Box<[Literal<'a>]>),
    Map(Box<[(Literal<'a>, Literal<'a>)]>),
    Set(Box<[Literal<'a>]>),
}

pub(super) fn check_set(values: &[Form], source: &str) -> Result<(), ReadError> {
    if values.len() >= 2 {
        check(values.iter(), source, "Duplicate element in set literal")?;
    }
    Ok(())
}

pub(super) fn check_map(entries: &[Form], source: &str) -> Result<(), ReadError> {
    if entries.len() >= 4 {
        let keys = entries.as_chunks::<2>().0.iter().map(|[key, _]| key);
        check(keys, source, "Duplicate key in map literal")?;
    }
    Ok(())
}

fn check<'a>(
    mut values: impl Iterator<Item = &'a Form>,
    source: &'a str,
    message: &'static str,
) -> Result<(), ReadError> {
    let mut atoms = BTreeSet::new();
    let mut first_composite = None;
    for value in values.by_ref() {
        let value = value.without_metadata();
        if let Some(atom) = atom(value, source) {
            if !atoms.insert(atom) {
                return Err(duplicate(value, message));
            }
        } else if matches!(
            value.kind,
            Kind::List(_) | Kind::Vector(_) | Kind::Map(_) | Kind::Set(_) | Kind::Quote(_)
        ) {
            if let Some(first) = first_composite {
                return check_composites(
                    [first, value].into_iter().chain(values),
                    atoms,
                    source,
                    message,
                );
            }
            first_composite = Some(value);
        }
    }
    Ok(())
}

#[cold]
fn check_composites<'a>(
    values: impl Iterator<Item = &'a Form>,
    mut atoms: BTreeSet<Atom<'a>>,
    source: &'a str,
    message: &'static str,
) -> Result<(), ReadError> {
    let mut composites = BTreeSet::new();
    for value in values {
        let value = value.without_metadata();
        let inserted = if let Some(atom) = atom(value, source) {
            atoms.insert(atom)
        } else if let Some(key) = composite(value, source) {
            composites.insert(key)
        } else {
            continue;
        };
        if !inserted {
            return Err(duplicate(value, message));
        }
    }
    Ok(())
}

fn duplicate(value: &Form, message: &'static str) -> ReadError {
    ReadError {
        span: value.span,
        message: message.into(),
    }
}

#[inline(always)]
fn atom<'a>(value: &'a Form, source: &'a str) -> Option<Atom<'a>> {
    Some(match &value.kind {
        Kind::Nil => Atom::Nil,
        Kind::Bool(value) => Atom::Bool(*value),
        Kind::Character(value) => Atom::Character(*value),
        Kind::Number => {
            let spelling = &source[value.span.start.offset..value.span.end.offset];
            integer_key(spelling).map_or(Atom::Number(spelling), |(negative, magnitude)| {
                Atom::Integer(negative, magnitude)
            })
        }
        Kind::SymbolicNumber(value) => Atom::SymbolicNumber(*value),
        Kind::String(value) => Atom::String(value),
        Kind::Utf16String(value) => Atom::Utf16String(value),
        Kind::Symbol(value) | Kind::AnonymousArgument(value) => Atom::Symbol(value),
        Kind::Keyword(value) => Atom::Keyword(value),
        _ => return None,
    })
}

fn literal<'a>(value: &'a Form, source: &'a str) -> Option<Literal<'a>> {
    let value = value.without_metadata();
    match &value.kind {
        // NaN equality depends on the enclosing collection's lookup and equality rules.
        Kind::SymbolicNumber(SymbolicNumber::NaN) => None,
        Kind::AnonymousArgument(_) => None,
        Kind::Keyword(name) if name.starts_with("::") => None,
        _ => match atom(value, source) {
            Some(atom) => Some(Literal::Atom(atom)),
            None => composite(value, source),
        },
    }
}

fn composite<'a>(value: &'a Form, source: &'a str) -> Option<Literal<'a>> {
    let children = |forms: &'a [Form]| {
        forms
            .iter()
            .map(|form| literal(form, source))
            .collect::<Option<Vec<_>>>()
    };
    Some(match &value.kind {
        Kind::List(items) | Kind::Vector(items) => Literal::Sequence(children(items)?.into()),
        Kind::Map(entries) => {
            let (pairs, trailing) = entries.as_chunks::<2>();
            if !trailing.is_empty() {
                return None;
            }
            let mut entries = pairs
                .iter()
                .map(|[key, value]| Some((literal(key, source)?, literal(value, source)?)))
                .collect::<Option<Vec<_>>>()?;
            entries.sort_unstable();
            Literal::Map(entries.into())
        }
        Kind::Set(items) => {
            let mut items = children(items)?;
            items.sort_unstable();
            Literal::Set(items.into())
        }
        Kind::Quote(item) => Literal::Sequence(Box::new([
            Literal::Atom(Atom::Symbol("quote")),
            literal(item, source)?,
        ])),
        _ => return None,
    })
}
