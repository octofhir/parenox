//! Scope descriptions introduce bindings without claiming expansion semantics.
use crate::Diagnostic;
use crate::checker::model::MacroScope;
use crate::checker::{Checker, Env, Value};
use crate::reader::{Form, Span};
use crate::types::Type;

const MAX_MACRO_SCOPE_DEPTH: usize = 16;

impl Checker<'_> {
    pub(super) fn macro_scope(
        &mut self,
        scope: &MacroScope,
        args: &[Form],
        span: Span,
        env: &Env,
    ) -> Value {
        match scope {
            MacroScope::LetLike(_) => self.let_like_scope(scope, args, span, env),
        }
    }

    fn let_like_scope(
        &mut self,
        scope: &MacroScope,
        args: &[Form],
        span: Span,
        env: &Env,
    ) -> Value {
        if self.macro_scope_depth >= MAX_MACRO_SCOPE_DEPTH {
            return self.macro_gap(
                scope,
                span,
                format!("Macro scope nesting limit ({MAX_MACRO_SCOPE_DEPTH}) exceeded"),
            );
        }
        let Some((vector, body)) = args.split_first() else {
            return self.macro_gap(scope, span, "A :let-like macro requires a binding vector");
        };
        let Some(bindings) = vector.vector() else {
            return self.macro_gap(
                scope,
                vector.span,
                "A :let-like macro requires a binding vector",
            );
        };
        let (pairs, remainder) = bindings.as_chunks::<2>();
        if !remainder.is_empty() {
            return self.macro_gap(
                scope,
                vector.span,
                "A :let-like macro requires binding/value pairs",
            );
        }
        for [binding, _] in pairs {
            if binding
                .symbol()
                .is_none_or(|symbol| symbol.contains('/') || symbol == "&")
            {
                return self.macro_gap(
                    scope,
                    binding.span,
                    "A :let-like macro currently requires unqualified symbol bindings",
                );
            }
        }

        // Speculative traversal reuses the usual lexical rules, but must not
        // affect the caller's effects, recur targets or inference budgets.
        let mut lexical = Checker::new(self.model, self.globals, self.source);
        lexical.macro_scope_depth = self.macro_scope_depth + 1;
        let mut local = env.clone();
        for [binding, init] in pairs {
            lexical.infer_path(init, &local, false, false);
            local.insert(binding.symbol().unwrap().into(), Value::new(Type::Unknown));
        }
        for form in body {
            lexical.infer_path(form, &local, false, false);
        }
        self.diagnostics.extend(
            lexical
                .diagnostics
                .into_iter()
                .filter_map(|mut diagnostic| {
                    if !matches!(diagnostic.code.as_str(), "PNX2304" | "PNX9009") {
                        return None;
                    }
                    diagnostic
                        .contract
                        .get_or_insert_with(|| scope.origin().clone());
                    Some(diagnostic)
                }),
        );
        self.macro_gap(
            scope,
            span,
            format!(
                "Only lexical scope was checked for macro {}; result and effects are unknown",
                scope.origin().name
            ),
        )
    }

    pub(in crate::checker) fn macro_gap(
        &mut self,
        scope: &MacroScope,
        span: Span,
        message: impl Into<String>,
    ) -> Value {
        let mut diagnostic = Diagnostic::new("PNX9009", span, message);
        diagnostic.contract = Some(scope.origin().clone());
        self.diagnostics.push(diagnostic);
        self.may_throw();
        Value::new(Type::Unknown)
    }
}
