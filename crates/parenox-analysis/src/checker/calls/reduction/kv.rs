//! Reductions that pass an accumulator, key and value.
use super::{
    Callable, Checker, Env, MAX_REDUCE_ALTERNATIVES, MAX_REDUCE_STEPS, PreparedCallable, Value,
    bounded_state, reducer_argument,
};
use crate::reader::{Form, Span};
use crate::types::Type;

struct KvCall<'a, 'form, 'model> {
    callable: &'a PreparedCallable<'form, 'model>,
    initial: (Value, Span),
    source: &'a Form,
    notes: &'a [String],
    span: Span,
    env: &'a Env,
}

#[derive(Clone, Copy)]
enum Items {
    Entries,
    Indexed,
}

impl Checker<'_> {
    pub(in crate::checker::calls) fn reduce_kv_form(
        &mut self,
        forms: &[Form],
        span: Span,
        env: &Env,
    ) -> Value {
        let [reducer, _, source] = forms else {
            self.error(
                "PNX2554",
                span,
                "reduce-kv expects a reducer, an initial value and a collection",
            );
            return Value::new(Type::Unknown);
        };
        let callable = self.prepare_callable(reducer, env);
        let (initial, (collection, _)) = {
            let mut inputs = self.eval_args(&forms[1..], env, callable.returns());
            (inputs.next().unwrap(), inputs.next().unwrap())
        };
        if collection.known == Type::Never {
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }
        if collection.unproven_tuple_traversal() {
            self.withhold_callback_facts(&callable);
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return self.unchecked_tuple_traversal("clojure.core/reduce-kv", source.span);
        }
        let call = KvCall {
            callable: &callable,
            initial,
            source,
            notes: &collection.notes,
            span,
            env,
        };
        let start = self.diagnostics.len();
        let result = self.reduce_kv_type(&collection.known, &call);
        self.dedup_diagnostics(start);
        result
    }

    fn reduce_kv_type(&mut self, ty: &Type, call: &KvCall<'_, '_, '_>) -> Value {
        match ty {
            Type::Nil => self.reduce_kv_empty(call),
            Type::Union(parts) if parts.len() <= MAX_REDUCE_ALTERNATIVES => {
                let results = parts.iter().map(|part| self.reduce_kv_type(part, call));
                results.fold(Value::new(Type::Never), Value::join)
            }
            Type::Union(_) => self.unsupported(
                call.source.span,
                "reduce-kv has too many possible collection shapes",
            ),
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.reduce_kv_type(upper, call)
            }
            Type::Map(_) | Type::Tuple(_) | Type::Vector(_) => {
                // Native maps and vectors cast the reducer even when empty.
                if let Callable::Value(function) = &call.callable.kind
                    && !reducer_argument(&function.known)
                {
                    self.error(
                        "PNX2322",
                        call.callable.span,
                        format!("reduce-kv expects a callable or Nil, got {}", function.ty),
                    );
                    self.class_cast();
                    return Value::new(Type::Never);
                }
                if matches!(ty, Type::Map(_)) {
                    let entries = self.seq_type(ty, "reduce-kv", call.source.span);
                    self.reduce_kv_items(&entries, Items::Entries, call)
                } else {
                    self.reduce_kv_items(ty, Items::Indexed, call)
                }
            }
            Type::Any | Type::Unknown => {
                self.may_throw();
                self.call_prepared(call.callable, Vec::new(), call.span, call.env, false);
                Value::new(Type::Unknown)
            }
            Type::MapEntry(_)
            | Type::Seq(_)
            | Type::SeqTuple(_)
            | Type::List(_)
            | Type::ListTuple(_)
            | Type::Set(_)
            | Type::Str(_)
            | Type::String => self.unsupported(
                call.source.span,
                "reduce-kv analysis currently supports maps, vectors and Nil",
            ),
            _ => {
                self.error(
                    "PNX2322",
                    call.source.span,
                    format!(
                        "reduce-kv expects a map, vector or Nil, got {}",
                        ty.inferred()
                    ),
                );
                self.illegal_argument();
                Value::new(Type::Never)
            }
        }
    }

    fn reduce_kv_items(&mut self, ty: &Type, items: Items, call: &KvCall<'_, '_, '_>) -> Value {
        match ty {
            Type::Nil => self.reduce_kv_empty(call),
            Type::Vector(item) if **item == Type::Never => self.reduce_kv_empty(call),
            Type::Union(parts) => parts
                .iter()
                .map(|part| self.reduce_kv_items(part, items, call))
                .fold(Value::new(Type::Never), Value::join),
            Type::Tuple(values) | Type::SeqTuple(values) | Type::ListTuple(values) => {
                if values.is_empty() {
                    return self.reduce_kv_empty(call);
                }
                let literal = call
                    .source
                    .vector()
                    .filter(|forms| forms.len() == values.len());
                self.reduce_exact_with(
                    call.initial.clone(),
                    values.iter(),
                    call.span,
                    |checker, accumulator, value, step| {
                        let item_span =
                            literal.map_or(call.source.span, |forms| forms[step - 1].span);
                        let index = matches!(items, Items::Indexed).then(|| {
                            Type::Integer(i32::try_from(step - 1).expect("fixed vector length"))
                        });
                        checker.reduce_kv_step(call, accumulator, value, item_span, index, step)
                    },
                )
            }
            Type::Vector(value) | Type::Seq(value) | Type::List(value) => {
                let result = if matches!(ty, Type::Seq(_) | Type::List(_)) {
                    Value::new(Type::Never)
                } else {
                    call.initial.0.clone()
                };
                self.reduce_open_with(
                    call.initial.clone(),
                    result,
                    "reduce-kv",
                    call.span,
                    |checker, accumulator, step| {
                        let index = matches!(items, Items::Indexed).then_some(Type::Number);
                        checker.reduce_kv_step(
                            call,
                            accumulator,
                            value,
                            call.source.span,
                            index,
                            step,
                        )
                    },
                )
            }
            Type::Unknown => Value::new(Type::Unknown),
            _ => unreachable!("a modeled reduction source supplies indexed values or map entries"),
        }
    }

    fn reduce_kv_empty(&mut self, call: &KvCall<'_, '_, '_>) -> Value {
        self.call_prepared(call.callable, Vec::new(), call.span, call.env, false);
        call.initial.0.clone()
    }

    fn reduce_kv_step(
        &mut self,
        call: &KvCall<'_, '_, '_>,
        accumulator: (Value, Span),
        item: &Type,
        item_span: Span,
        index: Option<Type>,
        step: usize,
    ) -> Value {
        if !bounded_state(std::slice::from_ref(&accumulator.0)) {
            return self.unsupported(
                call.span,
                "reduce-kv accumulator shape exceeds the analysis limit",
            );
        }
        let invoke = |checker: &mut Self, accumulator, key, value: &Type| {
            let key = (Value::new(key), call.source.span);
            let value = (
                Value::with_notes(value.clone(), call.notes.to_vec()),
                item_span,
            );
            checker.reduce_step(
                call.callable,
                [accumulator, key, value],
                Some(call.source.span),
                call.span,
                call.env,
                step,
            )
        };
        if let Some(index) = index {
            return invoke(self, accumulator, index, item);
        }
        let entries = match item {
            Type::Union(parts) => parts.as_slice(),
            _ => std::slice::from_ref(item),
        };
        // Invoke each pair together. Combining keys and values independently
        // would introduce impossible arguments for a heterogeneous map.
        let mut result = Value::new(Type::Never);
        for entry in entries {
            let Type::MapEntry(pair) = entry else {
                unreachable!("a map sequence contains entries");
            };
            let [key, value] = pair.as_ref();
            let next = invoke(self, accumulator.clone(), key.clone(), value);
            if next.known == Type::Unknown && self.reduction_steps >= MAX_REDUCE_STEPS {
                return next;
            }
            result = result.join(next);
        }
        result
    }
}
