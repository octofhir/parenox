//! Preparation and invocation of comp and juxt.
use super::super::literal_vector_value;
use super::{Callable, Checker, Env, PreparedCallable, Value};
use crate::reader::{Form, Span};
use crate::types::Type;

#[derive(Clone, Copy)]
pub(super) enum Combiner {
    Comp,
    Juxt,
}

pub(super) struct CombinedCallable<'form, 'model> {
    combiner: Combiner,
    pub(super) stages: Vec<PreparedCallable<'form, 'model>>,
    pub(super) returns: bool,
}

impl<'model> Checker<'model> {
    pub(super) fn prepare_combiner<'form>(
        &mut self,
        combiner: Combiner,
        forms: &'form [Form],
        span: Span,
        env: &Env,
    ) -> PreparedCallable<'form, 'model> {
        let kind = match (combiner, forms) {
            (Combiner::Comp, []) => Callable::Builtin("clojure.core/identity".into()),
            (Combiner::Comp, [target]) => return self.prepare_callable(target, env),
            (Combiner::Juxt, []) => {
                self.error("PNX2554", span, "juxt requires at least one function");
                self.arity_error();
                Callable::Value(Value::new(Type::Never))
            }
            _ => {
                let mut returns = true;
                let mut stages = Vec::with_capacity(forms.len());
                for stage in forms {
                    let prepared =
                        self.inspect_path(returns, |checker| checker.prepare_callable(stage, env));
                    returns &= prepared.returns();
                    stages.push(prepared);
                }
                Callable::Combined(CombinedCallable {
                    combiner,
                    stages,
                    returns,
                })
            }
        };
        PreparedCallable::new(kind, span)
    }

    pub(super) fn call_combiner(
        &mut self,
        callable: &CombinedCallable<'_, '_>,
        args: Vec<(Value, Span)>,
        call: Span,
        env: &Env,
        mut invoked: bool,
    ) -> Value {
        match callable.combiner {
            Combiner::Comp => {
                let mut stages = callable.stages.iter().rev();
                let rightmost = stages
                    .next()
                    .expect("a composition has at least two stages");
                let mut result = self.call_prepared(rightmost, args, call, env, invoked);
                let mut producer = rightmost.span;
                // The rightmost function receives all arguments. Each
                // earlier function receives one value, including nil or false.
                for stage in stages {
                    let reached = invoked && result.known != Type::Never;
                    result =
                        self.call_prepared(stage, vec![(result, producer)], call, env, reached);
                    producer = stage.span;
                }
                result
            }
            Combiner::Juxt => {
                let values = callable.stages.iter().map(|stage| {
                    let inputs = if invoked { args.clone() } else { Vec::new() };
                    let result = self.call_prepared(stage, inputs, call, env, invoked);
                    invoked &= result.known != Type::Never;
                    result
                });
                literal_vector_value(values, callable.stages.len(), true)
            }
        }
    }
}
