//! Definite call-site findings in function bodies without a checked signature.
use super::contracts::function_parts;
use super::declarations::symbols_match;
use super::externs::ExternalDeclaration;
use super::{Declarations, DeclaredValue, Globals, Model};
use crate::checker::calls::is_modeled_string_member;
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
const INDEX_OPERATORS: &[&str] = &["nth", "subvec"];

impl Declarations {
    pub(crate) fn check_untyped_calls(
        &self,
        forms: &[Form],
        model: &Model,
        globals: &Globals,
        source: &str,
    ) -> Vec<Diagnostic> {
        if !self.all_refers.is_empty() || !uses_standard_partial_calls(globals) {
            return Vec::new();
        }
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
        let mut definitions = self
            .unchecked_definitions(forms, model)
            .filter(|form| model.partial_check_candidate(form, globals, false, extern_calls))
            .peekable();
        let mut methods = self
            .method_forms
            .iter()
            .filter(|_| self.static_method_scope)
            .map(|&index| &forms[index])
            .filter(|form| model.partial_check_candidate(form, globals, true, false))
            .peekable();
        let functions = definitions.peek().is_some() && self.has_known_var_surface(forms);
        if !functions && methods.peek().is_none() {
            return Vec::new();
        }
        let shadowed = if self.static_method_scope {
            self.partial_shadowed(forms)
        } else {
            self.top_level_names(forms)
        };
        let definitions = definitions.filter(|_| functions);
        let mut diagnostics = Vec::new();
        for (form, method) in definitions
            .map(|form| (form, false))
            .chain(methods.map(|form| (form, true)))
        {
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
                            && origin
                                .name
                                .strip_prefix("clojure.core/")
                                .is_some_and(|name| INDEX_OPERATORS.contains(&name)))
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
    fn partial_check_candidate(
        &self,
        form: &Form,
        globals: &Globals,
        method: bool,
        extern_calls: bool,
    ) -> bool {
        // This pass selects bodies to check. The checker still resolves
        // lexical bindings and proves the operand types within each body.
        !symbols_match(form, &mut |name| {
            let (namespace, local) = self.names.resolve_parts(name);
            let builtin = if method {
                namespace == "clojure.string" && STRING_OPERATORS.contains(&local)
            } else {
                partial_string_operator_parts(namespace, local)
                    || (namespace == "clojure.core" && INDEX_OPERATORS.contains(&local))
                    // Bare member names can occur in an explicit dot form.
                    || is_modeled_string_member(name.strip_prefix('.').unwrap_or(name))
            };
            !builtin
                && !(extern_calls
                    && matches!(self.declared_value(name, globals, form.span.start.offset),
                        Some(DeclaredValue::Function(contract))
                            if contract.origin.kind == ContractKind::Extern
                                && self.partial_extern_available(globals, &contract.origin, false)))
        })
    }

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
        let args = vec![Value::new(Type::Unknown); clause.parameters().count()];
        self.function_body(clause, args, &Env::new(), None);
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
        if matches!(name, "if" | "do" | "quote" | "throw" | "let*" | "fn*")
            || (!self.partial_method_body
                && (name == "." || name.strip_prefix('.').is_some_and(is_modeled_string_member)))
        {
            return true;
        }
        if env.contains_key(name) {
            return !self.partial_method_body;
        }
        let (namespace, local) = self.model.names.resolve_parts(name);
        let modeled = partial_string_operator_parts(namespace, local)
            || (namespace == "clojure.core"
                && ((self.model.can_use_standard_core(name, self.globals)
                    && (INDEX_OPERATORS.contains(&local)
                        || matches!(
                            local,
                            "mapv" | "count" | "first" | "inc" | "dec" | "min" | "max"
                        )))
                    || matches!(
                        local,
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
                    )));
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
    name.split_once('/')
        .is_some_and(|(namespace, local)| partial_string_operator_parts(namespace, local))
}

fn partial_string_operator_parts(namespace: &str, local: &str) -> bool {
    match namespace {
        "clojure.core" => local == "subs",
        "clojure.string" => STRING_OPERATORS.contains(&local),
        "java.lang.String" => is_modeled_string_member(local),
        _ => false,
    }
}
