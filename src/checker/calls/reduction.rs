//! Ordered and bounded abstract reduction of sequences.
use super::super::recur::{bounded_state, widen_growing_collections};
use super::{
    Callable, Checker, Env, PreparedCallable, Value, vectors::CollectionError,
    vectors::element_type,
};
use crate::reader::{Form, Span};
use crate::types::Type;

const MAX_REDUCE_ALTERNATIVES: usize = 16;
const MAX_REDUCE_STATES: usize = 16;
const MAX_REDUCE_STEPS: usize = 128;

struct ReductionInput<'a> {
    items: &'a [Type],
    form: &'a Form,
    notes: &'a [String],
}

impl Checker<'_> {
    pub(super) fn reduce_form(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        let (reducer, source) = match forms {
            [reducer, source] | [reducer, _, source] => (reducer, source),
            _ => {
                self.error(
                    "PNX2554",
                    span,
                    "reduce expects a function, an optional initial value and a collection",
                );
                return Value::new(Type::Unknown);
            }
        };
        let callable = self.prepare_callable(reducer, env);
        let mut inputs: Vec<_> = self
            .eval_args(&forms[1..], env, callable.returns())
            .collect();
        let (collection, _) = inputs.pop().unwrap();
        let initial = inputs.first();
        if collection.known == Type::Never {
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }
        let shapes = match &collection.known {
            Type::Union(shapes) => shapes.as_slice(),
            shape => std::slice::from_ref(shape),
        };
        if shapes.len() > MAX_REDUCE_ALTERNATIVES {
            return self.unsupported(
                source.span,
                "reduce has too many possible collection shapes",
            );
        }
        let mut result = Value::new(Type::Never);
        let start = self.diagnostics.len();
        for shape in shapes {
            let casts_reducer = matches!(shape, Type::Tuple(_) | Type::Vector(_))
                || matches!(shape, Type::SeqTuple(items) if !items.is_empty());
            if casts_reducer
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
            let items = match shape {
                Type::Nil => &[][..],
                Type::Vector(element) if **element == Type::Never => &[][..],
                Type::Tuple(items) | Type::SeqTuple(items) => items,
                Type::MapEntry(items) => items.as_slice(),
                Type::Vector(element) | Type::Seq(element) => {
                    let item =
                        Value::with_notes(element.as_ref().clone(), collection.notes.clone());
                    result = result.join(self.reduce_open(
                        &callable,
                        initial,
                        (item, source.span),
                        matches!(shape, Type::Seq(_)),
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
                notes: &collection.notes,
            };
            result = result.join(self.reduce_exact(&callable, initial, input, span, env));
        }
        self.dedup_diagnostics(start);
        result
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
                Value::with_notes(ty.clone(), input.notes.to_vec()),
                element_span,
            )
        });
        let Some((mut accumulator, mut accumulator_span)) =
            initial.cloned().or_else(|| elements.next())
        else {
            return self.call_prepared(callable, Vec::new(), span, env, true);
        };
        if elements.len() == 0 {
            self.call_prepared(callable, Vec::new(), span, env, false);
        }
        for (step, (item, item_span)) in elements.enumerate() {
            accumulator = self.reduce_step(
                callable,
                [(accumulator, accumulator_span), (item, item_span)],
                literal.is_none().then_some(input.form.span),
                span,
                env,
                step + 1,
            );
            if matches!(accumulator.known, Type::Never | Type::Unknown) {
                break;
            }
            accumulator_span = span;
        }
        accumulator
    }

    fn reduce_open(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        initial: Option<&(Value, Span)>,
        input: (Value, Span),
        nonempty: bool,
        span: Span,
        env: &Env,
    ) -> Value {
        let (mut accumulator, mut accumulator_span) =
            initial.cloned().unwrap_or_else(|| input.clone());
        let mut result = if nonempty && initial.is_some() {
            Value::new(Type::Never)
        } else {
            accumulator.clone()
        };
        if initial.is_none() && !nonempty {
            result = result.join(self.call_prepared(callable, Vec::new(), span, env, true));
        }

        let mut seen = vec![vec![accumulator.clone()]];
        for step in 1..=MAX_REDUCE_STATES {
            let mut next = self.reduce_step(
                callable,
                [(accumulator, accumulator_span), input.clone()],
                Some(input.1),
                span,
                env,
                step,
            );
            if next.known == Type::Never {
                return result;
            }
            if next.known == Type::Unknown {
                return next;
            }
            widen_growing_collections(std::slice::from_mut(&mut next), &seen);
            if !bounded_state(std::slice::from_ref(&next)) {
                break;
            }
            result = result.join(next.clone());
            if seen.iter().any(|state| state[0].known == next.known) {
                return result;
            }
            seen.push(vec![next.clone()]);
            accumulator = next;
            accumulator_span = span;
        }
        self.unsupported(
            span,
            "reduce accumulator types did not stabilize within the analysis limit",
        )
    }

    fn reduce_step(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        args: [(Value, Span); 2],
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
        let result = self.call_prepared(callable, Vec::from(args), span, env, true);
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
        | Type::Unknown
        | Type::Never
        | Type::Function(_)
        | Type::Key(_)
        | Type::Keyword
        | Type::Map(_)
        | Type::Set(_)
        | Type::Tuple(_)
        | Type::Vector(_)
        | Type::MapEntry(_) => true,
        Type::Union(parts) => parts.iter().all(reducer_argument),
        _ => false,
    }
}
