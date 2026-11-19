//! Type findings from selected built-in calls in bodies that have no contract.
use super::contracts::function_parts;
use super::{Declarations, DeclaredValue, Globals, Model};
use crate::Diagnostic;
use crate::checker::{Checker, Env, Value, function_forms::function_clauses};
use crate::reader::{Form, Kind};
use crate::types::Type;

const STRING_OPERATORS: &[&str] = &[
    "upper-case",
    "lower-case",
    "trim",
    "blank?",
    "join",
    "replace",
    "split",
];
const STRING_MEMBERS: &[&str] = &["charAt", "length", "startsWith", "endsWith", "indexOf"];

impl Declarations {
    pub(crate) fn check_untyped_strings(
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
        if (!string_namespace && !source.contains("subs") && !jvm_string)
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
            if !operators.iter().any(|&operator| text.contains(operator)) {
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
                checker.function_body(&params, clause.body, args, &Env::new(), clause.span);
            }
            diagnostics.extend(checker.diagnostics.into_iter().filter(|diagnostic| {
                matches!(diagnostic.code.as_str(), "PNX2322" | "PNX2801")
                    && diagnostic
                        .contract
                        .as_ref()
                        .is_some_and(|origin| partial_string_operator(&origin.name))
            }));
        }
        diagnostics
    }
}

impl Checker<'_> {
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
        match self.model.declared_value(name, self.globals) {
            Some(DeclaredValue::Function(_)) => true,
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
