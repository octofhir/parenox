//! Control flow, type narrowing and core threading forms.
use super::calls::predicate_type;
use super::{Checker, Env, Value, nil_part};
use crate::expand::{self, Threading};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

impl Checker<'_> {
    pub(super) fn thread_form(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        threading: Threading,
        name: &str,
    ) -> Value {
        if xs.len() < 2 {
            self.error(
                "PNX2554",
                span,
                format!("{name} requires an initial expression"),
            );
            return Value::new(Type::Unknown);
        }
        if threading.is_conditional() && !(xs.len() - 2).is_multiple_of(2) {
            self.error(
                "PNX2554",
                span,
                format!("{name} requires condition/step pairs"),
            );
            return Value::new(Type::Unknown);
        }
        self.with_thread_expansion(span, |checker| {
            if threading.is_conditional() {
                checker.conditional_steps(xs, env, threading, name, span)
            } else {
                checker.thread_steps(xs, env, threading, name, span)
            }
        })
    }

    fn with_thread_expansion(
        &mut self,
        span: Span,
        expand: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        if self.thread_depth >= 32 {
            return self.unsupported(span, "Threading expansion nesting limit (32) exceeded");
        }
        self.thread_depth += 1;
        let result = expand(self);
        self.thread_depth -= 1;
        result
    }

    fn infer_path(&mut self, form: &Form, env: &Env, reachable: bool) -> Value {
        self.check_path(reachable, |checker| checker.infer(form, env))
    }

    pub(super) fn check_path(
        &mut self,
        reachable: bool,
        check: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        let start = self.diagnostics.len();
        let mut value = check(self);
        if !reachable {
            // Unexecuted forms must still resolve and pass compiler-level checks.
            let errors = self.diagnostics.split_off(start);
            self.diagnostics.extend(
                errors
                    .into_iter()
                    .filter(|d| !matches!(d.code.as_str(), "PNX2322" | "PNX2741" | "PNX2801")),
            );
            value.narrow(Type::Never);
        }
        value
    }

    fn expansion_origin(&mut self, start: usize, name: &str, span: Span, transformed: bool) {
        for diagnostic in &mut self.diagnostics[start..] {
            // Synthetic argument spans can point at bare steps. Replacing such a
            // fragment may change threading semantics; original as-> forms are editable.
            if transformed && !diagnostic.suggestions.is_empty() {
                diagnostic.suggestions.clear();
                diagnostic.help = Some("Handle the expected type explicitly in the threading expression before this call.".into());
            }
            diagnostic.notes.push(format!(
                "While expanding {name} at {}:{}",
                span.start.line, span.start.column
            ));
        }
    }

    fn thread_steps(
        &mut self,
        xs: &[Form],
        env: &Env,
        threading: Threading,
        name: &str,
        span: Span,
    ) -> Value {
        let mut current = self.infer(&xs[1], env);
        if xs.len() == 2 {
            return current;
        }
        let temporary = expand::temporary(&xs[1..], |name| env.contains_key(name));
        let mut local = env.clone();
        local.insert(temporary.clone(), Value::new(Type::Never));
        let mut origin = xs[1].span;
        let mut skipped_nil = false;
        for step in &xs[2..] {
            if threading.guards_nil() {
                skipped_nil |= nil_part(&current.known) == Type::Nil;
                current.narrow(current.known.without_nil());
            }
            let reachable = current.known != Type::Never;
            *local.get_mut(&temporary).unwrap() = current;
            let call = threading.apply(
                step,
                Form {
                    kind: Kind::Symbol(temporary.clone()),
                    span: origin,
                },
            );
            let start = self.diagnostics.len();
            current = self.infer_path(&call, &local, reachable);
            self.expansion_origin(start, name, span, true);
            origin = step.span;
        }
        if skipped_nil {
            let known = std::mem::replace(&mut current.known, Type::Never);
            current.narrow(known.nilable());
            current.notes.push(format!("{name} returns Nil when its input or an intermediate result is Nil; False continues through the chain."));
        }
        current
    }

    fn conditional_steps(
        &mut self,
        xs: &[Form],
        env: &Env,
        threading: Threading,
        name: &str,
        span: Span,
    ) -> Value {
        let mut current = self.infer(&xs[1], env);
        if xs.len() == 2 {
            return current;
        }
        let temporary = expand::temporary(&xs[1..], |name| env.contains_key(name));
        let mut origin = xs[1].span;
        let mut aliases_seed = true;
        let mut joined_branches = false;
        for [test, step] in xs[2..].as_chunks::<2>().0 {
            let start = self.diagnostics.len();
            let condition = self.infer_path(test, env, current.known != Type::Never);
            self.expansion_origin(start, name, span, false);
            let yes_reachable = condition.known.truthy() != Type::Never;
            let no_reachable = condition.known.falsey() != Type::Never;
            joined_branches |= yes_reachable && no_reachable;
            let mut yes_env = env.clone();
            self.narrow(test, true, &mut yes_env);
            let mut skipped = if no_reachable {
                current.clone()
            } else {
                Value::new(Type::Never)
            };
            // Before a potentially executed transformation, the temporary still
            // denotes the original seed local, so its guard also narrows the input.
            if aliases_seed && let Some(seed) = xs[1].symbol() {
                if let Some(value) = yes_env.get(seed) {
                    current = value.clone();
                }
                if no_reachable {
                    let mut no_env = env.clone();
                    self.narrow(test, false, &mut no_env);
                    if let Some(value) = no_env.get(seed) {
                        skipped = value.clone();
                    }
                }
            }
            yes_env.insert(temporary.clone(), current);
            let call = threading.apply(
                step,
                Form {
                    kind: Kind::Symbol(temporary.clone()),
                    span: origin,
                },
            );
            let start = self.diagnostics.len();
            let applied = self.infer_path(&call, &yes_env, yes_reachable);
            self.expansion_origin(start, name, span, true);
            current = applied.join(skipped);
            if yes_reachable {
                aliases_seed = false;
                origin = step.span;
            }
        }
        if joined_branches {
            current.notes.push(format!("{name} keeps the previous value when a condition is false; its result includes both paths."));
        }
        current
    }

    pub(super) fn named_thread(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if xs.len() < 3 {
            self.error(
                "PNX2554",
                span,
                "clojure.core/as-> requires an expression and binding name",
            );
            return Value::new(Type::Unknown);
        }
        let Some(binding) = xs[2].symbol() else {
            if matches!(
                xs[2].without_metadata().kind,
                Kind::Map(_) | Kind::Vector(_)
            ) {
                return self.unsupported(
                    xs[2].span,
                    "as-> destructuring bindings are not supported yet",
                );
            }
            self.error("PNX1001", xs[2].span, "as-> requires a local binding name");
            return Value::new(Type::Unknown);
        };
        if binding == "&" || binding.contains('/') {
            self.error(
                "PNX1001",
                xs[2].span,
                "as-> requires an unqualified local binding name",
            );
            return Value::new(Type::Unknown);
        }
        self.with_thread_expansion(span, |checker| {
            let mut current = checker.infer(&xs[1], env);
            if xs.len() == 3 {
                return current;
            }
            let mut local = env.clone();
            local.insert(binding.to_owned(), Value::new(Type::Never));
            for step in &xs[3..] {
                let reachable = current.known != Type::Never;
                *local.get_mut(binding).unwrap() = current;
                let start = checker.diagnostics.len();
                current = checker.infer_path(step, &local, reachable);
                checker.expansion_origin(start, "clojure.core/as->", span, false);
            }
            current
        })
    }

    pub(super) fn let_form(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        let Some(bindings) = xs.get(1).and_then(Form::vector) else {
            return self.unsupported(span, "let requires a binding vector");
        };
        if bindings.len() % 2 != 0 {
            return self.unsupported(xs[1].span, "let requires binding/value pairs");
        }
        let mut local = env.clone();
        for pair in bindings.as_chunks::<2>().0 {
            let value = self.infer(&pair[1], &local);
            self.bind(&pair[0], value, &mut local);
        }
        self.body(&xs[2..], &local)
    }

    pub(super) fn if_form(&mut self, xs: &[Form], span: Span, env: &Env, when: bool) -> Value {
        if xs.len() < 3 || (!when && xs.len() > 4) {
            return self.unsupported(span, "Invalid conditional arity");
        }
        let condition = self.infer(&xs[1], env);
        let mut yes = {
            let mut local = env.clone();
            self.narrow(&xs[1], true, &mut local);
            if when {
                self.body(&xs[2..], &local)
            } else {
                self.infer(&xs[2], &local)
            }
        };
        let mut no = {
            let mut local = env.clone();
            self.narrow(&xs[1], false, &mut local);
            if when {
                Value::new(Type::Nil)
            } else {
                xs.get(3)
                    .map_or_else(|| Value::new(Type::Nil), |x| self.infer(x, &local))
            }
        };
        // Even unreachable forms must be resolved: Clojure compiles both branches.
        if condition.ty.truthy() == Type::Never {
            yes.narrow(Type::Never);
        }
        if condition.ty.falsey() == Type::Never {
            no.narrow(Type::Never);
        }
        Value::with_notes(
            Type::union([yes.known, no.known]),
            [yes.notes, no.notes].concat(),
        )
    }

    pub(super) fn binding_conditional(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        name: &str,
    ) -> Value {
        let when = name.starts_with("clojure.core/when-");
        if xs.len() < 2 || (!when && !(3..=4).contains(&xs.len())) {
            self.error("PNX2554", span, format!("Invalid arity for {name}"));
            return Value::new(Type::Unknown);
        }
        let Some([pattern, init]) = xs[1].vector() else {
            self.error(
                "PNX1001",
                xs[1].span,
                format!("{name} requires a binding vector with exactly two forms"),
            );
            return Value::new(Type::Unknown);
        };
        let valid_pattern = match &pattern.without_metadata().kind {
            Kind::Symbol(symbol) => symbol != "&" && !symbol.contains('/'),
            Kind::Map(_) | Kind::Vector(_) => true,
            _ => false,
        };
        if !valid_pattern {
            self.error(
                "PNX1001",
                pattern.span,
                format!("Invalid binding form for {name}"),
            );
            return Value::new(Type::Unknown);
        }

        let mut input = self.infer(init, env);
        let non_nil = name.ends_with("-some");
        let (matched, skipped) = if non_nil {
            (input.known.without_nil(), nil_part(&input.known))
        } else {
            (input.known.truthy(), input.known.falsey())
        };
        let yes_reachable = matched != Type::Never;
        let no_reachable = skipped != Type::Never;
        let narrow = |truth, ty, scope: &mut Env| {
            if non_nil {
                if let Some(value) = init.symbol().and_then(|symbol| scope.get_mut(symbol)) {
                    value.narrow(ty);
                }
            } else {
                self.narrow(init, truth, scope);
            }
        };
        let mut yes_env = env.clone();
        narrow(true, matched.clone(), &mut yes_env);
        let no_env = if !when && xs.len() == 4 {
            let mut scope = env.clone();
            narrow(false, skipped, &mut scope);
            Some(scope)
        } else {
            None
        };

        input.narrow(matched);
        let yes = self.check_path(yes_reachable, |checker| {
            checker.bind(pattern, input, &mut yes_env);
            checker.body(if when { &xs[2..] } else { &xs[2..3] }, &yes_env)
        });
        let no = if let Some(scope) = no_env {
            self.infer_path(&xs[3], &scope, no_reachable)
        } else if no_reachable {
            Value::new(Type::Nil)
        } else {
            Value::new(Type::Never)
        };
        Value::with_notes(
            Type::union([yes.known, no.known]),
            [yes.notes, no.notes].concat(),
        )
    }

    pub(super) fn short_circuit(&mut self, xs: &[Form], env: &Env, or: bool) -> Value {
        if xs.len() == 1 {
            return Value::new(if or { Type::Nil } else { Type::Bool(true) });
        }
        let mut local = env.clone();
        let mut types = Vec::new();
        let mut notes = Vec::new();
        let mut reachable = true;
        for (index, x) in xs[1..].iter().enumerate() {
            let value = self.infer(x, &local);
            if !reachable {
                continue;
            }
            notes.extend(value.notes);
            if index == xs.len() - 2 {
                types.push(value.known);
                break;
            }
            types.push(if or {
                value.known.truthy()
            } else {
                value.known.falsey()
            });
            if (if or {
                value.ty.falsey()
            } else {
                value.ty.truthy()
            }) == Type::Never
            {
                reachable = false;
            }
            self.narrow(x, !or, &mut local);
        }
        Value::with_notes(Type::union(types), notes)
    }

    fn narrow(&self, test: &Form, truth: bool, env: &mut Env) {
        if let Some(name) = test.symbol() {
            if let Some(value) = env.get_mut(name) {
                value.narrow(if truth {
                    value.known.truthy()
                } else {
                    value.known.falsey()
                });
            }
            return;
        }
        let Kind::List(xs) = &test.without_metadata().kind else {
            return;
        };
        if xs.len() < 2 {
            return;
        }
        let Some(op) = xs[0].symbol() else { return };
        // Locals and project functions may shadow core predicates.
        if env.contains_key(op)
            || self.model.local_function(op).is_some()
            || self.model.signature(op, self.globals).is_some()
        {
            return;
        }
        let op = self.model.resolve(op);
        if op == "clojure.core/contains?" {
            self.narrow_contains(&xs[1..], truth, env);
            return;
        }
        if xs.len() != 2 {
            return;
        }
        if op == "clojure.core/not" {
            self.narrow(&xs[1], !truth, env);
            return;
        }
        let Some(name) = xs[1].symbol() else { return };
        let Some(value) = env.get_mut(name) else {
            return;
        };
        let positive = match op.as_str() {
            "clojure.core/nil?" => Some(Type::Nil),
            "clojure.core/some?" => {
                value.narrow(if truth {
                    value.known.without_nil()
                } else {
                    nil_part(&value.known)
                });
                return;
            }
            _ => predicate_type(&op),
        };
        if let Some(wanted) = positive {
            value.narrow(narrow_by_type(&value.known, &wanted, truth));
        }
    }
}

pub(super) fn narrow_by_type(actual: &Type, wanted: &Type, truth: bool) -> Type {
    match actual {
        Type::Union(parts) => {
            Type::union(parts.iter().map(|part| narrow_by_type(part, wanted, truth)))
        }
        actual if actual.subtype_of(wanted) => {
            if truth {
                actual.clone()
            } else {
                Type::Never
            }
        }
        actual if *actual == Type::Unknown || wanted.subtype_of(actual) => {
            if truth {
                wanted.clone()
            } else {
                actual.clone()
            }
        }
        actual => {
            if truth {
                Type::Never
            } else {
                actual.clone()
            }
        }
    }
}
