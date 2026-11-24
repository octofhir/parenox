//! Ordered and bounded abstract reduction of modeled collections.
mod kv;

use std::borrow::Cow;

use super::super::recur::{bounded_state, widen_growing_collections};
use super::{
    Callable, Checker, Env, LiteralContainer, PreparedCallable, Value, vectors::CollectionError,
    vectors::element_type,
};
use crate::reader::{Form, Span};
use crate::types::{LengthBounds, Type};

const MAX_REDUCE_ALTERNATIVES: usize = 16;
const MAX_REDUCE_STATES: usize = 16;
const MAX_REDUCE_STEPS: usize = 128;

struct ReductionInput<'a> {
    items: &'a [Type],
    form: &'a Form,
    collection: &'a Value,
}

impl Checker<'_> {
    pub(super) fn reduce_form(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        let reducer = match forms {
            [reducer, _] | [reducer, _, _] => reducer,
            _ => {
                self.error(
                    "PNX2554",
                    span,
                    "reduce expects a function, an optional initial value and a collection",
                );
                return Value::new(Type::Unknown);
            }
        };
        let inputs = &forms[1..];
        self.with_callback_choice(reducer, env, |checker, selected, scope| {
            checker.reduce_using(selected, inputs, span, scope)
        })
    }

    fn reduce_using(&mut self, reducer: &Form, inputs: &[Form], span: Span, env: &Env) -> Value {
        let source = inputs.last().unwrap();
        let callable = self.prepare_callable(reducer, env);
        let mut arguments: Vec<_> = self.eval_args(inputs, env, callable.returns()).collect();
        let (collection, _) = arguments.pop().unwrap();
        let initial = arguments.first();
        if collection.known == Type::Never {
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }
        if collection.unproven_tuple_traversal() {
            self.withhold_callback_facts(&callable, env);
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return self.unchecked_tuple_traversal("clojure.core/reduce", source.span);
        }
        let shapes = match &collection.known {
            Type::Union(shapes) => shapes.as_slice(),
            shape => std::slice::from_ref(shape),
        };
        let shapes = self.reduce_shapes(shapes, source.span);
        if shapes.len() > MAX_REDUCE_ALTERNATIVES {
            return self.unsupported(
                source.span,
                "reduce has too many possible collection shapes",
            );
        }
        let mut result = Value::new(Type::Never);
        let start = self.diagnostics.len();
        for (shape, casts_reducer) in &shapes {
            if *casts_reducer
                && let Callable::Value(function) = &callable.kind
                && !reducer_argument(&function.known)
            {
                self.error(
                    "PNX2322",
                    callable.span,
                    format!("reduce expects a callable or Nil, got {}", function.ty),
                );
                self.may_throw();
                return Value::new(Type::Unknown);
            }
            let shape = shape.as_ref();
            let items = match shape {
                Type::Nil => &[][..],
                Type::Vector(element) if **element == Type::Never => &[][..],
                Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => items,
                Type::MapEntry(items) => items.as_slice(),
                Type::Vector(_)
                | Type::Seq(_)
                | Type::List(_)
                | Type::SpecCollection(_)
                | Type::BoundedCollection(..) => {
                    let element = element_type(shape.clone())
                        .unwrap_or_else(|_| unreachable!("a collection has an element type"));
                    let item = collection.traversal_element(element);
                    let bounds = if matches!(
                        collection.literal_container,
                        Some(LiteralContainer::NonemptyVector)
                    ) {
                        LengthBounds::NONEMPTY
                    } else {
                        shape.length_bounds()
                    };
                    result = result.join(self.reduce_open(
                        &callable,
                        initial,
                        (item, source.span),
                        bounds,
                        span,
                        env,
                    ));
                    continue;
                }
                Type::Unknown => {
                    self.call_prepared(&callable, Vec::new(), span, env, false);
                    return Value::new(Type::Unknown);
                }
                ty if matches!(element_type(ty.clone()), Err(CollectionError::NotSeqable)) => {
                    self.error(
                        "PNX2322",
                        source.span,
                        format!("reduce expects a collection, got {}", ty.inferred()),
                    );
                    self.may_throw();
                    return Value::new(Type::Unknown);
                }
                _ => {
                    self.call_prepared(&callable, Vec::new(), span, env, false);
                    return self.unsupported(
                        source.span,
                        "reduce currently requires a collection with known positions",
                    );
                }
            };
            let input = ReductionInput {
                items,
                form: source,
                collection: &collection,
            };
            result = result.join(self.reduce_exact(&callable, initial, input, span, env));
        }
        self.dedup_diagnostics(start);
        result
    }

    fn reduce_shapes<'a>(&mut self, shapes: &'a [Type], span: Span) -> Vec<(Cow<'a, Type>, bool)> {
        let mut alternatives = Vec::with_capacity(shapes.len());
        for shape in shapes {
            if matches!(
                shape,
                Type::Map(_) | Type::Set(_) | Type::String | Type::Str(_)
            ) {
                // The JVM's sequence fallback casts the reducer only when it
                // invokes it. Keep the original collection's behavior.
                let sequence = self.seq_type(shape, "reduce", span);
                if let Type::Union(parts) = sequence {
                    alternatives.extend(parts.into_iter().map(|part| (Cow::Owned(part), false)));
                } else {
                    alternatives.push((Cow::Owned(sequence), false));
                }
            } else {
                let casts_reducer = matches!(
                    shape,
                    Type::Tuple(_) | Type::Vector(_) | Type::List(_)
                ) || matches!(shape, Type::SeqTuple(items) | Type::ListTuple(items) if !items.is_empty());
                let casts_reducer = casts_reducer
                    || matches!(shape,
                        Type::BoundedCollection(collection, _) if matches!(collection.as_ref(), Type::Vector(_))
                    );
                alternatives.push((Cow::Borrowed(shape), casts_reducer));
            }
        }
        alternatives
    }

    fn reduce_exact(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        initial: Option<&(Value, Span)>,
        input: ReductionInput<'_>,
        span: Span,
        env: &Env,
    ) -> Value {
        let literal = input
            .form
            .vector()
            .filter(|forms| forms.len() == input.items.len());
        let mut elements = input.items.iter().enumerate().map(|(index, ty)| {
            let element_span = literal.map_or(input.form.span, |forms| forms[index].span);
            (
                input.collection.position_value(ty.clone(), index),
                element_span,
            )
        });
        let Some(accumulator) = initial.cloned().or_else(|| elements.next()) else {
            return self.call_prepared(callable, Vec::new(), span, env, true);
        };
        if elements.len() == 0 {
            self.call_prepared(callable, Vec::new(), span, env, false);
        }
        self.reduce_exact_with(
            accumulator,
            elements,
            span,
            |checker, accumulator, item, step| {
                checker.reduce_step(
                    callable,
                    [accumulator, item],
                    literal.is_none().then_some(input.form.span),
                    span,
                    env,
                    step,
                )
            },
        )
    }

    fn reduce_exact_with<T>(
        &mut self,
        mut accumulator: (Value, Span),
        items: impl Iterator<Item = T>,
        span: Span,
        mut apply: impl FnMut(&mut Self, (Value, Span), T, usize) -> Value,
    ) -> Value {
        let mut stopped = Value::new(Type::Never);
        for (index, item) in items.enumerate() {
            let next = apply(self, accumulator, item, index + 1);
            let (continuing, finished) = partition_reduced(next);
            accumulator = (continuing, span);
            stopped = stopped.join(finished);
            if matches!(accumulator.0.known, Type::Never | Type::Unknown) {
                break;
            }
        }
        stopped.join(accumulator.0)
    }

    fn reduce_open(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        initial: Option<&(Value, Span)>,
        input: (Value, Span),
        bounds: LengthBounds,
        span: Span,
        env: &Env,
    ) -> Value {
        let accumulator = initial.cloned().unwrap_or_else(|| input.clone());
        let steps = bounds.after_consuming(usize::from(initial.is_none()));
        let mut result = if steps.includes(0) && bounds.includes(usize::from(initial.is_none())) {
            accumulator.0.clone()
        } else {
            Value::new(Type::Never)
        };
        if initial.is_none() && bounds.includes(0) {
            result = result.join(self.call_prepared(callable, Vec::new(), span, env, true));
        } else if steps.max == Some(0) {
            self.call_prepared(callable, Vec::new(), span, env, false);
        }

        self.reduce_open_with(
            accumulator,
            result,
            steps,
            "reduce",
            span,
            |checker, accumulator, step| {
                checker.reduce_step(
                    callable,
                    [accumulator, input.clone()],
                    Some(input.1),
                    span,
                    env,
                    step,
                )
            },
        )
    }

    fn reduce_open_with(
        &mut self,
        mut accumulator: (Value, Span),
        mut result: Value,
        steps: LengthBounds,
        name: &str,
        span: Span,
        mut apply: impl FnMut(&mut Self, (Value, Span), usize) -> Value,
    ) -> Value {
        let mut seen = vec![vec![accumulator.0.clone()]];
        for step in 1..=steps
            .max
            .unwrap_or(MAX_REDUCE_STATES)
            .min(MAX_REDUCE_STATES)
        {
            let next = apply(self, accumulator, step);
            let (mut next, finished) = partition_reduced(next);
            result = result.join(finished);
            if next.known == Type::Never {
                return result;
            }
            if next.known == Type::Unknown {
                return next;
            }
            if steps.max == Some(step) {
                return result.join(next);
            }
            widen_growing_collections(std::slice::from_mut(&mut next), &seen);
            if !bounded_state(std::slice::from_ref(&next)) {
                break;
            }
            if step >= steps.min {
                result = result.join(next.clone());
            }
            if let Some(first) = seen
                .iter()
                .position(|state| state[0].same_value_facts(&next))
            {
                // The remaining lengths can only reach states in this cycle.
                let period = step - first;
                for (phase, state) in seen[first..].iter().enumerate() {
                    let at = step + phase;
                    let repetitions = steps.min.saturating_sub(at).div_ceil(period);
                    if steps.includes(at + repetitions * period) {
                        result = result.join(state[0].clone());
                    }
                }
                return result;
            }
            seen.push(vec![next.clone()]);
            accumulator = (next, span);
        }
        if steps.max == Some(0) {
            return result;
        }
        self.unsupported(
            span,
            format!("{name} accumulator types did not stabilize within the analysis limit"),
        )
    }

    fn reduce_step(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        args: impl Into<Vec<(Value, Span)>>,
        projected: Option<Span>,
        span: Span,
        env: &Env,
        step: usize,
    ) -> Value {
        if self.reduction_steps >= MAX_REDUCE_STEPS {
            return self.unsupported(span, "reduce step analysis limit (128) exceeded");
        }
        self.reduction_steps += 1;
        let start = self.diagnostics.len();
        let result = self.call_prepared(callable, args.into(), span, env, true);
        for diagnostic in &mut self.diagnostics[start..] {
            diagnostic
                .notes
                .push(format!("While checking reduction step {step}"));
            if diagnostic.code == "PNX2322" {
                let help = if diagnostic.span == span {
                    Some("The reducer's first argument receives the previous step's result.")
                } else if projected == Some(diagnostic.span) {
                    Some("Check the collection's elements against the reducer's argument types.")
                } else {
                    None
                };
                if let Some(help) = help {
                    diagnostic.suggestions.clear();
                    diagnostic.help = Some(help.into());
                }
            }
        }
        result
    }
}

fn reducer_argument(ty: &Type) -> bool {
    match ty {
        Type::Nil
        | Type::Any
        | Type::Unknown
        | Type::Never
        | Type::Function(_)
        | Type::Symbol
        | Type::SimpleSymbol
        | Type::QualifiedSymbol
        | Type::Sym(_)
        | Type::Key(_)
        | Type::Keyword
        | Type::SpecKeyword
        | Type::Map(_)
        | Type::Set(_) => true,
        Type::Union(parts) => parts.iter().all(reducer_argument),
        _ => ty.is_vector(),
    }
}

fn partition_reduced(value: Value) -> (Value, Value) {
    if value.literal_container.is_some() {
        return (value, Value::new(Type::Never));
    }
    let types = match value.known {
        Type::Union(parts) => parts,
        ty => vec![ty],
    };
    let mut continuing = Vec::new();
    let mut finished = Vec::new();
    for ty in types {
        match ty {
            Type::Reduced(value) => finished.push(*value),
            Type::Parameter(_) => {
                continuing.push(ty);
                finished.push(Type::Unknown);
            }
            ty => continuing.push(ty),
        }
    }
    (
        Value::with_notes(Type::union(continuing), value.notes.clone()),
        Value::with_notes(Type::union(finished), value.notes),
    )
}

pub(super) fn reduced_operation(name: &str, input: &Value) -> Value {
    if name == "clojure.core/reduced" {
        return Value::with_notes(
            Type::Reduced(Box::new(input.known.clone())),
            input.notes.clone(),
        );
    }
    let parts = match &input.known {
        Type::Union(parts) => parts.as_slice(),
        ty => std::slice::from_ref(ty),
    };
    let ty = Type::union(parts.iter().map(|ty| match name {
        "clojure.core/reduced?" => match ty {
            Type::Any | Type::Unknown | Type::Parameter(_) => Type::Boolean,
            ty => Type::Bool(matches!(ty, Type::Reduced(_))),
        },
        "clojure.core/unreduced" => match ty {
            Type::Reduced(value) => value.as_ref().clone(),
            Type::Parameter(_) => Type::Unknown,
            ty => ty.clone(),
        },
        "clojure.core/ensure-reduced" => match ty {
            Type::Reduced(_) => ty.clone(),
            Type::Parameter(_) => Type::Reduced(Box::new(Type::Unknown)),
            ty => Type::Reduced(Box::new(ty.clone())),
        },
        _ => unreachable!(),
    }));
    Value::with_notes(ty, input.notes.clone())
}
