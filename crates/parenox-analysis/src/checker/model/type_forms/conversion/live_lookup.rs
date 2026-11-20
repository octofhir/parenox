use super::{
    Names, guarded_map::map_fields, is_wrapper_input, local_var, projection_key, resolves,
    unary_function,
};
use crate::checker::model::specs::{LiveLookupProjection, ProjectedValue, SpecConversion};
use crate::reader::{Form, Kind};
use std::sync::Arc;

pub(super) fn parse(names: &Names, namespace: &str, form: &Form) -> Option<SpecConversion> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [head, function] = items.as_slice() else {
        return None;
    };
    if !resolves(names, head, "clojure.spec.alpha", "conformer") {
        return None;
    }
    let (param, result) = unary_function(names, function)?;
    let mut var = None;
    let fields = map_fields(names, result, 0, &mut |value| {
        if let Kind::Bool(value) = value.kind {
            return Some(ProjectedValue::Boolean(value));
        }
        if var.is_some() {
            return None;
        }
        let (table, fallback) = fallback_lookup(names, namespace, param, value)?;
        var = Some(table);
        Some(ProjectedValue::LiveFallback(projection_key(
            names, fallback,
        )?))
    })?;
    Some(SpecConversion::LiveLookupMap(Arc::new(
        LiveLookupProjection { var: var?, fields },
    )))
}

fn fallback_lookup<'a>(
    names: &Names,
    namespace: &str,
    param: &str,
    form: &'a Form,
) -> Option<(String, &'a Form)> {
    let core = |form: &Form, name| {
        matches!(form.kind, Kind::Symbol(_))
            && form.symbol() != Some(param)
            && resolves(names, form, "clojure.core", name)
    };
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [or, first, fallback] = items.as_slice() else {
        return None;
    };
    let (Kind::List(first), Kind::List(fallback)) = (&first.kind, &fallback.kind) else {
        return None;
    };
    let ([get, table, input], [fallback_get, fallback_table, key]) =
        (first.as_slice(), fallback.as_slice())
    else {
        return None;
    };
    if !core(or, "or")
        || !core(get, "get")
        || !core(fallback_get, "get")
        || !is_wrapper_input(input, param)
        || !matches!(key.kind, Kind::Keyword(_))
    {
        return None;
    }
    let table_var = |form: &Form| {
        let Kind::Symbol(symbol) = &form.kind else {
            return None;
        };
        if symbol == param {
            return None;
        }
        let name = if symbol.contains('/') {
            let (resolved, local) = names.resolve_parts(symbol);
            (resolved == namespace).then_some(local)?
        } else {
            (names.namespace == namespace).then_some(())?;
            local_var(names, form)?
        };
        Some(format!("{namespace}/{name}"))
    };
    let var = table_var(table)?;
    (var == table_var(fallback_table)?).then_some((var, key))
}
