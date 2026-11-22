//! Detect reader-level duplicates whose equality is known from inert syntax.
use std::borrow::Cow;
use std::collections::BTreeSet;

use super::namespace_map::{qualify_keyword, qualify_symbol};
use super::numbers::{NumberKey, double_key, number_key};
use super::{Form, ImmediateSyntaxQuote, Kind, Position, ReadError, Reader, SymbolicNumber};

type ResolveKeyword<'a> = dyn Fn(&str, Position) -> Option<String> + 'a;

#[derive(Clone, Copy)]
struct Context<'a, 'r> {
    source: &'a str,
    resolve_keyword: Option<&'r ResolveKeyword<'r>>,
    compare_raw_auto_keywords: bool,
}

#[derive(Clone, Copy)]
enum KeyNames<'a> {
    Plain,
    ProjectIn(&'a str),
    AlreadyQualified,
}

impl KeyNames<'_> {
    fn resolved_map(form: &Form, source: &str) -> Self {
        if source[form.span.start.offset..].starts_with("#:") {
            Self::AlreadyQualified
        } else {
            Self::Plain
        }
    }
}

const DUPLICATE_KEY: &str = "Duplicate key in map literal";
const DUPLICATE_ELEMENT: &str = "Duplicate element in set literal";

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Atom<'a> {
    Nil,
    Bool(bool),
    Character(char),
    Number(NumberKey<'a>),
    NaN,
    String(&'a str),
    Utf16String(&'a [u16]),
    Symbol(Cow<'a, str>),
    Keyword(Cow<'a, str>),
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Literal<'a> {
    Atom(Atom<'a>),
    Sequence(Box<[Literal<'a>]>),
    Map(Box<[(Literal<'a>, Literal<'a>)]>),
    Set(Box<[Literal<'a>]>),
}

pub(super) fn check_set(
    values: &[Form],
    source: &str,
    compare_raw_auto_keywords: bool,
) -> Result<(), ReadError> {
    if values.len() >= 2 {
        check(
            values.iter(),
            Context {
                source,
                resolve_keyword: None,
                compare_raw_auto_keywords,
            },
            KeyNames::Plain,
            DUPLICATE_ELEMENT,
        )?;
    }
    Ok(())
}

pub(super) fn check_map(
    entries: &[Form],
    source: &str,
    compare_raw_auto_keywords: bool,
) -> Result<(), ReadError> {
    if entries.len() >= 4 {
        check(
            map_keys(entries),
            Context {
                source,
                resolve_keyword: None,
                compare_raw_auto_keywords,
            },
            KeyNames::Plain,
            DUPLICATE_KEY,
        )?;
    }
    Ok(())
}

pub(super) fn check_qualified_map(
    entries: &[Form],
    source: &str,
    namespace: &str,
) -> Result<(), ReadError> {
    if entries.len() >= 4 {
        check(
            map_keys(entries),
            Context {
                source,
                resolve_keyword: None,
                compare_raw_auto_keywords: false,
            },
            KeyNames::ProjectIn(namespace),
            DUPLICATE_KEY,
        )?;
    }
    Ok(())
}

/// Recheck a literal after a source consumer has established its namespace context.
/// The supplied resolver is called only for auto-resolved keywords.
pub fn check_resolved_collection(
    form: &Form,
    source: &str,
    resolve_keyword: &ResolveKeyword<'_>,
) -> Result<(), ReadError> {
    let context = Context {
        source,
        resolve_keyword: Some(resolve_keyword),
        compare_raw_auto_keywords: false,
    };
    match &form.kind {
        Kind::Map(entries)
            if entries.len() >= 4
                && map_keys(entries).any(|key| may_need_resolution(key, source)) =>
        {
            check(
                map_keys(entries),
                context,
                KeyNames::resolved_map(form, source),
                DUPLICATE_KEY,
            )
        }
        Kind::Set(values)
            if values.len() >= 2 && values.iter().any(|key| may_need_resolution(key, source)) =>
        {
            check(values.iter(), context, KeyNames::Plain, DUPLICATE_ELEMENT)
        }
        _ => Ok(()),
    }
}

fn map_keys(entries: &[Form]) -> impl Iterator<Item = &Form> {
    entries.as_chunks::<2>().0.iter().map(|[key, _]| key)
}

fn may_need_resolution(value: &Form, source: &str) -> bool {
    let value = value.without_metadata();
    match &value.kind {
        Kind::Keyword(_) => {
            source[value.span.start.offset..value.span.end.offset].starts_with("::")
        }
        Kind::List(_)
        | Kind::Vector(_)
        | Kind::Map(_)
        | Kind::Set(_)
        | Kind::Quote(_)
        | Kind::SyntaxQuote(_)
        | Kind::Unquote { .. } => {
            source[value.span.start.offset..value.span.end.offset].contains("::")
        }
        _ => false,
    }
}

fn check<'a>(
    mut values: impl Iterator<Item = &'a Form>,
    context: Context<'a, '_>,
    names: KeyNames<'_>,
    message: &'static str,
) -> Result<(), ReadError> {
    let mut atoms = BTreeSet::new();
    let mut first_composite = None;
    for value in values.by_ref() {
        let value = value.without_metadata();
        if let Some(atom) = key_atom(value, context, names) {
            if !atoms.insert(atom) {
                return Err(duplicate(value, message));
            }
        } else if matches!(
            value.kind,
            Kind::List(_)
                | Kind::Vector(_)
                | Kind::Map(_)
                | Kind::Set(_)
                | Kind::Quote(_)
                | Kind::Unquote { .. }
        ) {
            if let Some(first) = first_composite {
                return check_composites(
                    [first, value].into_iter().chain(values),
                    atoms,
                    context,
                    names,
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
    context: Context<'a, '_>,
    names: KeyNames<'_>,
    message: &'static str,
) -> Result<(), ReadError> {
    let mut composites = BTreeSet::new();
    for value in values {
        let value = value.without_metadata();
        let inserted = if let Some(atom) = key_atom(value, context, names) {
            atoms.insert(atom)
        } else if let Some(key) = composite(value, context) {
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

fn key_atom<'a>(
    value: &'a Form,
    context: Context<'a, '_>,
    names: KeyNames<'_>,
) -> Option<Atom<'a>> {
    let atom = atom(value, context)?;
    if !matches!(value.kind, Kind::SyntaxQuote(_)) {
        return Some(atom);
    }
    // Direct keys already carry their qualified spelling. A syntax quote
    // keeps its template, so qualify only the observed name for comparison.
    match (names, atom) {
        (KeyNames::ProjectIn(namespace), Atom::Symbol(name)) => Some(Atom::Symbol(
            qualify_symbol(&name, namespace).map_or(name, Cow::Owned),
        )),
        (KeyNames::ProjectIn(namespace), Atom::Keyword(name)) => Some(Atom::Keyword(
            qualify_keyword(&name, namespace).map_or(name, Cow::Owned),
        )),
        // A later recheck has the normalized keys and no namespace value
        // for the source prefix. The qualification pass checked this projection.
        (KeyNames::AlreadyQualified, Atom::Symbol(_) | Atom::Keyword(_)) => None,
        (_, atom) => Some(atom),
    }
}

fn duplicate(value: &Form, message: &'static str) -> ReadError {
    ReadError::invalid(value.span, message)
}

#[inline(always)]
fn atom<'a>(value: &'a Form, context: Context<'a, '_>) -> Option<Atom<'a>> {
    Some(match &value.kind {
        Kind::Nil => Atom::Nil,
        Kind::Bool(value) => Atom::Bool(*value),
        Kind::Character(value) => Atom::Character(*value),
        Kind::Number => {
            let spelling = &context.source[value.span.start.offset..value.span.end.offset];
            Atom::Number(number_key(spelling))
        }
        Kind::SymbolicNumber(SymbolicNumber::PositiveInfinity) => {
            Atom::Number(double_key(f64::INFINITY))
        }
        Kind::SymbolicNumber(SymbolicNumber::NegativeInfinity) => {
            Atom::Number(double_key(f64::NEG_INFINITY))
        }
        Kind::SymbolicNumber(SymbolicNumber::NaN) => Atom::NaN,
        Kind::String(value) => Atom::String(value),
        Kind::Utf16String(value) => Atom::Utf16String(value),
        Kind::Symbol(value) | Kind::AnonymousArgument(value) => Atom::Symbol(Cow::Borrowed(value)),
        Kind::Keyword(name) if name.starts_with("::") => {
            let spelling = match context.resolve_keyword {
                Some(resolve) => Cow::Owned(resolve(name, value.span.start)?),
                // A data reader can change what the same source spelling means.
                None if context.compare_raw_auto_keywords => Cow::Borrowed(name.as_str()),
                None => return None,
            };
            Atom::Keyword(spelling)
        }
        Kind::Keyword(name) => Atom::Keyword(Cow::Borrowed(name)),
        Kind::SyntaxQuote(_) => match Reader::immediate_syntax_quote_value(value)? {
            ImmediateSyntaxQuote::Nil => Atom::Nil,
            ImmediateSyntaxQuote::Form(form) => return atom(form.without_metadata(), context),
        },
        _ => return None,
    })
}

fn literal<'a>(value: &'a Form, context: Context<'a, '_>) -> Option<Literal<'a>> {
    literal_key(value, context, KeyNames::Plain)
}

fn literal_key<'a>(
    value: &'a Form,
    context: Context<'a, '_>,
    names: KeyNames<'_>,
) -> Option<Literal<'a>> {
    let value = value.without_metadata();
    match key_atom(value, context, names) {
        Some(Atom::Keyword(ref name))
            if context.resolve_keyword.is_none() && name.starts_with("::") =>
        {
            None
        }
        Some(atom) => Some(Literal::Atom(atom)),
        None => composite(value, context),
    }
}

fn composite<'a>(value: &'a Form, context: Context<'a, '_>) -> Option<Literal<'a>> {
    let children = |forms: &'a [Form]| {
        forms
            .iter()
            .map(|form| literal(form, context))
            .collect::<Option<Vec<_>>>()
    };
    Some(match &value.kind {
        Kind::List(items) | Kind::Vector(items) => Literal::Sequence(children(items)?.into()),
        Kind::Map(entries) => {
            let names = KeyNames::resolved_map(value, context.source);
            let (pairs, trailing) = entries.as_chunks::<2>();
            if !trailing.is_empty() {
                return None;
            }
            let mut entries = pairs
                .iter()
                .map(|[key, value]| {
                    // The reader reuses one boxed NaN, which compares equal
                    // in sequences and sets. Map equality looks up its keys,
                    // and array-map numeric key lookup does not match NaN.
                    if matches!(
                        key.without_metadata().kind,
                        Kind::SymbolicNumber(SymbolicNumber::NaN)
                    ) {
                        return None;
                    }
                    Some((literal_key(key, context, names)?, literal(value, context)?))
                })
                .collect::<Option<Vec<_>>>()?;
            entries.sort_unstable();
            Literal::Map(entries.into())
        }
        Kind::Set(items) => {
            let mut items = children(items)?;
            items.sort_unstable();
            Literal::Set(items.into())
        }
        Kind::Quote(_) | Kind::Unquote { .. } => {
            let (symbol, item) = value.prefixed_list()?;
            Literal::Sequence(Box::new([
                Literal::Atom(Atom::Symbol(Cow::Borrowed(symbol))),
                literal(item, context)?,
            ]))
        }
        _ => return None,
    })
}
