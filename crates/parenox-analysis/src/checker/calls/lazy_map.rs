//! Deferred callbacks for clojure.core/map.
use super::{
    Checker, Env, PreparedCallable, Value, call_returns,
    sequences::{definitely_not_seqable, nonempty_sequence},
    vectors::element_type,
};
use crate::reader::{Form, Span};
use crate::types::{LengthBounds, Type};

impl Checker<'_> {
    pub(super) fn lazy_map(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        let [head, callback, sources @ ..] = forms else {
            self.error("PNX2554", span, "map expects at least a callback");
            self.arity_error();
            return Value::new(Type::Never);
        };
        if !self
            .model
            .can_use_standard_core(head.symbol().unwrap(), self.globals)
        {
            return self.unchecked_form(head.span, "Standard clojure.core/map is unconfirmed");
        }
        self.with_callback_choice(callback, env, |checker, selected, scope| {
            let callable = checker.prepare_callable(selected, scope);
            let inputs = checker
                .eval_args(sources, scope, callable.returns())
                .collect::<Vec<_>>();
            if !callable.returns() || !call_returns(&inputs) {
                checker.call_prepared(&callable, Vec::new(), span, scope, false);
                return Value::new(Type::Never);
            }
            if inputs.is_empty() {
                checker.call_prepared(&callable, Vec::new(), span, scope, false);
                return checker.unchecked_form(
                    span,
                    "Type analysis for an unapplied map transducer is unavailable",
                );
            }
            // Constructing the lazy sequence evaluates the operands, but does
            // not traverse them or invoke the callback.
            let enclosing = checker.throws.take();
            let result = checker.lazy_map_items(&callable, inputs, span, scope);
            checker.throws = enclosing;
            result
        })
    }

    fn lazy_map_items(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        inputs: Vec<(Value, Span)>,
        span: Span,
        env: &Env,
    ) -> Value {
        let mut lengths = LengthBounds {
            min: usize::MAX,
            max: None,
        };
        let mut lengths_known = true;
        let mut unsupported = false;
        let args = inputs
            .into_iter()
            .map(|(input, source)| {
                if input.unproven_tuple_traversal() {
                    unsupported = true;
                    return (self.unchecked_tuple_traversal("map", source), source);
                }
                let seq = self.map_source_seq(&input.known, source);
                lengths_known &= !matches!(seq, Type::Never | Type::Unknown);
                let source_lengths = input.known.length_bounds();
                let traversal_lengths = seq.length_bounds();
                lengths.min = lengths
                    .min
                    .min(source_lengths.min.max(traversal_lengths.min));
                lengths.max = lengths
                    .max
                    .into_iter()
                    .chain(source_lengths.max)
                    .chain(traversal_lengths.max)
                    .min();
                let element = element_type(seq)
                    .unwrap_or_else(|_| unreachable!("seq_type returns sequence shapes"));
                (input.traversal_element(element), source)
            })
            .collect::<Vec<_>>();
        let can_invoke = !unsupported && args.iter().all(|(arg, _)| arg.known != Type::Never);
        if unsupported {
            self.withhold_callback_facts(callable, env);
        }
        let item = self.call_prepared(callable, args, span, env, can_invoke);
        if lengths_known && !unsupported && lengths.max == Some(0) {
            return Value::new(Type::empty_seq());
        }
        let element = if unsupported {
            Type::Unknown
        } else {
            item.known
        };
        let lengths = if !lengths_known || unsupported || element == Type::Never {
            LengthBounds::UNBOUNDED
        } else {
            lengths
        };
        // A completed traversal emits one value per step up to the shortest input.
        let results = nonempty_sequence(element, lengths);
        let ty = if lengths.min == 0 {
            Type::union([Type::empty_seq(), results])
        } else {
            results
        };
        Value::with_notes(ty, item.notes)
    }

    fn map_source_seq(&mut self, ty: &Type, source: Span) -> Type {
        match ty {
            Type::Union(parts) => {
                Type::union(parts.iter().map(|part| self.map_source_seq(part, source)))
            }
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.map_source_seq(upper, source)
            }
            Type::Nil
            | Type::Never
            | Type::Unknown
            | Type::Any
            | Type::SpecCollection(_)
            | Type::BoundedCollection(..)
            | Type::Tuple(_)
            | Type::Vector(_)
            | Type::ListTuple(_)
            | Type::List(_)
            | Type::SeqTuple(_)
            | Type::Seq(_)
            | Type::MapEntry(_)
            | Type::Map(_)
            | Type::Set(_)
            | Type::String
            | Type::Str(_) => self.seq_type(ty, "map", source),
            other if definitely_not_seqable(other) => self.seq_type(ty, "map", source),
            _ => {
                self.unchecked_form(source, "map traversal for this source type is unchecked")
                    .known
            }
        }
    }
}
