//! Statically capture a local root from the immediately preceding `def`.
use super::super::Declarations;
use super::super::declarations::static_namespace_prelude;
use super::def_initializer;
use crate::reader::{Form, Kind};

const MAX_SUBSTITUTED_BYTES: usize = 64 * 1024;

pub(super) fn adjacent_root<'a>(
    declarations: &'a Declarations,
    forms: &'a [Form],
    source: &str,
    index: usize,
) -> Option<(&'a str, &'a Form)> {
    let (name, value) = adjacent_literal_def(declarations, forms, source, index)?;
    let (current_name, initializer) = plain_def(&forms[index])?;
    (name != current_name
        && source[initializer.span.start.offset..initializer.span.end.offset].contains(name))
    .then_some((name, value))
}

pub(in crate::checker::model) fn adjacent_literal_def<'a>(
    declarations: &'a Declarations,
    forms: &'a [Form],
    source: &str,
    index: usize,
) -> Option<(&'a str, &'a Form)> {
    let previous = forms.get(index.checked_sub(1)?)?;
    let current = &forms[index];
    let (name, value) = plain_def(previous)?;
    if !forms.first().is_some_and(static_namespace_prelude)
        || declarations.forward_vars.contains_key(name)
        || declarations
            .source_declarations(forms, name)
            .nth(1)
            .is_some()
        || !inert(value, false)
    {
        return None;
    }
    declarations.reader_namespace_before(forms, previous.span.start.offset)?;
    // Discarded forms disappear from the AST, but their reader handlers may
    // have run between the initialization and the use of the root.
    let text = &source[previous.span.start.offset..current.span.end.offset];
    if text.contains("#_") || text.contains("#=") {
        return None;
    }
    Some((name, value))
}

fn plain_def(form: &Form) -> Option<(&str, &Form)> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [op, name, tail @ ..] = items.as_slice() else {
        return None;
    };
    if !matches!(&op.kind, Kind::Symbol(symbol) if symbol == "def") {
        return None;
    }
    let name = match &name.kind {
        Kind::Metadata(meta, target) => {
            let Kind::Map(entries) = &meta.kind else {
                return None;
            };
            let [key, value] = entries.as_slice() else {
                return None;
            };
            if !matches!(&key.kind, Kind::Keyword(keyword) if keyword == ":private")
                || !matches!(value.kind, Kind::Bool(true))
            {
                return None;
            }
            target
        }
        _ => name,
    };
    let Kind::Symbol(name) = &name.kind else {
        return None;
    };
    (!name.contains('/')).then_some((name, def_initializer(tail)?))
}

pub(super) fn substitute(initializer: &Form, name: &str, root: &Form) -> Option<Form> {
    let root_bytes = root.span.end.offset.checked_sub(root.span.start.offset)?;
    let initializer_bytes = initializer
        .span
        .end
        .offset
        .checked_sub(initializer.span.start.offset)?;
    let budget = MAX_SUBSTITUTED_BYTES.checked_sub(initializer_bytes)?;
    let mut remaining = budget;
    let mut result = initializer.clone();
    replace_values(&mut result, name, root, root_bytes, &mut remaining)?;
    (remaining < budget).then_some(result)
}

fn replace_values(
    form: &mut Form,
    name: &str,
    root: &Form,
    root_bytes: usize,
    remaining: &mut usize,
) -> Option<()> {
    match &mut form.kind {
        Kind::Symbol(symbol) if symbol == name => {
            *remaining = remaining.checked_sub(root_bytes)?;
            *form = root.clone();
        }
        Kind::Vector(items) => {
            for value in items {
                replace_values(value, name, root, root_bytes, remaining)?;
            }
        }
        Kind::Map(entries) => {
            for [key, value] in entries.as_chunks_mut::<2>().0 {
                if !inert(key, false) {
                    return None;
                }
                replace_values(value, name, root, root_bytes, remaining)?;
            }
        }
        _ => inert(form, false).then_some(())?,
    }
    Some(())
}

pub(super) fn inert(form: &Form, quoted: bool) -> bool {
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
        Kind::List(items) if !quoted => items.is_empty(),
        Kind::List(items) | Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => {
            items.iter().all(|item| inert(item, quoted))
        }
        _ => false,
    }
}
