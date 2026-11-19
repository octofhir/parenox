//! Built-in spec conversions whose input checks and effects are known.
use super::Names;
use crate::checker::model::specs::SpecConversion;
use crate::reader::{Form, Kind};

pub(in crate::checker::model) fn source_conversion(
    names: &Names,
    form: &Form,
) -> Option<SpecConversion> {
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

fn resolves(names: &Names, form: &Form, namespace: &str, name: &str) -> bool {
    form.symbol().is_some_and(|symbol| {
        names.resolve_parts(symbol) == (namespace, name)
            && (symbol.contains('/')
                || (names.spec_core_scope_known && !names.spec_core_shadows.contains(symbol)))
    })
}
