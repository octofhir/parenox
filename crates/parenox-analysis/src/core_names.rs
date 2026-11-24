//! Public Vars and modeled functions in the standard clojure.core namespace.

use std::{collections::HashMap, sync::LazyLock};

use crate::reader::{Kind, read_edn};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoreBinding {
    Macro,
    Var,
}

pub(crate) fn core_binding(name: &str) -> Option<CoreBinding> {
    static BINDINGS: LazyLock<HashMap<Box<str>, CoreBinding>> = LazyLock::new(|| {
        let forms = read_edn(include_str!("core_names/jvm.edn")).expect("valid core catalog EDN");
        forms[0]
            .map()
            .expect("core catalog is a map")
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[category, names]| {
                let binding = match category.keyword() {
                    Some(":macros") => CoreBinding::Macro,
                    Some(":vars") => CoreBinding::Var,
                    _ => unreachable!("unknown core catalog category"),
                };
                names
                    .vector()
                    .expect("core names are listed in vectors")
                    .iter()
                    .map(move |name| {
                        let Kind::String(name) = &name.kind else {
                            unreachable!("core names are strings");
                        };
                        (name.as_str().into(), binding)
                    })
            })
            .collect()
    });
    BINDINGS.get(name).copied()
}

pub(crate) fn known_core_function(name: &str) -> bool {
    matches!(
        name,
        "*" | "+"
            | "-"
            | "/"
            | "<"
            | "<="
            | "="
            | "=="
            | ">"
            | ">="
            | "apply"
            | "assoc"
            | "assoc-in"
            | "boolean"
            | "boolean?"
            | "byte"
            | "char"
            | "char?"
            | "conj"
            | "cons"
            | "contains?"
            | "count"
            | "dec"
            | "deref"
            | "disj"
            | "dissoc"
            | "double"
            | "empty?"
            | "even?"
            | "every?"
            | "ex-data"
            | "ex-info"
            | "filter"
            | "filterv"
            | "first"
            | "float"
            | "fnil"
            | "get"
            | "get-in"
            | "group-by"
            | "hash-map"
            | "identity"
            | "inc"
            | "int"
            | "int?"
            | "integer?"
            | "into"
            | "key"
            | "keys"
            | "keyword"
            | "keyword?"
            | "last"
            | "list"
            | "long"
            | "map"
            | "map-indexed"
            | "map?"
            | "mapcat"
            | "mapv"
            | "merge"
            | "merge-with"
            | "next"
            | "nil?"
            | "not"
            | "nth"
            | "odd?"
            | "number?"
            | "partial"
            | "pos?"
            | "prn"
            | "println"
            | "range"
            | "re-find"
            | "re-matches"
            | "reduce"
            | "reduce-kv"
            | "rest"
            | "seq"
            | "set"
            | "set?"
            | "short"
            | "some"
            | "some?"
            | "str"
            | "string?"
            | "subs"
            | "symbol?"
            | "update"
            | "update-in"
            | "update-keys"
            | "update-vals"
            | "val"
            | "vals"
            | "vec"
            | "vector"
            | "vector?"
            | "zero?"
    )
}
