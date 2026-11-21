//! Static operand checks for standard clojure.test assertions.
use std::collections::BTreeSet;

use super::{
    Declarations, Globals, Model,
    declarations::{static_namespace_prelude, symbols_match},
    require::known_core_var,
};
use crate::Diagnostic;
use crate::checker::{
    Checker, Env,
    calls::{call_returns, is_modeled_call},
};
use crate::reader::{Form, Kind};

impl Declarations {
    pub(crate) fn check_test_bodies(
        &self,
        forms: &[Form],
        model: &Model,
        globals: &Globals,
        source: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> BTreeSet<usize> {
        let mut checked = BTreeSet::new();
        if self.test_forms.is_empty()
            || !forms.first().is_some_and(static_namespace_prelude)
            || self.all_refers.iter().any(|name| name != "clojure.test")
        {
            return checked;
        }
        let shadowed = self.top_level_names(forms);
        let mut test_names = BTreeSet::new();
        let mut repeated = BTreeSet::new();
        for &index in &self.test_forms {
            let Kind::List(items) = &forms[index].without_metadata().kind else {
                continue;
            };
            if let Some(name) = items.get(1).and_then(Form::symbol)
                && !test_names.insert(name)
            {
                repeated.insert(name);
            }
        }
        let opaque_scope = forms.iter().enumerate().find_map(|(index, form)| {
            (self.test_forms.binary_search(&index).is_err()
                && !symbols_match(form, &mut |name| {
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
                                | "eval"
                                | "intern"
                                | "alter-var-root"
                        )
                    )
                }))
            .then_some(form.span.start.offset)
        });
        let reference_findings = diagnostics.len();
        for (form, name, body) in self.test_declarations(forms) {
            let Kind::List(items) = &form.kind else {
                continue;
            };
            let head = &items[0];
            let Kind::Symbol(symbol) = &name.kind else {
                continue;
            };
            if symbol.contains('/')
                || opaque_scope.is_some_and(|offset| offset < form.span.start.offset)
                || repeated.contains(symbol.as_str())
                || self.forward_vars.contains_key(symbol)
                || self.names.refers.contains_key(symbol)
                || self.names.imports.contains_key(symbol)
                || self.source_declarations(forms, symbol).next().is_some()
                || model
                    .declared_value(symbol, globals, form.span.start.offset)
                    .is_some()
                || known_core_var(symbol)
                || is_modeled_call(&format!("clojure.core/{symbol}"))
                || matches!(
                    symbol.as_str(),
                    "declare"
                        | "defmacro"
                        | "defonce"
                        | "defmulti"
                        | "defmethod"
                        | "defrecord"
                        | "deftype"
                        | "definterface"
                        | "defprotocol"
                        | "extend"
                        | "extend-type"
                        | "extend-protocol"
                        | "comment"
                )
                || trusted_name(model, &shadowed, head) != Some(("clojure.test", "deftest"))
            {
                continue;
            }
            let mut checker = Checker::new(model, globals, source);
            let env = Env::new();
            let covered = checker.check_assertions(body, &env, &shadowed)
                && !checker.has_unproven_expectation
                && !checker
                    .diagnostics
                    .iter()
                    .any(|item| item.code.starts_with("PNX9"));
            if covered {
                checked.insert(head.span.start.offset);
            }
            for item in checker.diagnostics {
                if (covered || matches!(item.code.as_str(), "PNX2322" | "PNX2801"))
                    && !diagnostics[..reference_findings]
                        .iter()
                        .any(|prior| prior.code == item.code && prior.span == item.span)
                {
                    diagnostics.push(item);
                }
            }
        }
        checked
    }
}

impl Checker<'_> {
    fn check_assertions(&mut self, body: &[Form], env: &Env, shadowed: &BTreeSet<&str>) -> bool {
        let mut covered = !body.is_empty();
        for form in body {
            let Kind::List(items) = &form.kind else {
                covered = false;
                continue;
            };
            let [head, operand, rest @ ..] = items.as_slice() else {
                covered = false;
                continue;
            };
            covered &= match trusted_name(self.model, shadowed, head) {
                Some(("clojure.test", "testing")) if operand.is_string() => {
                    self.check_assertions(rest, env, shadowed)
                }
                Some(("clojure.test", "is"))
                    if rest.is_empty() || matches!(rest, [message] if message.is_string()) =>
                {
                    self.check_assertion(operand, env, shadowed)
                }
                _ => false,
            };
        }
        covered
    }

    fn check_assertion(&mut self, form: &Form, env: &Env, shadowed: &BTreeSet<&str>) -> bool {
        let items = match &form.kind {
            Kind::Nil
            | Kind::Bool(_)
            | Kind::Number
            | Kind::SymbolicNumber(_)
            | Kind::Character(_)
            | Kind::String(_)
            | Kind::Utf16String(_) => return true,
            Kind::Keyword(key) => {
                return !key.starts_with("::") || self.model.validation_key(key).is_some();
            }
            Kind::List(items) => items,
            _ => return false,
        };
        let Some((head, args)) = items.split_first() else {
            return false;
        };
        if !matches!(
            trusted_name(self.model, shadowed, head),
            Some((
                "clojure.core",
                "=" | "nil?" | "some?" | "true?" | "false?" | "not"
            ))
        ) {
            return false;
        }
        // Standard assert-expr evaluates predicate arguments once, then applies
        // the predicate. Its truth value belongs to the test runner.
        let callable = self.prepare_callable(head, env);
        let args: Vec<_> = self.eval_args(args, env, callable.returns()).collect();
        let covered = args
            .iter()
            .all(|(value, _)| value.rejected_call || !value.known.contains_unknown());
        let invoked = call_returns(&args);
        self.call_prepared(&callable, args, form.span, env, invoked);
        covered
    }
}

fn trusted_name<'a>(
    model: &'a Model,
    shadowed: &BTreeSet<&str>,
    form: &'a Form,
) -> Option<(&'a str, &'a str)> {
    let Kind::Symbol(name) = &form.kind else {
        return None;
    };
    (model.runtime_alias_gap(name).is_none()
        && (name.contains('/') || !shadowed.contains(name.as_str())))
    .then(|| model.names.resolve_parts(name))
}
