//! Identify files where standard method registration leaves a known scope.
use std::collections::BTreeSet;

use super::super::vars::{def_initializer, literal};
use super::{
    Declarations, function_parts, multimethod_parts, static_namespace_prelude, symbols_match_inner,
};
use crate::checker::has_evaluated_metadata;
use crate::reader::{Form, Kind};

impl Declarations {
    pub(super) fn has_static_method_scope(&self, forms: &[Form]) -> bool {
        if self.method_forms.is_empty()
            || !self.names.required_libraries.contains("clojure.string")
            || !self.all_refers.is_empty()
            || self.diagnostics.iter().any(|item| item.severity == "error")
            || !forms.first().is_some_and(static_namespace_prelude)
        {
            return false;
        }
        let mut targets = BTreeSet::new();
        forms.iter().enumerate().skip(1).all(|(index, form)| {
            let Kind::List(items) = &form.kind else {
                return false;
            };
            let [head, name, tail @ ..] = items.as_slice() else {
                return false;
            };
            let Some(name_symbol) = name.symbol() else {
                return false;
            };
            if !matches!(&name.kind, Kind::Symbol(_)) || name_symbol.contains('/') {
                return false;
            }
            let Some(operator) = head.symbol() else {
                return false;
            };
            let (namespace, local) = self.names.resolve_parts(operator);
            if operator != "def"
                && (namespace != "clojure.core"
                    || !self.is_unshadowed_core_operator(forms, head, local))
            {
                return false;
            }
            match local {
                "def" if operator == "def" => def_initializer(tail).is_some_and(inert),
                "declare" => tail
                    .iter()
                    .all(|name| matches!(&name.kind, Kind::Symbol(name) if !name.contains('/'))),
                "defn" | "defn-" if self.functions.binary_search(&index).is_ok() => {
                    function_parts(tail).1.all(inert_attribute)
                }
                "defprotocol" if self.protocols.binary_search(&index).is_ok() => {
                    standard_protocol(tail)
                }
                "defmulti" if self.is_source_multimethod(forms, name_symbol, name.span) => {
                    let (attrs, dispatcher) = multimethod_parts(tail);
                    let Some((dispatch, options)) = dispatcher.split_first() else {
                        return false;
                    };
                    let (pairs, remainder) = options.as_chunks::<2>();
                    let (attrs, attr_tail) = attrs.as_chunks::<2>();
                    attr_tail.is_empty()
                        && attrs.iter().all(inert_attribute)
                        && (inert(dispatch)
                            || (!has_evaluated_metadata(dispatch)
                                && self.multimethod_clauses(forms, tail).is_some()))
                        && remainder.is_empty()
                        && pairs
                            .iter()
                            .all(|[key, value]| key.keyword() == Some(":default") && inert(value))
                        && targets.insert(name_symbol)
                }
                "defmethod" if self.method_forms.binary_search(&index).is_ok() => {
                    targets.contains(name_symbol) && tail.first().is_some_and(inert)
                }
                _ => false,
            }
        }) && forms.iter().skip(1).all(|form| {
            symbols_match_inner::<true>(form, &mut |name| {
                !matches!(
                    self.names.resolve_parts(name),
                    (
                        "clojure.core",
                        "alias"
                            | "ns-unalias"
                            | "refer"
                            | "ns-unmap"
                            | "in-ns"
                            | "require"
                            | "use"
                            | "load"
                            | "load-file"
                            | "load-string"
                            | "eval"
                            | "intern"
                            | "alter-var-root"
                            | "with-redefs"
                            | "with-redefs-fn"
                            | "set!"
                    )
                )
            })
        })
    }
}

fn inert(form: &Form) -> bool {
    literal(form) && !has_evaluated_metadata(form)
}

fn inert_attribute([key, value]: &[Form; 2]) -> bool {
    matches!(&key.kind, Kind::Keyword(key) if !matches!(key.as_str(), ":macro" | ":inline" | ":inline-arities"))
        && inert(value)
}

fn standard_protocol(mut tail: &[Form]) -> bool {
    if tail.first().is_some_and(Form::is_string) {
        tail = &tail[1..];
    }
    tail.iter().all(|method| {
        let Kind::List(items) = &method.kind else {
            return false;
        };
        let [name, clauses @ ..] = items.as_slice() else {
            return false;
        };
        let clauses = match clauses.split_last() {
            Some((doc, rest)) if doc.is_string() => rest,
            _ => clauses,
        };
        matches!(&name.kind, Kind::Symbol(name) if !name.contains('/'))
            && !clauses.is_empty()
            && clauses.iter().all(|clause| {
                matches!(&clause.kind, Kind::Vector(args)
                    if !args.is_empty() && args.iter().all(|arg| matches!(&arg.kind, Kind::Symbol(_))))
            })
    })
}
