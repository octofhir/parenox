use super::{Names, is_wrapper_input, projection_key, resolves};
use crate::checker::model::specs::{
    GuardedMapProjection, ProjectedFields, ProjectedValue, SpecConversion,
};
use crate::reader::{Form, Kind};
use crate::types::Type;
use std::{collections::BTreeSet, sync::Arc};

const MAX_TERMS: usize = 8;

pub(super) fn parse(
    names: &Names,
    guard: &Form,
    result: &Form,
    param: &str,
) -> Option<SpecConversion> {
    let parser = ProjectionParser { names, param };
    let guards = match &guard.kind {
        Kind::List(items)
            if let Some((head, terms)) = items.split_first()
                && parser.core(head, "and") =>
        {
            terms
        }
        _ => std::slice::from_ref(guard),
    };
    if !(1..=MAX_TERMS).contains(&guards.len()) {
        return None;
    }
    let guards = guards
        .iter()
        .map(|form| parser.lookup(form))
        .collect::<Option<_>>()?;
    let fields = parser.map(result, 0)?;
    Some(SpecConversion::GuardedMap(Arc::new(GuardedMapProjection {
        guards,
        fields,
    })))
}

struct ProjectionParser<'a> {
    names: &'a Names,
    param: &'a str,
}

impl ProjectionParser<'_> {
    fn core(&self, form: &Form, name: &str) -> bool {
        form.symbol() != Some(self.param) && resolves(self.names, form, "clojure.core", name)
    }

    fn key(&self, form: &Form) -> Option<Type> {
        matches!(form.kind, Kind::Keyword(_))
            .then(|| projection_key(self.names, form))
            .flatten()
    }

    fn lookup(&self, form: &Form) -> Option<Type> {
        let Kind::List(items) = &form.kind else {
            return None;
        };
        let [key, input] = items.as_slice() else {
            return None;
        };
        is_wrapper_input(input, self.param)
            .then(|| self.key(key))
            .flatten()
    }

    fn map(&self, form: &Form, depth: usize) -> Option<ProjectedFields> {
        let Kind::Map(entries) = &form.kind else {
            return None;
        };
        if entries.len() > 2 * MAX_TERMS || depth == MAX_TERMS {
            return None;
        }
        let mut seen = BTreeSet::new();
        entries
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[key, value]| {
                let key = projection_key(self.names, key)?;
                if !seen.insert(key.clone()) {
                    return None;
                }
                let value = match &value.kind {
                    Kind::Map(_) => ProjectedValue::Map(self.map(value, depth + 1)?),
                    Kind::List(items)
                        if let [head, accessor] = items.as_slice()
                            && self.core(head, "boolean") =>
                    {
                        let Kind::List(call) = &accessor.kind else {
                            return None;
                        };
                        let [get, input, key, default] = call.as_slice() else {
                            return None;
                        };
                        if !self.core(get, "get")
                            || !is_wrapper_input(input, self.param)
                            || !matches!(default.kind, Kind::Bool(false))
                        {
                            return None;
                        }
                        ProjectedValue::BooleanLookup(self.key(key)?)
                    }
                    _ => ProjectedValue::Lookup(self.lookup(value)?),
                };
                Some((key, value))
            })
            .collect()
    }
}
