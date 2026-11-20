//! Tail transfers and bounded type inference for loop and function bodies.
use std::collections::{BTreeSet, VecDeque};

use super::function_forms::FunctionConditions;
use super::{Checker, Env, Value};
use crate::reader::{Form, Kind, Span};
use crate::types::{ExtraFields, IntegerType, Type};

const MAX_STATES: usize = 16;
const MAX_EXACT_INTEGERS_PER_SLOT: usize = 8;
const MAX_RECHECKS: usize = 128;
const MAX_STATE_NODES: usize = 2048;

pub(super) struct RecurTarget {
    kind: &'static str,
    arity: usize,
    pub(super) reachable: bool,
    pub(super) blocked_by_try: bool,
    edges: Vec<RecurEdge>,
}

struct RecurEdge {
    args: Vec<Value>,
    span: Span,
}

impl Checker<'_> {
    pub(super) fn recur_form(&mut self, args: &[Form], span: Span, env: &Env, tail: bool) -> Value {
        let Some(target) = &self.recur_target else {
            self.error("PNX1001", span, "recur requires an enclosing loop or fn");
            return Value::new(Type::Unknown);
        };
        if !tail {
            self.error("PNX1001", span, "recur is only allowed in tail position");
            return Value::new(Type::Unknown);
        }
        if target.blocked_by_try {
            self.error("PNX1001", span, "Cannot recur across try");
            return Value::new(Type::Unknown);
        }
        if args.len() != target.arity {
            self.error(
                "PNX2554",
                span,
                format!(
                    "recur to {} expects {} arguments, got {}",
                    target.kind,
                    target.arity,
                    args.len()
                ),
            );
            return Value::new(Type::Unknown);
        }
        let args: Vec<_> = self
            .eval_args(args, env, true)
            .map(|(value, _)| value)
            .collect();
        let target = self.recur_target.as_mut().unwrap();
        if target.reachable && args.iter().all(|arg| arg.known != Type::Never) {
            target.edges.push(RecurEdge { args, span });
        }
        Value::new(Type::Never)
    }

    pub(super) fn loop_form(&mut self, xs: &[Form], span: Span, env: &Env, raw: bool) -> Value {
        let Some(bindings) = xs.get(1).and_then(Form::vector) else {
            self.error("PNX1001", span, "loop requires a binding vector");
            return Value::new(Type::Unknown);
        };
        let (bindings, remainder) = bindings.as_chunks::<2>();
        if !remainder.is_empty() {
            self.error("PNX1001", xs[1].span, "loop requires binding/value pairs");
            return Value::new(Type::Unknown);
        }
        let mut destructures = false;
        for [pattern, _] in bindings {
            match &pattern.without_metadata().kind {
                Kind::Symbol(name) | Kind::AnonymousArgument(name)
                    if name != "&" && !name.contains('/') => {}
                Kind::Map(_) | Kind::Vector(_) if !raw => destructures = true,
                _ => {
                    self.error("PNX1001", pattern.span, "Invalid loop binding");
                    return Value::new(Type::Unknown);
                }
            }
        }
        let mut outer = env.clone();
        let mut initial = Vec::with_capacity(bindings.len());
        let start = self.diagnostics.len();
        let mut reachable = true;
        for [pattern, init] in bindings {
            let value = self.check_path(reachable, |checker| {
                let value = checker.infer(init, &outer);
                checker.check_path(value.known != Type::Never, |checker| {
                    checker.bind(pattern, value.clone(), &mut outer);
                    value
                })
            });
            reachable = value.known != Type::Never;
            initial.push(value);
        }

        let result = self.recur_body("loop", span, initial, |checker, state| {
            // The loop macro retains the initial let scope for destructuring.
            // Each iteration starts with all raw slots in scope, then expands
            // the patterns in order. Recur's arity is the number of raw slots.
            let mut local = if destructures {
                outer.clone()
            } else {
                env.clone()
            };
            for ([pattern, _], value) in bindings.iter().zip(state) {
                if let Some(name) = pattern.symbol() {
                    local.insert(name.to_string(), value.clone());
                }
            }
            if destructures {
                for ([pattern, _], value) in bindings.iter().zip(state) {
                    let value = pattern
                        .symbol()
                        .and_then(|name| local.get(name))
                        .unwrap_or(value)
                        .clone();
                    checker.bind(pattern, value, &mut local);
                }
            }
            checker.body(&xs[2..], &local, true)
        });
        if destructures {
            self.dedup_diagnostics(start);
        }
        result
    }

    pub(super) fn function_body(
        &mut self,
        params: &[Form],
        conditions: FunctionConditions<'_>,
        body: &[Form],
        args: Vec<Value>,
        env: &Env,
        span: Span,
    ) -> Value {
        self.recur_body("fn", span, args, |checker, state| {
            let local = checker.bind_parameters(env.clone(), params, |index| state[index].clone());
            let mut active = (!conditions.pre.is_empty()).then(|| local.clone());
            let preconditions_pass = active.as_mut().map_or(checker.path_reachable, |env| {
                checker.check_conditions(
                    conditions.pre,
                    env,
                    checker.path_reachable,
                    "precondition",
                )
            });
            // The body can also run with assertions disabled. A postcondition
            // wrapper removes its tail position in both compiler modes.
            let result = checker.body(body, &local, conditions.post.is_none());
            if let Some(post) = conditions.post.filter(|post| !post.is_empty()) {
                let mut active = active.unwrap_or(local);
                let reachable =
                    preconditions_pass && active.reachable() && result.known != Type::Never;
                // A directly returned local keeps any narrowing from the
                // enabled preconditions. The overall return still includes
                // the path where the compiler elides both sets of checks.
                let post_value = body
                    .last()
                    .and_then(Form::symbol)
                    .and_then(|name| active.get(name))
                    .unwrap_or(&result)
                    .clone();
                active.insert("%".into(), post_value);
                checker.check_conditions(post, &mut active, reachable, "postcondition");
            }
            result
        })
    }

    fn check_conditions(
        &mut self,
        conditions: &[Form],
        env: &mut Env,
        mut reachable: bool,
        description: &str,
    ) -> bool {
        if self.globals.project_namespaces.contains("clojure.core")
            || self.model.resolve("clojure.core/assert") != "clojure.core/assert"
        {
            self.inspect_path(reachable, |checker| {
                checker.may_throw();
                checker.unsupported(
                    conditions[0].span,
                    format!(
                        "Function {description}s require the standard clojure.core/assert macro"
                    ),
                );
            });
            return reachable;
        }
        let start = self.diagnostics.len();
        for condition in conditions {
            let value = self.infer_path(condition, env, reachable, false);
            if value.known.falsey() != Type::Never {
                self.assertion_error();
            }
            reachable = value.known.truthy() != Type::Never;
            if reachable {
                self.narrow(condition, true, env);
            }
        }
        for diagnostic in &mut self.diagnostics[start..] {
            diagnostic.notes.push(format!(
                "This {description} runs when *assert* is enabled at compile time."
            ));
        }
        reachable
    }

    fn recur_body(
        &mut self,
        kind: &'static str,
        span: Span,
        initial: Vec<Value>,
        check: impl Fn(&mut Self, &[Value]) -> Value,
    ) -> Value {
        let enclosing = self.recur_target.replace(RecurTarget {
            kind,
            arity: initial.len(),
            reachable: true,
            blocked_by_try: false,
            edges: Vec::new(),
        });
        let start = self.diagnostics.len();
        let reachable = initial.iter().all(|arg| arg.known != Type::Never);
        let mut result = self.check_path(reachable, |checker| check(checker, &initial));
        let mut target = self.recur_target.take().unwrap();
        if target
            .edges
            .iter()
            .any(|edge| !same_state(&edge.args, &initial))
        {
            let mut seen = vec![initial];
            let mut pending = VecDeque::from(std::mem::take(&mut target.edges));
            while let Some(RecurEdge {
                args: mut state,
                span: edge,
            }) = pending.pop_front()
            {
                if seen.iter().any(|previous| same_state(&state, previous)) {
                    continue;
                }
                let widened_collections = widen_growing_collections(&mut state, &seen);
                if (widen_changing_integers(&mut state, &seen) || widened_collections)
                    && seen.iter().any(|previous| same_state(&state, previous))
                {
                    continue;
                }
                if seen.len() == MAX_STATES
                    || self.recur_checks >= MAX_RECHECKS
                    || !bounded_state(&state)
                {
                    if self.path_reachable
                        && self.local_read.as_ref().is_some_and(|read| {
                            span.start.offset <= read.span.start.offset
                                && read.span.end.offset <= span.end.offset
                        })
                    {
                        self.local_read = None;
                    }
                    result = self.unsupported(
                        span,
                        format!("recur type inference limit reached for {kind}"),
                    );
                    break;
                }
                self.recur_checks += 1;
                self.recur_target = Some(target);
                let before = self.diagnostics.len();
                result = result.join(check(self, &state));
                for diagnostic in &mut self.diagnostics[before..] {
                    diagnostic.notes.push(format!(
                        "On a later iteration reached by recur at {}:{}",
                        edge.start.line, edge.start.column
                    ));
                }
                target = self.recur_target.take().unwrap();
                pending.extend(std::mem::take(&mut target.edges));
                seen.push(state);
            }
            self.dedup_diagnostics(start);
        }
        self.recur_target = enclosing;
        result
    }
}

fn same_state(left: &[Value], right: &[Value]) -> bool {
    left.iter()
        .zip(right)
        .all(|(left, right)| left.same_value_facts(right))
}

fn widen_changing_integers(state: &mut [Value], seen: &[Vec<Value>]) -> bool {
    if seen.len() < MAX_EXACT_INTEGERS_PER_SLOT {
        return false;
    }
    let fixed = Type::Integral(IntegerType::FIXED);
    let mut changed = false;
    for (index, value) in state.iter_mut().enumerate() {
        if value.known.exact_long_value().is_none() || !value.known.subtype_of(&fixed) {
            continue;
        }
        let previous: BTreeSet<_> = seen
            .iter()
            .map(|state| &state[index].known)
            .filter(|ty| ty.subtype_of(&fixed))
            .filter_map(Type::exact_long_value)
            .collect();
        if previous.len() >= MAX_EXACT_INTEGERS_PER_SLOT {
            value.narrow(fixed.clone());
            changed = true;
        }
    }
    changed
}

pub(super) fn widen_growing_collections(state: &mut [Value], seen: &[Vec<Value>]) -> bool {
    // Two earlier states with the same other slots distinguish repeated growth
    // from a single append before a loop exits.
    let mut prior = Vec::new();
    for previous in seen.iter().rev() {
        let Some(slots) = growing_slots(state, previous) else {
            continue;
        };
        if !prior.contains(&slots) {
            prior.push(slots);
            continue;
        }
        for index in slots {
            let value = &mut state[index];
            let known = match &value.known {
                Type::Tuple(items) => Type::Vector(Box::new(Type::union(items.iter().cloned()))),
                Type::SeqTuple(items) => Type::Seq(Box::new(Type::union(items.iter().cloned()))),
                Type::ListTuple(items) => Type::List(Box::new(Type::union(items.iter().cloned()))),
                _ => unreachable!(),
            };
            value.narrow(known);
            value
                .notes
                .push("The collection length can grow across iterations.".into());
        }
        return true;
    }
    false
}

fn growing_slots(current: &[Value], previous: &[Value]) -> Option<Vec<usize>> {
    let mut growing = Vec::new();
    for (index, (current, previous)) in current.iter().zip(previous).enumerate() {
        if current.same_value_facts(previous) {
            continue;
        }
        match (&current.known, &previous.known) {
            (Type::Tuple(items), Type::Tuple(old)) if items.starts_with(old) => {}
            (Type::SeqTuple(items), Type::SeqTuple(old))
            | (Type::ListTuple(items), Type::ListTuple(old))
                if items.ends_with(old) => {}
            _ => return None,
        }
        growing.push(index);
    }
    (!growing.is_empty()).then_some(growing)
}

// A growing shape such as (recur [state state]) can otherwise double in size
// at every step, even before it reaches the iteration limit.
pub(super) fn bounded_state(state: &[Value]) -> bool {
    fn visit(ty: &Type, remaining: &mut usize) -> bool {
        let Some(next) = remaining.checked_sub(1) else {
            return false;
        };
        *remaining = next;
        match ty {
            Type::Tuple(parts) | Type::SeqTuple(parts) | Type::ListTuple(parts) => {
                parts.iter().all(|part| visit(part, remaining))
            }
            Type::Union(parts) => parts.iter().all(|part| visit(part, remaining)),
            Type::MapEntry(parts) => parts.iter().all(|part| visit(part, remaining)),
            Type::Vector(part)
            | Type::Set(part)
            | Type::Seq(part)
            | Type::List(part)
            | Type::Reduced(part)
            | Type::ExceptionInfo(part) => visit(part, remaining),
            Type::Function(signatures) => signatures
                .iter()
                .all(|sig| sig.types().all(|arg| visit(arg, remaining))),
            Type::Map(map) => {
                map.fields
                    .iter()
                    .all(|(key, field)| visit(key, remaining) && visit(&field.ty, remaining))
                    && match &map.extra {
                        ExtraFields::Typed(parts) => {
                            parts.iter().all(|part| visit(part, remaining))
                        }
                        _ => true,
                    }
            }
            _ => true,
        }
    }
    let mut remaining = MAX_STATE_NODES;
    state.iter().all(|arg| visit(&arg.known, &mut remaining))
}
