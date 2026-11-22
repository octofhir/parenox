//! Statically capture local roots from consecutive inert `def` forms.
use std::collections::{BTreeMap, BTreeSet};

use super::super::Declarations;
use super::super::declarations::{SourceDeclaration, static_namespace_prelude};
use super::{CapturedInitializer, def_initializer};
use crate::reader::{Form, Kind, Span};

pub(super) const MAX_SUBSTITUTED_BYTES: usize = 64 * 1024;

pub(in crate::checker::model) fn prior_initializer<'a>(
    declarations: &'a Declarations,
    forms: &'a [Form],
    local: &'a str,
    before: usize,
) -> Option<&'a Form> {
    let mut definitions = declarations.source_declarations(forms, local);
    let SourceDeclaration::Value([head, name, tail @ ..]) = definitions.next()? else {
        return None;
    };
    if definitions.next().is_some()
        || !matches!(head.symbol(), Some("def" | "clojure.core/def"))
        || name.span.end.offset >= before
        || name.metadata(":dynamic").is_some_and(|value| {
            !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
        })
    {
        return None;
    }
    def_initializer(tail)
}

pub(super) fn adjacent_root<'a>(
    declarations: &Declarations,
    forms: &'a [Form],
    source: &str,
    index: usize,
    captured: &BTreeMap<usize, CapturedInitializer>,
) -> Option<(usize, &'a str, &'a Form)> {
    let (previous, name, value, allow_const) =
        adjacent_definition(declarations, forms, index, Some(captured))?;
    let (current_name, initializer) = plain_def(&forms[index], allow_const)?;
    (name != current_name
        && source[initializer.span.start.offset..initializer.span.end.offset].contains(name))
    .then_some((previous, name, value))
}

pub(in crate::checker::model) fn adjacent_literal_def<'a>(
    declarations: &Declarations,
    forms: &'a [Form],
    index: usize,
) -> Option<(&'a str, &'a Form)> {
    adjacent_definition(declarations, forms, index, None).map(|(_, name, value, _)| (name, value))
}

fn adjacent_definition<'a>(
    declarations: &Declarations,
    forms: &'a [Form],
    index: usize,
    captured: Option<&BTreeMap<usize, CapturedInitializer>>,
) -> Option<(usize, &'a str, &'a Form, bool)> {
    let current = forms.get(index)?;
    let (previous_index, previous) =
        forms[..index].iter().enumerate().rev().find(|(index, _)| {
            declarations
                .discarded_comments
                .binary_search(index)
                .is_err()
        })?;
    let captured = captured.and_then(|values| values.get(&previous_index));
    let allow_const = matches!(captured, Some(CapturedInitializer::PlatformLong(_)));
    let (name, value) = plain_def(previous, allow_const)?;
    if !forms.first().is_some_and(static_namespace_prelude)
        || declarations.forward_vars.contains_key(name)
        || declarations
            .source_declarations(forms, name)
            .nth(1)
            .is_some()
        || (captured.is_none() && !inert(value, false))
    {
        return None;
    }
    declarations.reader_namespace_before(forms, previous.span.start.offset)?;
    // Discarded forms disappear from the AST, but their reader handlers may
    // have run between the initialization and the use of the root.
    if declarations.has_unchecked_dispatch(Span {
        start: previous.span.start,
        end: current.span.end,
    }) {
        return None;
    }
    Some((previous_index, name, value, allow_const))
}

fn plain_def(form: &Form, allow_const: bool) -> Option<(&str, &Form)> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [op, name, tail @ ..] = items.as_slice() else {
        return None;
    };
    if !matches!(&op.kind, Kind::Symbol(symbol) if symbol == "def") {
        return None;
    }
    let mut name = name;
    while let Kind::Metadata(meta, target) = &name.kind {
        let Kind::Map(entries) = &meta.kind else {
            return None;
        };
        let (entries, remainder) = entries.as_chunks::<2>();
        if entries.is_empty()
            || !remainder.is_empty()
            || (!allow_const && (entries.len() != 1 || matches!(target.kind, Kind::Metadata(..))))
            || !entries.iter().all(|[key, value]| {
                matches!(&key.kind, Kind::Keyword(keyword)
                    if keyword == ":private" || (allow_const && keyword == ":const"))
                    && matches!(value.kind, Kind::Bool(true))
            })
        {
            return None;
        }
        name = target;
    }
    let Kind::Symbol(name) = &name.kind else {
        return None;
    };
    (!name.contains('/')).then_some((name, def_initializer(tail)?))
}

pub(super) fn capture_literal_run(
    declarations: &Declarations,
    forms: &[Form],
    index: usize,
    known_root: impl Fn(&str) -> bool,
) -> Option<Form> {
    let current = forms.get(index)?;
    let (current_name, initializer) = plain_def(current, false)?;
    let mut needed = BTreeSet::new();
    collect_roots(initializer, &mut needed)?;
    if needed.is_empty() || !forms.first().is_some_and(static_namespace_prelude) {
        return None;
    }
    let mut roots = BTreeMap::new();
    for (at, previous) in forms[..index].iter().enumerate().rev() {
        if declarations.discarded_comments.binary_search(&at).is_ok() {
            continue;
        }
        let (name, value) = plain_def(previous, false)?;
        if !inert(value, false) {
            return None;
        }
        if !needed.remove(name) {
            continue;
        }
        if name == current_name
            || declarations.forward_vars.contains_key(name)
            || declarations
                .source_declarations(forms, name)
                .nth(1)
                .is_some()
            || !known_root(name)
        {
            return None;
        }
        roots.insert(name, value);
        if needed.is_empty() {
            declarations.reader_namespace_before(forms, previous.span.start.offset)?;
            if declarations.has_unchecked_dispatch(Span {
                start: previous.span.start,
                end: current.span.end,
            }) {
                return None;
            }
            return substitute_roots(initializer, |name| roots.get(name).copied());
        }
    }
    None
}

fn collect_roots<'a>(form: &'a Form, names: &mut BTreeSet<&'a str>) -> Option<()> {
    match &form.kind {
        Kind::Symbol(name) => {
            names.insert(name);
        }
        Kind::Vector(items) => {
            for value in items {
                collect_roots(value, names)?;
            }
        }
        Kind::Map(entries) => {
            for [key, value] in entries.as_chunks::<2>().0 {
                inert(key, false).then_some(())?;
                collect_roots(value, names)?;
            }
        }
        _ => inert(form, false).then_some(())?,
    }
    Some(())
}

pub(super) fn substitute(initializer: &Form, name: &str, root: &Form) -> Option<Form> {
    substitute_roots(initializer, |symbol| (symbol == name).then_some(root))
}

fn substitute_roots<'a>(
    initializer: &Form,
    lookup: impl Fn(&str) -> Option<&'a Form>,
) -> Option<Form> {
    let initializer_bytes = initializer
        .span
        .end
        .offset
        .checked_sub(initializer.span.start.offset)?;
    let budget = MAX_SUBSTITUTED_BYTES.checked_sub(initializer_bytes)?;
    let mut remaining = budget;
    let mut result = initializer.clone();
    replace_values(&mut result, &lookup, &mut remaining)?;
    (remaining < budget).then_some(result)
}

fn replace_values<'a>(
    form: &mut Form,
    lookup: &impl Fn(&str) -> Option<&'a Form>,
    remaining: &mut usize,
) -> Option<()> {
    match &mut form.kind {
        Kind::Symbol(symbol) => {
            let root = lookup(symbol)?;
            let root_bytes = root.span.end.offset.checked_sub(root.span.start.offset)?;
            *remaining = remaining.checked_sub(root_bytes)?;
            *form = root.clone();
        }
        Kind::Vector(items) => {
            for value in items {
                replace_values(value, lookup, remaining)?;
            }
        }
        Kind::Map(entries) => {
            for [key, value] in entries.as_chunks_mut::<2>().0 {
                if !inert(key, false) {
                    return None;
                }
                replace_values(value, lookup, remaining)?;
            }
        }
        _ => inert(form, false).then_some(())?,
    }
    Some(())
}

pub(in crate::checker::model) fn inert(form: &Form, quoted: bool) -> bool {
    match &form.kind {
        Kind::Nil
        | Kind::Bool(_)
        | Kind::Number
        | Kind::SymbolicNumber(_)
        | Kind::String(_)
        | Kind::Utf16String(_)
        | Kind::Character(_)
        | Kind::Keyword(_) => true,
        Kind::Symbol(_) => quoted,
        Kind::Quote(inner) => inert(inner, true),
        Kind::Unquote { form, .. } => quoted && inert(form, true),
        Kind::List(items) if !quoted => match items.as_slice() {
            [] => true,
            [head, operand] if matches!(&head.kind, Kind::Symbol(name) if name == "quote") => {
                inert(operand, true)
            }
            _ => false,
        },
        Kind::List(items) | Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => {
            items.iter().all(|item| inert(item, quoted))
        }
        Kind::AutoNamespacedMap { entries, .. } => entries.iter().all(|item| inert(item, quoted)),
        _ => false,
    }
}
