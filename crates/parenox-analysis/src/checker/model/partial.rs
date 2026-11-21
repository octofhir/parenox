//! Definite call-site findings in function bodies without a checked signature.
use super::contracts::function_parts;
use super::declarations::symbols_match;
use super::externs::ExternalDeclaration;
use super::{Declarations, DeclaredValue, Globals, Model};
use crate::checker::{Checker, Env, Value, function_forms::function_clauses};
use crate::reader::{Form, Kind};
use crate::types::Type;
use crate::{ContractKind, ContractOrigin, Diagnostic};

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
            || ["clojure.core", "clojure.string"]
                .iter()
                .any(|namespace| globals.project_namespaces.contains(*namespace))
        {
            return Vec::new();
        }
        let mut definitions = self.unchecked_definitions(forms, model).peekable();
        if definitions.peek().is_none() || !self.has_known_var_surface(forms) {
            return Vec::new();
        }
        let shadowed = self.top_level_names(forms);
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
        for form in definitions {
            let text = &source[form.span.start.offset..form.span.end.offset];
            let has_string_call = operators.iter().any(|&operator| text.contains(operator));
            let has_extern_call = extern_calls
                && !has_string_call
                && !symbols_match(form, &mut |name| {
                    !matches!(model.declared_value(name, globals, form.span.start.offset),
                        Some(DeclaredValue::Function(contract))
                            if contract.origin.kind == ContractKind::Extern
                                && model.partial_extern_available(globals, &contract.origin))
                });
            if !has_string_call && !has_extern_call {
                continue;
            }
            let Kind::List(items) = &form.without_metadata().kind else {
                continue;
            };
            let Ok(clauses) = function_clauses(function_parts(&items[2..]).0, form.span) else {
                continue;
            };
            let mut checker = Checker::new(model, globals, source);
            checker.partial_scope = Some(&shadowed);
            for clause in clauses.values() {
                let params = clause.bindings();
                let args = vec![Value::new(Type::Unknown); params.len()];
                checker.function_body(
                    &params,
                    clause.conditions,
                    clause.body,
                    args,
                    &Env::new(),
                    clause.span,
                );
            }
            diagnostics.extend(checker.diagnostics.into_iter().filter(|diagnostic| {
                diagnostic.contract.as_ref().is_some_and(|origin| {
                    (matches!(diagnostic.code.as_str(), "PNX2322" | "PNX2801")
                        && partial_string_operator(&origin.name))
                        || (diagnostic.code == "PNX2322"
                            && origin.kind == ContractKind::Extern
                            && model.partial_extern_available(globals, origin))
                })
            }));
        }
        diagnostics
    }
}

impl Model {
    fn partial_extern_available(&self, globals: &Globals, origin: &ContractOrigin) -> bool {
        let Some((namespace, _)) = origin.name.split_once('/') else {
            return false;
        };
        let loaded =
            namespace == self.namespace() || self.names.required_libraries.contains(namespace);
        // Source must establish an ordinary call when it is in the project.
        // An extern alone cannot make a known macro's arguments eager.
        loaded
            && (!globals.project_namespaces.contains(namespace)
                || (origin.kind == ContractKind::Extern
                    && globals
                        .exports
                        .get(namespace)
                        .is_some_and(|exports| exports.has_unchecked_function(&origin.name))))
    }
}

impl Checker<'_> {
    pub(in crate::checker) fn partial_contract_visible(&self, origin: &ContractOrigin) -> bool {
        self.partial_scope.is_none()
            || !matches!(
                origin.kind,
                ContractKind::Extern | ContractKind::ExternLiteral
            )
            || self.model.partial_extern_available(self.globals, origin)
    }

    pub(in crate::checker) fn allows_partial_call(&self, head: &Form, env: &Env) -> bool {
        let Some(shadowed) = self.partial_scope else {
            return true;
        };
        let Some(name) = head.symbol() else {
            return head.keyword().is_some();
        };
        if matches!(name, "if" | "do" | "quote" | ".")
            || name
                .strip_prefix('.')
                .is_some_and(|member| STRING_MEMBERS.contains(&member))
            || env.contains_key(name)
        {
            return true;
        }
        let resolved = self.model.resolve(name);
        let modeled = partial_string_operator(&resolved)
            || (resolved == "clojure.core/mapv"
                && self.model.can_use_standard_core(name, self.globals))
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
