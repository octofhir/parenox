//! Control flow, type narrowing and core threading forms.
mod selectors;
mod spec_guards;

use std::collections::BTreeSet;

pub(super) use selectors::{narrow_literal, refine_owner};

use super::bindings::{dotted_binding_error, raw_local_symbol};
use super::calls::{is_builtin, narrow_emptiness, narrow_vector_predicate, predicate_type};
use super::maps::narrow_map_predicate;
use super::model::DeclaredValue;
use super::recur::bounded_state;
use super::{Checker, Env, Value, has_evaluated_metadata, nil_part};
use crate::expand::{self, Threading};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

const MAX_THREAD_EXPANSION_DEPTH: usize = 32;

fn condp_scalar(ty: &Type) -> bool {
    match ty {
        Type::Union(parts) => parts.iter().all(condp_scalar),
        Type::Never
        | Type::Nil
        | Type::Boolean
        | Type::Bool(_)
        | Type::String
        | Type::Str(_)
        | Type::Character
        | Type::Char(_)
        | Type::Keyword
        | Type::SpecKeyword
        | Type::Key(_)
        | Type::Symbol
        | Type::SimpleSymbol
        | Type::QualifiedSymbol
        | Type::Sym(_) => true,
        _ => false,
    }
}

#[derive(Clone, Copy)]
pub(super) enum Conditional {
    If,
    When,
    IfNot,
    WhenNot,
}

impl Conditional {
    pub(super) fn runs_on_truth(self) -> bool {
        matches!(self, Self::If | Self::When)
    }

    pub(super) fn takes_else(self) -> bool {
        matches!(self, Self::If | Self::IfNot)
    }

    fn name(self) -> &'static str {
        match self {
            Self::If => "if",
            Self::When => "when",
            Self::IfNot => "if-not",
            Self::WhenNot => "when-not",
        }
    }
}

struct BindingPaths<'a> {
    body: &'a [Form],
    bound_scope: Env,
    alternate: Option<(&'a [Form], Env)>,
    skips_binding: bool,
}

impl Checker<'_> {
    pub(super) fn thread_form(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        threading: Threading,
        name: &str,
        tail: bool,
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
        if matches!(threading, Threading::First | Threading::Last) {
            let steps = &xs[2..];
            if steps.len() <= MAX_THREAD_EXPANSION_DEPTH {
                return self.with_thread_expansion(span, steps.len().max(1), |checker| {
                    let start = checker.diagnostics.len();
                    let expanded = steps
                        .iter()
                        .fold(xs[1].clone(), |input, step| threading.apply(step, input));
                    let result = checker.infer_position(&expanded, env, tail);
                    if !steps.is_empty() {
                        checker.expansion_origin(start, name, span, true);
                    }
                    result
                });
            }
            // The checker can stream a long chain of bare built-in calls whose input
            // is their only argument. Other steps may suppress or reorder it.
            if !steps.iter().all(|step| {
                step.keyword().is_some()
                    || step
                        .symbol()
                        .is_some_and(|name| is_builtin(&self.model.resolve(name)))
            }) {
                return self.thread_expansion_limit(span);
            }
        }
        self.with_thread_expansion(span, 1, |checker| {
            if threading.is_conditional() {
                checker.conditional_steps(xs, env, threading, name, span, tail)
            } else {
                checker.thread_steps(xs, env, threading, name, span, tail)
            }
        })
    }

    fn with_thread_expansion(
        &mut self,
        span: Span,
        depth: usize,
        expand: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        if self.thread_depth + depth > MAX_THREAD_EXPANSION_DEPTH {
            return self.thread_expansion_limit(span);
        }
        self.thread_depth += depth;
        let result = expand(self);
        self.thread_depth -= depth;
        result
    }

    fn thread_expansion_limit(&mut self, span: Span) -> Value {
        self.unsupported(
            span,
            format!("Threading expansion nesting limit ({MAX_THREAD_EXPANSION_DEPTH}) exceeded"),
        )
    }

    pub(super) fn infer_path(
        &mut self,
        form: &Form,
        env: &Env,
        reachable: bool,
        tail: bool,
    ) -> Value {
        self.check_path(reachable && env.reachable(), |checker| {
            checker.infer_position(form, env, tail)
        })
    }

    pub(super) fn check_path(
        &mut self,
        reachable: bool,
        check: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        if reachable {
            return check(self);
        }
        let mut value = self.inspect_path(false, check);
        value.narrow(Type::Never);
        value
    }

    pub(super) fn inspect_path<T>(
        &mut self,
        reachable: bool,
        check: impl FnOnce(&mut Self) -> T,
    ) -> T {
        if reachable {
            return check(self);
        }
        let start = self.diagnostics.len();
        let enclosing_throws = self.throws.take();
        let enclosing_reachability = std::mem::replace(&mut self.path_reachable, false);
        let old = self
            .recur_target
            .as_mut()
            .map(|target| std::mem::replace(&mut target.reachable, false));
        let value = check(self);
        self.throws = enclosing_throws;
        self.path_reachable = enclosing_reachability;
        if let Some((target, old)) = self.recur_target.as_mut().zip(old) {
            target.reachable = old;
        }
        // Unexecuted forms must still resolve and pass compiler-level checks.
        let errors = self.diagnostics.split_off(start);
        self.diagnostics.extend(errors.into_iter().filter(|d| {
            !matches!(
                d.code.as_str(),
                "PNX2322" | "PNX2741" | "PNX2801" | "PNX2802" | "PNX9004"
            )
        }));
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
        tail: bool,
    ) -> Value {
        if xs.len() == 2 {
            return self.infer_position(&xs[1], env, tail && !threading.guards_nil());
        }
        let mut current = self.infer(&xs[1], env);
        let temporary = expand::temporary(&xs[1..], |name| env.contains_key(name));
        let mut local = env.clone();
        if threading.guards_nil() && nil_part(&current.known) == Type::Nil {
            // Every executed step passed the first nil test. Repeated reads
            // can share that fact only when they refer to the saved seed.
            self.narrow_subject(&xs[1], &mut local, Type::without_nil);
        }
        local.insert(temporary.clone(), Value::new(Type::Never));
        let mut origin = xs[1].span;
        let mut skipped_nil = false;
        for (index, step) in xs[2..].iter().enumerate() {
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
            current = self.infer_path(&call, &local, reachable, tail && index + 3 == xs.len());
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
        tail: bool,
    ) -> Value {
        let mut current = self.infer(&xs[1], env);
        if xs.len() == 2 {
            return current;
        }
        let temporary = expand::temporary(&xs[1..], |name| env.contains_key(name));
        let mut origin = xs[1].span;
        let mut aliases_seed = true;
        let mut joined_branches = false;
        for (index, [test, step]) in xs[2..].as_chunks::<2>().0.iter().enumerate() {
            let start = self.diagnostics.len();
            let condition = self.infer_path(test, env, current.known != Type::Never, false);
            self.expansion_origin(start, name, span, false);
            let mut yes_reachable = condition.known.truthy() != Type::Never;
            let no_reachable = condition.known.falsey() != Type::Never;
            let mut yes_env = env.clone();
            self.narrow(test, true, &mut yes_env);
            yes_reachable &= yes_env.reachable();
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
                    if !no_env.reachable() {
                        skipped = Value::new(Type::Never);
                    } else if let Some(value) = no_env.get(seed) {
                        skipped = value.clone();
                    }
                }
            }
            joined_branches |= yes_reachable && skipped.known != Type::Never;
            yes_env.insert(temporary.clone(), current);
            let call = threading.apply(
                step,
                Form {
                    kind: Kind::Symbol(temporary.clone()),
                    span: origin,
                },
            );
            let start = self.diagnostics.len();
            let applied = self.infer_path(
                &call,
                &yes_env,
                yes_reachable,
                tail && 2 * index + 4 == xs.len(),
            );
            self.expansion_origin(start, name, span, true);
            if self.untyped_binding_budget.is_some()
                && (self.type_query.is_none()
                    || !bounded_state(std::slice::from_ref(&applied))
                    || !bounded_state(std::slice::from_ref(&skipped)))
            {
                self.type_query = None;
                return Value::new(Type::Unknown);
            }
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

    pub(super) fn named_thread(&mut self, xs: &[Form], span: Span, env: &Env, tail: bool) -> Value {
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
        if let Some(diagnostic) = dotted_binding_error(binding, xs[2].without_metadata().span) {
            self.diagnostics.push(diagnostic);
            return Value::new(Type::Unknown);
        }
        self.with_thread_expansion(span, 1, |checker| {
            if xs.len() == 3 {
                return checker.infer(&xs[1], env);
            }
            let mut current = checker.infer(&xs[1], env);
            let mut local = env.clone();
            for (index, step) in xs[3..].iter().enumerate() {
                let reachable = current.known != Type::Never;
                local.insert(binding.to_owned(), current);
                let start = checker.diagnostics.len();
                current =
                    checker.infer_path(step, &local, reachable, tail && index + 4 == xs.len());
                checker.expansion_origin(start, "clojure.core/as->", span, false);
            }
            current
        })
    }

    pub(super) fn let_form(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        tail: bool,
        raw: bool,
    ) -> Value {
        let Some(bindings) = xs.get(1).and_then(Form::vector) else {
            if raw {
                self.error(
                    "PNX1001",
                    xs.get(1).map_or(span, |form| form.span),
                    "let* requires a binding vector",
                );
                return Value::new(Type::Unknown);
            }
            return self.unsupported(span, "let requires a binding vector");
        };
        let (pairs, remaining) = bindings.as_chunks::<2>();
        if !remaining.is_empty() {
            if raw {
                self.error("PNX1001", xs[1].span, "let* requires binding/value pairs");
                return Value::new(Type::Unknown);
            }
            return self.unsupported(xs[1].span, "let requires binding/value pairs");
        }
        if raw
            && let Some([pattern, _]) = pairs
                .iter()
                .find(|[pattern, _]| raw_local_symbol(pattern).is_none())
        {
            self.error(
                "PNX1001",
                pattern.without_metadata().span,
                "let* requires an unqualified local symbol other than &",
            );
            return Value::new(Type::Unknown);
        }
        let mut local = env.clone();
        let mut reachable = true;
        let start = self.diagnostics.len();
        let mut checked_inline = false;
        for pair in pairs {
            if self.inspect_path(reachable && local.reachable(), |checker| {
                checker.bind_inline(&pair[0], &pair[1], &mut local)
            }) {
                checked_inline = true;
                continue;
            }
            reachable = self.let_binding(&pair[0], &pair[1], &mut local, reachable);
        }
        let value = self.check_path(reachable && local.reachable(), |checker| {
            checker.body(&xs[2..], &local, tail)
        });
        if checked_inline {
            self.dedup_diagnostics(start);
        }
        value
    }

    pub(super) fn if_form(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        mode: Conditional,
        tail: bool,
    ) -> Value {
        let when = !mode.takes_else();
        if xs.len() < 2 || (!when && !(3..=4).contains(&xs.len())) {
            self.error(
                "PNX2554",
                span,
                format!("Invalid arity for {}", mode.name()),
            );
            return Value::new(Type::Unknown);
        }
        let runs_on_truth = mode.runs_on_truth();
        let branches = self.check_if_paths(&xs[1], env, runs_on_truth, |checker, scope, truth| {
            let body = if truth == runs_on_truth {
                &xs[2..if when { xs.len() } else { 3 }]
            } else if when {
                &[]
            } else {
                &xs[3..]
            };
            checker.body(body, scope, tail)
        });
        Value::union_branches(branches)
    }

    pub(super) fn check_if_paths(
        &mut self,
        test: &Form,
        env: &Env,
        first_truth: bool,
        mut branch: impl FnMut(&mut Self, &Env, bool) -> Value,
    ) -> [Value; 2] {
        let condition = self.infer(test, env);
        [first_truth, !first_truth].map(|truth| {
            let possible = if truth {
                condition.known.truthy()
            } else {
                condition.known.falsey()
            };
            let mut scope = env.clone();
            self.narrow(test, truth, &mut scope);
            self.check_path(
                !condition.stops_evaluation() && possible != Type::Never && scope.reachable(),
                |checker| branch(checker, &scope, truth),
            )
        })
    }

    pub(super) fn cond_form(
        &mut self,
        clauses: &[Form],
        span: Span,
        env: &Env,
        tail: bool,
    ) -> Value {
        let (pairs, remaining) = clauses.as_chunks::<2>();
        if !remaining.is_empty() {
            self.error("PNX2554", span, "cond requires test/result pairs");
            return Value::new(Type::Unknown);
        }
        let mut fallthrough = env.clone();
        let mut reachable = true;
        let mut branches = Vec::new();
        for [test, result] in pairs {
            let condition = self.infer_path(test, &fallthrough, reachable, false);
            let returns = !condition.stops_evaluation();
            let mut selected = fallthrough.clone();
            self.narrow(test, true, &mut selected);
            let branch = self.infer_path(
                result,
                &selected,
                returns && condition.known.truthy() != Type::Never,
                tail,
            );
            branches.push(branch);
            self.narrow(test, false, &mut fallthrough);
            reachable =
                returns && condition.known.falsey() != Type::Never && fallthrough.reachable();
        }
        if reachable {
            branches.push(Value::new(Type::Nil));
        }
        Value::union_branches(branches)
    }

    pub(super) fn condp_form(
        &mut self,
        forms: &[Form],
        span: Span,
        env: &Env,
        tail: bool,
    ) -> Value {
        let [head, predicate, subject, clauses @ ..] = forms else {
            self.error(
                "PNX2554",
                span,
                "condp requires a predicate and an expression",
            );
            return Value::new(Type::Unknown);
        };
        if !self
            .model
            .can_use_standard_core(head.symbol().unwrap(), self.globals)
        {
            return self.unchecked_condp(
                head.span,
                "Standard clojure.core/condp is unconfirmed",
                env,
            );
        }
        if !matches!(predicate.kind, Kind::Symbol(_))
            || self.standard_core_name(predicate, env).as_deref() != Some("clojure.core/=")
        {
            return self.unchecked_condp(
                predicate.span,
                "condp analysis requires the standard = predicate",
                env,
            );
        }
        let value = self.infer(subject, env);
        let (pairs, fallback) = clauses.as_chunks::<2>();
        let mut tests = Vec::with_capacity(pairs.len());
        for [test, result] in pairs {
            let Some(literal) = self
                .guard_literal(test)
                .filter(|_| !has_evaluated_metadata(test))
            else {
                return self.unchecked_condp(
                    test.span,
                    "condp = analysis requires scalar literal tests",
                    env,
                );
            };
            if result.keyword() == Some(":>>") {
                return self.unchecked_condp(result.span, "condp :>> clauses are unchecked", env);
            }
            tests.push((literal, result));
        }

        let mut remaining = if value.stops_evaluation() {
            Type::Never
        } else {
            value.known.clone()
        };
        // JVM equality can delegate to a collection operand's custom equiv.
        if !tests.is_empty() && !condp_scalar(&remaining) {
            return self.unchecked_condp(
                subject.span,
                "condp = analysis requires a known scalar expression",
                env,
            );
        }
        let mut fallthrough = env.clone();
        let mut outcomes = Vec::with_capacity(tests.len() + fallback.len());
        let mut compared = BTreeSet::new();
        let mut matched = false;
        for (literal, result) in &tests {
            if !compared.insert(literal) {
                outcomes.push(self.infer_path(result, &fallthrough, false, tail));
                continue;
            }
            let selected = narrow_literal(&remaining, literal, true);
            matched |= selected != Type::Never;
            let mut branch = fallthrough.clone();
            self.narrow_subject(subject, &mut branch, |ty| narrow_literal(ty, literal, true));
            outcomes.push(self.infer_path(result, &branch, selected != Type::Never, tail));
            self.narrow_subject(subject, &mut fallthrough, |ty| {
                narrow_literal(ty, literal, false)
            });
            remaining = narrow_literal(&remaining, literal, false);
        }
        if let Some(default) = fallback.first() {
            outcomes.push(self.infer_path(default, &fallthrough, remaining != Type::Never, tail));
        } else if remaining != Type::Never {
            if condp_scalar(&remaining) {
                self.illegal_argument();
            } else {
                // With no tests the macro still prints the subject in its error.
                self.may_throw();
            }
            if !matched {
                self.error(
                    "PNX2802",
                    subject.span,
                    format!("condp has no matching clause for {remaining}"),
                );
            }
        }
        if value.is_rejected() {
            value
        } else {
            Value::union_branches(outcomes)
        }
    }

    fn unchecked_condp(&mut self, span: Span, reason: &str, env: &Env) -> Value {
        self.withhold_captured_facts(env.inline_functions());
        self.withhold_literals_after_opaque_form();
        self.unchecked_form(span, reason)
    }

    pub(super) fn binding_conditional(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        name: &str,
        tail: bool,
    ) -> Value {
        Value::union_branches(self.check_binding_conditional_paths(
            xs,
            span,
            env,
            name,
            |checker, scope, body, _| match body {
                [form] => checker.infer_position(form, scope, tail),
                _ => checker.body(body, scope, tail),
            },
        ))
    }

    pub(super) fn check_binding_conditional_paths<'a>(
        &mut self,
        xs: &'a [Form],
        span: Span,
        env: &Env,
        name: &str,
        mut branch: impl FnMut(&mut Self, &Env, &'a [Form], bool) -> Value,
    ) -> [Value; 2] {
        let Some(paths) = self.prepare_binding_paths(xs, span, env, name) else {
            return [Value::new(Type::Unknown), Value::new(Type::Never)];
        };
        let yes = self.check_path(paths.bound_scope.reachable(), |checker| {
            branch(checker, &paths.bound_scope, paths.body, true)
        });
        let no = match &paths.alternate {
            Some((body, scope)) => self
                .check_path(paths.skips_binding && scope.reachable(), |checker| {
                    branch(checker, scope, body, false)
                }),
            None if paths.skips_binding => Value::new(Type::Nil),
            None => Value::new(Type::Never),
        };
        [yes, no]
    }

    // Keep temporary initializer types off the stack while checking branch bodies.
    #[inline(never)]
    fn prepare_binding_paths<'a>(
        &mut self,
        xs: &'a [Form],
        span: Span,
        env: &Env,
        name: &str,
    ) -> Option<BindingPaths<'a>> {
        let when = name.starts_with("clojure.core/when-");
        if xs.len() < 2 || (!when && !(3..=4).contains(&xs.len())) {
            self.error("PNX2554", span, format!("Invalid arity for {name}"));
            return None;
        }
        let Some([pattern, init]) = xs[1].vector() else {
            self.error(
                "PNX1001",
                xs[1].span,
                format!("{name} requires a binding vector with exactly two forms"),
            );
            return None;
        };
        let valid_pattern = match &pattern.without_metadata().kind {
            Kind::Symbol(symbol) | Kind::AnonymousArgument(symbol) => {
                symbol != "&" && !symbol.contains('/')
            }
            Kind::Map(_) | Kind::Vector(_) => true,
            _ => false,
        };
        if !valid_pattern {
            self.error(
                "PNX1001",
                pattern.span,
                format!("Invalid binding form for {name}"),
            );
            return None;
        }

        let mut input = self.infer(init, env);
        let first = name == "clojure.core/when-first";
        if input.stops_evaluation() {
            input.narrow(Type::Never);
        } else if first {
            input = self.sequence_call("clojure.core/seq", &input, init.span);
        }
        let non_nil = first || name.ends_with("-some");
        let (matched, skipped) = if non_nil {
            (input.known.without_nil(), nil_part(&input.known))
        } else {
            (input.known.truthy(), input.known.falsey())
        };
        let skips_binding = skipped != Type::Never;
        let narrow = |truth: bool, scope: &mut Env| {
            if first {
                if self
                    .model
                    .can_use_standard_core(xs[0].symbol().unwrap(), self.globals)
                {
                    self.narrow_sequence_subject(init, scope, !truth);
                }
            } else if non_nil {
                self.narrow_subject(init, scope, |ty| {
                    if truth {
                        ty.without_nil()
                    } else {
                        nil_part(ty)
                    }
                });
            } else {
                self.narrow(init, truth, scope);
            }
        };
        let mut yes_env = env.clone();
        narrow(true, &mut yes_env);
        let alternate = if !when && xs.len() == 4 {
            let mut scope = env.clone();
            narrow(false, &mut scope);
            Some((&xs[3..], scope))
        } else {
            None
        };

        let reachable = matched != Type::Never && yes_env.reachable();
        input.narrow(matched);
        self.inspect_path(reachable, |checker| {
            if first {
                input = checker.sequence_call("clojure.core/first", &input, init.span);
                checker.bind(pattern, input, &mut yes_env);
            } else {
                checker.bind_initializer(pattern, init, input, &mut yes_env);
            }
        });
        if !reachable {
            yes_env.mark_impossible();
        }
        Some(BindingPaths {
            body: if when { &xs[2..] } else { &xs[2..3] },
            bound_scope: yes_env,
            alternate,
            skips_binding,
        })
    }

    pub(super) fn short_circuit(&mut self, xs: &[Form], env: &Env, or: bool, tail: bool) -> Value {
        if xs.len() == 1 {
            return Value::new(if or { Type::Nil } else { Type::Bool(true) });
        }
        let mut local = env.clone();
        let mut branches = Vec::new();
        let mut reachable = true;
        for (index, x) in xs[1..].iter().enumerate() {
            let last = index + 2 == xs.len();
            let mut value = self.infer_path(x, &local, reachable, tail && last);
            if !reachable {
                continue;
            }
            if value.stops_evaluation() {
                reachable = false;
                branches.push(value);
                continue;
            }
            if last {
                branches.push(value);
                break;
            }
            let selected = if or {
                value.known.truthy()
            } else {
                value.known.falsey()
            };
            if (if or {
                value.ty.falsey()
            } else {
                value.ty.truthy()
            }) == Type::Never
            {
                reachable = false;
            }
            value.narrow(selected);
            branches.push(value);
            self.narrow(x, !or, &mut local);
            reachable &= local.reachable();
        }
        Value::union_branches(branches)
    }

    fn unshadowed_operation(&self, head: &Form, env: &Env) -> Option<String> {
        let op = head.symbol()?;
        // Locals and project functions may shadow core predicates.
        if !self.allows_partial_call(head, env)
            || env.contains_key(op)
            || self.model.runtime_alias_gap(op).is_some()
            || self.model.local_function(op).is_some()
            || self
                .model
                .declared_value(op, self.globals, head.span.start.offset)
                .is_some()
        {
            return None;
        }
        let op = self.model.resolve(op);
        (op.starts_with("clojure.core/")
            || op == "clojure.string/blank?"
            || self.is_spec_validation(&op))
        .then_some(op)
    }

    pub(super) fn narrow(&self, test: &Form, truth: bool, env: &mut Env) {
        if !env.reachable() {
            return;
        }
        let truthy = |ty: &Type| if truth { ty.truthy() } else { ty.falsey() };
        if test.symbol().is_some() {
            self.narrow_subject(test, env, truthy);
            return;
        }
        let Kind::List(xs) = &test.without_metadata().kind else {
            return;
        };
        let Some((head, args)) = xs.split_first() else {
            return;
        };
        if !self.allows_partial_call(head, env) {
            return;
        }
        let Some(op) = self.unshadowed_operation(head, env) else {
            if head.keyword().is_some() {
                self.narrow_subject(test, env, truthy);
            } else if let Some(name) = head.symbol() {
                let guard = if let Some(value) = env.get(name) {
                    value.guard.clone()
                } else if let Some(DeclaredValue::Function(contract)) =
                    self.model
                        .declared_value(name, self.globals, head.span.start.offset)
                {
                    contract.guard.clone()
                } else {
                    None
                };
                if let Some(guard) = guard
                    && args.len() == guard.arity
                    && (truth || guard.both)
                {
                    self.narrow_subject(&args[guard.argument], env, |ty| {
                        narrow_by_type(ty, &guard.ty, truth)
                    });
                }
            }
            return;
        };
        match (op.as_str(), args) {
            ("clojure.core/contains?", _) => self.narrow_contains(args, truth, env),
            ("clojure.core/get" | "clojure.core/get-in", [_, _]) => {
                self.narrow_subject(test, env, truthy);
            }
            ("clojure.core/=", [left, right]) => {
                let pair = self
                    .guard_literal(left)
                    .map(|ty| (right, ty))
                    .or_else(|| self.guard_literal(right).map(|ty| (left, ty)));
                if let Some((subject, literal)) = pair {
                    self.narrow_subject(subject, env, |ty| narrow_literal(ty, &literal, truth));
                }
            }
            ("clojure.core/and" | "clojure.core/or", [operand]) => self.narrow(operand, truth, env),
            ("clojure.core/and", _) if truth => {
                for operand in args {
                    self.narrow(operand, true, env);
                }
            }
            ("clojure.core/or", _) if !truth => {
                for operand in args {
                    self.narrow(operand, false, env);
                }
            }
            ("clojure.core/not", [operand]) => self.narrow(operand, !truth, env),
            ("clojure.core/empty?", [operand]) => {
                self.narrow_subject(operand, env, |ty| narrow_emptiness(ty, truth));
            }
            ("clojure.core/seq" | "clojure.core/not-empty", [operand])
                if self
                    .model
                    .can_use_standard_core(head.symbol().unwrap(), self.globals) =>
            {
                self.narrow_sequence_subject(operand, env, !truth);
            }
            ("clojure.core/nil?", [operand]) => {
                self.narrow_subject(operand, env, |ty| narrow_by_type(ty, &Type::Nil, truth));
            }
            ("clojure.core/some?", [operand]) => {
                self.narrow_subject(operand, env, |ty| {
                    if truth {
                        ty.without_nil()
                    } else {
                        nil_part(ty)
                    }
                });
            }
            ("clojure.core/map?", [operand])
                if !self.globals.project_namespaces.contains("clojure.core") =>
            {
                self.narrow_subject(operand, env, |ty| narrow_map_predicate(ty, truth));
            }
            ("clojure.core/vector?", [operand])
                if !self.globals.project_namespaces.contains("clojure.core") =>
            {
                self.narrow_subject(operand, env, |ty| narrow_vector_predicate(ty, truth));
            }
            ("clojure.string/blank?", [operand]) if !truth => {
                self.narrow_subject(operand, env, Type::truthy);
            }
            ("clojure.spec.alpha/valid?", [spec, operand] | [spec, operand, _]) if truth => {
                if let Some(projection) = self.validation_projection(spec) {
                    self.narrow_subject(operand, env, |ty| spec_guards::refine(ty, projection));
                }
            }
            (_, [operand]) => {
                if let Some(wanted) = predicate_type(&op) {
                    self.narrow_subject(operand, env, |ty| narrow_by_type(ty, &wanted, truth));
                }
            }
            _ => {}
        }
    }
}

pub(super) fn narrow_by_type(actual: &Type, wanted: &Type, truth: bool) -> Type {
    match actual {
        Type::Parameter(parameter) => {
            let impossible = parameter.upper.as_ref().is_some_and(|upper| {
                if truth {
                    // The supported bounds and core predicate types are
                    // pairwise disjoint. A general wanted type can overlap.
                    matches!(
                        wanted,
                        Type::Nil
                            | Type::Number
                            | Type::String
                            | Type::Character
                            | Type::Boolean
                            | Type::Keyword
                            | Type::Symbol
                    ) && !upper.subtype_of(wanted)
                } else {
                    upper.subtype_of(wanted)
                }
            });
            if impossible {
                Type::Never
            } else {
                actual.clone()
            }
        }
        Type::Union(parts) => {
            Type::union(parts.iter().map(|part| narrow_by_type(part, wanted, truth)))
        }
        Type::Symbol if matches!(wanted, Type::SimpleSymbol | Type::QualifiedSymbol) => {
            match (wanted, truth) {
                (Type::SimpleSymbol, true) | (Type::QualifiedSymbol, false) => Type::SimpleSymbol,
                _ => Type::QualifiedSymbol,
            }
        }
        actual if actual.subtype_of(wanted) => {
            if truth {
                actual.clone()
            } else {
                Type::Never
            }
        }
        Type::SpecTruthy if truth => wanted.spec_truthy().unwrap_or_else(|| wanted.clone()),
        Type::SpecValue if truth => wanted.spec_value().unwrap_or_else(|| wanted.clone()),
        Type::Any | Type::SpecValue if !truth && *wanted == Type::Nil => actual.without_nil(),
        actual if truth && let Some(ty) = actual.integer_intersection(wanted) => ty,
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
