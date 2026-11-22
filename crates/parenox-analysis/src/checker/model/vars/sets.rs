//! Exact inputs to bounded clojure.set initializers.
use std::collections::BTreeMap;

use super::super::super::exact_set_member;
use super::super::type_forms::literal_union_member;
use super::initializers::MAX_SUBSTITUTED_BYTES;
use super::{Declarations, inert, prior_initializer};
use crate::reader::{Form, Kind};
use crate::types::{MAX_TUPLE_ITEMS, Type};

pub(super) fn union_members<'a>(
    declarations: &'a Declarations,
    forms: &'a [Form],
    source: &str,
    [left, right]: [&'a Form; 2],
    before: usize,
) -> Option<Vec<&'a Form>> {
    let mut left = literal_members(declarations, forms, source, left, before, MAX_TUPLE_ITEMS)?;
    let mut right = literal_members(
        declarations,
        forms,
        source,
        right,
        before,
        MAX_TUPLE_ITEMS - left.len(),
    )?;
    // The two-argument union conjoins into the larger set, keeping the left on ties.
    if left.len() < right.len() {
        std::mem::swap(&mut left, &mut right);
    }
    for (key, value) in right {
        left.entry(key).or_insert(value);
    }
    Some(left.into_values().collect())
}

fn literal_members<'a>(
    declarations: &'a Declarations,
    forms: &'a [Form],
    source: &str,
    input: &'a Form,
    before: usize,
    max_members: usize,
) -> Option<BTreeMap<Type, &'a Form>> {
    let input = match &input.kind {
        Kind::Symbol(local) if !local.contains('/') => {
            prior_initializer(declarations, forms, local, before)?
        }
        _ => input,
    };
    let quoted = input.quote_operand();
    let Kind::Set(items) = &quoted.unwrap_or(input).kind else {
        return None;
    };
    if items.len() > max_members
        || input.span.end.offset - input.span.start.offset > MAX_SUBSTITUTED_BYTES
        || !inert(input, false)
    {
        return None;
    }
    let mut seen = BTreeMap::new();
    for item in items {
        let value = match (quoted.is_some(), &item.kind) {
            (true, Kind::Symbol(_)) => item,
            (false, Kind::Quote(_) | Kind::List(_)) => {
                let value = item.quote_operand()?;
                matches!(value.kind, Kind::Symbol(_)).then_some(value)?
            }
            (
                _,
                Kind::Number
                | Kind::Nil
                | Kind::Bool(_)
                | Kind::Keyword(_)
                | Kind::String(_)
                | Kind::Character(_),
            ) => item,
            _ => return None,
        };
        let ty = if matches!(value.kind, Kind::Number) {
            Type::from_numeric_token(&source[value.span.start.offset..value.span.end.offset])
        } else {
            literal_union_member(&declarations.names, value, true)?
        };
        if seen
            .insert(exact_set_member(&ty)?.into_owned(), value)
            .is_some()
        {
            return None;
        }
    }
    Some(seen)
}
