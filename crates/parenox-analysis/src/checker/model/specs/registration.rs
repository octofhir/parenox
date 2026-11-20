//! Source forms that can run code between a tagged spec and its registrations.
use super::super::{
    Declarations, Names,
    type_forms::{named_tagged_branches, resolves, tagged_projection_branches},
    vars,
};
use crate::reader::{Form, Kind};

pub(super) fn barriers(declarations: &Declarations, forms: &[Form]) -> Vec<usize> {
    let names = &declarations.names;
    let specs = declarations.specs.values();
    let Some(first) = specs
        .clone()
        .filter(|spec| {
            tagged_projection_branches(names, &spec.body).is_some()
                || named_tagged_branches(names, &spec.body).is_some()
        })
        .map(|spec| spec.body.span.end.offset)
        .min()
    else {
        return Vec::new();
    };
    let last = specs
        .map(|spec| spec.body.span.end.offset)
        .max()
        .unwrap_or(first);
    let start = forms.partition_point(|form| form.span.start.offset < first);
    let end = forms.partition_point(|form| form.span.start.offset < last);
    forms[start..end]
        .iter()
        .filter(|form| !inert_declaration(names, form))
        .map(|form| form.span.start.offset)
        .collect()
}

fn inert_declaration(names: &Names, form: &Form) -> bool {
    let Kind::List(items) = &form.without_metadata().kind else {
        return false;
    };
    let [head, name, args @ ..] = items.as_slice() else {
        return false;
    };
    if matches!(head.symbol(), Some("def" | "clojure.core/def")) {
        return args.is_empty() || vars::def_initializer(args).is_some_and(vars::literal);
    }
    name.keyword().is_some()
        && resolves(names, head, "clojure.spec.alpha", "def")
        && matches!(args, [body] if inert_spec(names, body))
}

fn inert_spec(names: &Names, form: &Form) -> bool {
    match &form.without_metadata().kind {
        Kind::Symbol(_) | Kind::AnonymousFunction(_) => true,
        Kind::Vector(forms) | Kind::Map(forms) | Kind::Set(forms) => {
            forms.iter().all(|form| inert_spec(names, form))
        }
        Kind::List(items) => {
            let Some((head, args)) = items.split_first() else {
                return true;
            };
            if matches!(head.symbol(), Some("quote" | "fn*"))
                || resolves(names, head, "clojure.core", "fn")
            {
                return true;
            }
            let Some(symbol) = head.symbol() else {
                return false;
            };
            let (_, name) = names.resolve_parts(symbol);
            matches!(
                name,
                "and" | "or" | "keys" | "coll-of" | "map-of" | "tuple" | "nilable" | "conformer"
            ) && resolves(names, head, "clojure.spec.alpha", name)
                && args.iter().all(|form| inert_spec(names, form))
        }
        _ => vars::literal(form),
    }
}
