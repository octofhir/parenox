//! Built-in spec conversions whose input checks and effects are known.
use std::collections::BTreeMap;

use super::{super::declarations::SourceDeclaration, Names};
use crate::checker::model::Declarations;
use crate::checker::model::specs::SpecConversion;
use crate::reader::{Form, Kind};
use crate::types::{SPEC_INVALID, Type};

const MAX_ENUM_ENTRIES: usize = 32;

pub(in crate::checker::model) fn source_conversion(
    names: &Names,
    form: &Form,
) -> Option<SpecConversion> {
    if resolves(names, form, "clojure.core", "identity") {
        return Some(SpecConversion::Identity);
    }
    let Kind::List(items) = &form.without_metadata().kind else {
        return None;
    };
    let (head, stages) = items.split_first()?;
    if !resolves(names, head, "clojure.spec.alpha", "and") {
        return None;
    }
    let [predicate, stage] = stages else {
        return None;
    };
    let Kind::List(items) = &stage.without_metadata().kind else {
        return None;
    };
    let [head, function] = items.as_slice() else {
        return None;
    };
    (resolves(names, head, "clojure.spec.alpha", "conformer")
        && resolves(names, predicate, "clojure.core", "integer?")
        && resolves(names, function, "clojure.core", "int"))
    .then_some(SpecConversion::CheckedInt32)
}

/// An exact source pattern where the same literal map checks and converts an int.
pub(in crate::checker::model) fn enum_conversion(
    declarations: &Declarations,
    forms: &[Form],
    source: &str,
    form: &Form,
) -> Option<SpecConversion> {
    let names = &declarations.names;
    let Kind::List(items) = &form.without_metadata().kind else {
        return None;
    };
    let [head, predicate, lookup, stage] = items.as_slice() else {
        return None;
    };
    let Kind::List(stage) = &stage.without_metadata().kind else {
        return None;
    };
    let [conformer, convert] = stage.as_slice() else {
        return None;
    };
    if !names.spec_core_scope_known
        || !resolves(names, head, "clojure.spec.alpha", "and")
        || !resolves(names, predicate, "clojure.core", "int?")
        || !resolves(names, conformer, "clojure.spec.alpha", "conformer")
    {
        return None;
    }
    let local = local_var(names, lookup)?;
    if local_var(names, convert)? != local {
        return None;
    }
    let mut definitions = declarations.source_declarations(forms, local);
    let SourceDeclaration::Value(definition) = definitions.next()? else {
        return None;
    };
    let [head, name, tail @ ..] = definition else {
        return None;
    };
    if definitions.next().is_some()
        || !matches!(head.symbol(), Some("def" | "clojure.core/def"))
        || name.span.end.offset >= form.span.start.offset
        || name.metadata(":dynamic").is_some_and(|value| {
            !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
        })
    {
        return None;
    }
    let initializer = match tail {
        [value] => value,
        [doc, value] if doc.is_string() => value,
        _ => return None,
    };
    let entries = initializer.map()?.as_chunks::<2>().0;
    if entries.len() > MAX_ENUM_ENTRIES {
        return None;
    }
    let mut values = BTreeMap::new();
    for [key, value] in entries {
        let Type::Integer(key) = Type::integer_literal(key, source)? else {
            return None;
        };
        let value = match &value.without_metadata().kind {
            Kind::Nil | Kind::Bool(false) => None,
            Kind::Keyword(keyword) => {
                let keyword = names.key(keyword);
                (keyword != SPEC_INVALID).then_some(keyword)
            }
            _ => return None,
        };
        if values.insert(key, value).is_some() {
            return None;
        }
    }
    Some(SpecConversion::IntEnum(
        values
            .into_iter()
            .filter_map(|(key, value)| Some((key, value?)))
            .collect(),
    ))
}

fn local_var<'a>(names: &Names, form: &'a Form) -> Option<&'a str> {
    let symbol = form.symbol()?;
    if let Some((_, local)) = symbol.split_once('/') {
        (names.resolve_parts(symbol).0 == names.namespace).then_some(local)
    } else {
        (!names.refers.contains_key(symbol) && !names.imports.contains_key(symbol))
            .then_some(symbol)
    }
}

fn resolves(names: &Names, form: &Form, namespace: &str, name: &str) -> bool {
    form.symbol().is_some_and(|symbol| {
        names.resolve_parts(symbol) == (namespace, name)
            && (symbol.contains('/')
                || (names.spec_core_scope_known && !names.spec_core_shadows.contains(symbol)))
    })
}
