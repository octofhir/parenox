//! Definite call-site findings in function bodies without a checked signature.
use super::contracts::function_parts;
use super::declarations::symbols_match;
use super::externs::ExternalDeclaration;
use super::{Declarations, DeclaredValue, Globals, Model};
use crate::checker::{
    Checker, Env, Value,
    function_forms::{Clause, function_clauses},
};
use crate::reader::{Form, Kind};
use crate::types::{Arity, Type};
use crate::{ContractKind, ContractOrigin, Diagnostic};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

const STRING_OPERATORS: &[&str] = &[
    "upper-case",
    "lower-case",
    "capitalize",
    "trim",
    "blank?",
    "join",
    "replace",
    "split",
];
const STRING_MEMBERS: &[&str] = &["charAt", "length", "startsWith", "endsWith", "indexOf"];

impl Declarations {
    pub(crate) fn check_untyped_calls(
        &self,
        forms: &[Form],
        model: &Model,
        globals: &Globals,
        source: &str,
    ) -> Vec<Diagnostic> {
        let string_namespace = self
            .dependencies
            .iter()
            .any(|name| name == "clojure.string");
        let jvm_string = STRING_MEMBERS.iter().any(|member| source.contains(member));
        let extern_calls = globals.externs.as_ref().is_some_and(|externs| {
            std::iter::once(model.namespace())
                .chain(model.names.required_libraries.iter().map(String::as_str))
                .chain(
                    model
                        .names
                        .proven_alias_libraries
                        .iter()
                        .map(String::as_str),
                )
                .any(|namespace| {
                    externs.namespaces.get(namespace).is_some_and(|table| {
                        table
                            .values()
                            .any(|entry| matches!(entry, ExternalDeclaration::Function(_)))
                    })
                })
        });
        if (!extern_calls && !string_namespace && !source.contains("subs") && !jvm_string)
            || !self.all_refers.is_empty()
            || !uses_standard_partial_calls(globals)
        {
            return Vec::new();
        }
        let methods = self.static_method_scope;
        let mut definitions = self.unchecked_definitions(forms, model).peekable();
        let functions = definitions.peek().is_some() && self.has_known_var_surface(forms);
        if !methods && !functions {
            return Vec::new();
        }
        let shadowed = if methods {
            self.partial_shadowed(forms)
        } else {
            self.top_level_names(forms)
        };
        let definitions = definitions.filter(|_| functions);
        let methods = self
            .method_forms
            .iter()
            .filter(|_| methods)
            .map(|&index| &forms[index]);
        let operators: Vec<_> = STRING_OPERATORS
            .iter()
            .copied()
            .filter(|_| string_namespace)
            .chain(STRING_MEMBERS.iter().copied().filter(|_| jvm_string))
            .chain(["subs"])
            .chain(model.names.refers.iter().filter_map(|(local, target)| {
                partial_string_operator(target).then_some(local.as_str())
            }))
            .collect();
        let mut diagnostics = Vec::new();
        for (form, method) in definitions
            .map(|form| (form, false))
            .chain(methods.map(|form| (form, true)))
        {
            let text = &source[form.span.start.offset..form.span.end.offset];
            let has_string_call = operators.iter().any(|&operator| text.contains(operator));
            let has_extern_call = extern_calls
                && !method
                && !has_string_call
                && !symbols_match(form, &mut |name| {
                    !matches!(model.declared_value(name, globals, form.span.start.offset),
                        Some(DeclaredValue::Function(contract))
                            if contract.origin.kind == ContractKind::Extern
                                && model.partial_extern_available(globals, &contract.origin, false))
                });
            if !has_string_call && !has_extern_call {
                continue;
            }
            let Some(clauses) = untyped_clauses(form, method) else {
                continue;
            };
            let mut checker = Checker::new(model, globals, source);
            checker.partial_scope = Some(Cow::Borrowed(&shadowed));
            checker.partial_method_body = method;
            for clause in clauses.values() {
                checker.untyped_clause(clause);
            }
            diagnostics.extend(checker.diagnostics.into_iter().filter(|diagnostic| {
                diagnostic.contract.as_ref().is_some_and(|origin| {
                    if method {
                        return matches!(diagnostic.code.as_str(), "PNX2322" | "PNX2801")
                            && origin.kind == ContractKind::Builtin
                            && origin.name.starts_with("clojure.string/");
                    }
                    (matches!(diagnostic.code.as_str(), "PNX2322" | "PNX2801")
                        && partial_string_operator(&origin.name))
                        || (diagnostic.code == "PNX2801"
                            && origin.kind == ContractKind::Builtin
                            && matches!(
                                origin.name.as_str(),
                                "clojure.core/nth" | "clojure.core/subvec"
                            ))
                        || (diagnostic.code == "PNX2322"
                            && origin.kind == ContractKind::Extern
                            && model.partial_extern_available(globals, origin, false))
                })
            }));
        }
        diagnostics
    }

    pub(super) fn partial_shadowed<'a>(&'a self, forms: &'a [Form]) -> BTreeSet<&'a str> {
        let mut shadowed = self.top_level_names(forms);
        shadowed.extend(self.forward_vars.keys().map(String::as_str));
        shadowed
    }
}

pub(super) fn uses_standard_partial_calls(globals: &Globals) -> bool {
    ["clojure.core", "clojure.string"]
        .iter()
        .all(|namespace| !globals.project_namespaces.contains(*namespace))
}

pub(super) fn untyped_clauses(form: &Form, method: bool) -> Option<BTreeMap<Arity, Clause<'_>>> {
    let Kind::List(items) = &form.without_metadata().kind else {
        return None;
    };
    let body = if method {
        items.get(3..)?
    } else {
        function_parts(items.get(2..)?).0
    };
    function_clauses(body, form.span).ok()
}

impl Model {
    fn partial_extern_available(
        &self,
        globals: &Globals,
        origin: &ContractOrigin,
        check_source_arities: bool,
    ) -> bool {
        let Some((namespace, _)) = origin.name.split_once('/') else {
            return false;
        };
        if namespace != self.namespace() && !self.names.loads_library(namespace) {
            return false;
        }
        if !globals.project_namespaces.contains(namespace) {
            return true;
        }
        // Source must establish an ordinary call when it is in the project.
        // An extern alone cannot make a known macro's arguments eager.
        let Some(arities) = globals
            .exports
            .get(namespace)
            .filter(|_| origin.kind == ContractKind::Extern)
            .and_then(|exports| exports.unchecked_arities(&origin.name))
        else {
            return false;
        };
        if !check_source_arities {
            return true;
        }
        let Some(ExternalDeclaration::Function(contract)) =
            globals.extern_declaration(&origin.name)
        else {
            return false;
        };
        contract.signatures.iter().all(|signature| {
            arities.iter().any(|arity| match signature.arity() {
                Arity::Fixed(count) => arity.accepts(count),
                Arity::Variadic(count) => matches!(arity, Arity::Variadic(min) if *min <= count),
            })
        })
    }
}

impl Checker<'_> {
    pub(in crate::checker::model) fn untyped_clause(&mut self, clause: &Clause<'_>) {
        let diagnostics_start = self.diagnostics.len();
        self.partial_method_opaque = false;
        let params = clause.bindings();
        let args = vec![Value::new(Type::Unknown); params.len()];
        self.function_body(
            &params,
            clause.conditions,
            clause.body,
            args,
            &Env::new(),
            clause.span,
        );
        // An opaque form may change which operations are reached or which vars are called.
        if self.partial_method_opaque {
            self.diagnostics.truncate(diagnostics_start);
            self.type_query = None;
        }
    }

    pub(in crate::checker) fn partial_contract_visible(&self, origin: &ContractOrigin) -> bool {
        let verify_arity = self.untyped_binding_budget.is_some();
        (self.partial_scope.is_none() && !verify_arity)
            || !matches!(
                origin.kind,
                ContractKind::Extern | ContractKind::ExternLiteral
            )
            || self
                .model
                .partial_extern_available(self.globals, origin, verify_arity)
    }

    pub(in crate::checker) fn allows_partial_call(&self, head: &Form, env: &Env) -> bool {
        let Some(shadowed) = self.partial_scope.as_deref() else {
            return true;
        };
        let Some(name) = head.symbol() else {
            return !self.partial_method_body && head.keyword().is_some();
        };
        if self.partial_method_body && !matches!(&head.kind, Kind::Symbol(_)) {
            return false;
        }
        if matches!(name, "if" | "do" | "quote" | "throw")
            || (!self.partial_method_body
                && (name == "."
                    || name
                        .strip_prefix('.')
                        .is_some_and(|member| STRING_MEMBERS.contains(&member))))
        {
            return true;
        }
        if env.contains_key(name) {
            return !self.partial_method_body;
        }
        let resolved = self.model.resolve(name);
        let modeled = partial_string_operator(&resolved)
            || (matches!(
                resolved.as_str(),
                "clojure.core/mapv"
                    | "clojure.core/count"
                    | "clojure.core/first"
                    | "clojure.core/nth"
                    | "clojure.core/subvec"
                    | "clojure.core/inc"
                    | "clojure.core/dec"
                    | "clojure.core/min"
                    | "clojure.core/max"
            ) && self.model.can_use_standard_core(name, self.globals))
            || matches!(
                resolved.strip_prefix("clojure.core/"),
                Some(
                    "let"
                        | "if-not"
                        | "when"
                        | "when-not"
                        | "or"
                        | "and"
                        | "get"
                        | "get-in"
                        | "contains?"
                        | "identity"
                        | "nil?"
                        | "some?"
                        | "="
                        | "str"
                        | "long"
                        | "re-pattern"
                        | "re-find"
                        | "re-matches"
                        | "some->"
                        | "some->>"
                )
            );
        if modeled && !shadowed.contains(name) {
            return true;
        }
        if self.partial_method_body {
            return false;
        }
        match self
            .model
            .declared_value(name, self.globals, head.span.start.offset)
        {
            Some(DeclaredValue::Function(contract)) => {
                self.partial_contract_visible(&contract.origin)
            }
            Some(DeclaredValue::UncheckedFunction(function)) => function.arities.is_some(),
            _ => false,
        }
    }
}

fn partial_string_operator(name: &str) -> bool {
    name == "clojure.core/subs"
        || name
            .strip_prefix("clojure.string/")
            .is_some_and(|operator| STRING_OPERATORS.contains(&operator))
        || name
            .strip_prefix("java.lang.String/")
            .is_some_and(|method| STRING_MEMBERS.contains(&method))
}
