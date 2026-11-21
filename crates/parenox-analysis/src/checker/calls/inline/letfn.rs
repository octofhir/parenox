//! Lexical groups of named local functions.
use std::sync::Arc;

use crate::checker::{
    Checker, Env, Value,
    bindings::dotted_binding_error,
    env::{CapturedInline, LetFnGroup},
    function_forms::{letfn_bindings, letfn_definition},
};
use crate::reader::{Form, Span};
use crate::types::Type;

impl LetFnGroup {
    pub(super) fn bind_to(self: &Arc<Self>, env: &mut Env) {
        for (index, (name, _)) in self.definitions.iter().enumerate() {
            env.mark_inline(
                name,
                CapturedInline::Group {
                    group: self.clone(),
                    index,
                },
            );
        }
    }
}

impl Checker<'_> {
    pub(in crate::checker::calls) fn letfn_form(
        &mut self,
        forms: &[Form],
        span: Span,
        env: &Env,
        tail: bool,
    ) -> Value {
        let [bindings, body @ ..] = forms else {
            self.error("PNX2554", span, "letfn requires binding definitions");
            return Value::new(Type::Unknown);
        };
        let Some(specs) = letfn_bindings(bindings) else {
            return self.unchecked_form(
                bindings.span,
                "letfn analysis supports binding definitions in lists and vectors",
            );
        };
        if specs.is_empty() {
            return self.body(body, env, tail);
        }

        let mut local = env.clone();
        let mut definitions = Vec::with_capacity(specs.len());
        for spec in specs {
            let Some((name, _)) = letfn_definition(spec).and_then(|parts| parts.split_first())
            else {
                self.error("PNX1001", spec.span, "Expected a named letfn definition");
                return Value::new(Type::Unknown);
            };
            let Some(symbol) = name
                .symbol()
                .filter(|symbol| !symbol.contains('/') || *symbol == "/")
            else {
                self.error(
                    "PNX1001",
                    name.without_metadata().span,
                    "letfn requires an unqualified local name",
                );
                return Value::new(Type::Unknown);
            };
            if let Some(diagnostic) = dotted_binding_error(symbol, name.without_metadata().span) {
                self.diagnostics.push(diagnostic);
                return Value::new(Type::Unknown);
            }
            local.insert(symbol.to_owned(), Value::new(Type::Unknown));
            definitions.push((symbol.to_owned(), spec.clone()));
        }

        // Capture the binding IDs before attaching the group to its locals.
        // Each call reinstates those links without storing a reference cycle.
        let group = Arc::new(LetFnGroup {
            scope: local.clone(),
            definitions: definitions.into_boxed_slice(),
        });
        group.bind_to(&mut local);
        let start = self.diagnostics.len();
        for (index, (_, spec)) in group.definitions.iter().enumerate() {
            if !self.local_inline_step(spec.span) {
                break;
            }
            let captured = CapturedInline::Group {
                group: group.clone(),
                index,
            };
            let (_, function) = captured.syntax();
            let mut scope = local.clone();
            function.bind_name(&captured, &mut scope);
            self.inspect_path(false, |checker| {
                checker.call_inline(function, Vec::new(), spec.span, spec.span, &scope, false)
            });
        }
        let result = self.body(body, &local, tail);
        self.dedup_diagnostics(start);
        result
    }
}
