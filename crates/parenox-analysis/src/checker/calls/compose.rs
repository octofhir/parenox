//! Preparation and evaluation of direct comp calls and comp callbacks.
use super::{Callable, Checker, Env, PreparedCallable, Value};
use crate::reader::{Form, Span};
use crate::types::Type;

pub(super) struct Composition<'form, 'model> {
    stages: Vec<PreparedCallable<'form, 'model>>,
    pub(super) returns: bool,
}

impl<'model> Checker<'model> {
    pub(super) fn prepare_composition<'form>(
        &mut self,
        forms: &'form [Form],
        span: Span,
        env: &Env,
    ) -> PreparedCallable<'form, 'model> {
        let kind = match forms {
            [] => Callable::Builtin("clojure.core/identity".into()),
            [target] => return self.prepare_callable(target, env),
            _ => {
                let mut returns = true;
                let mut stages = Vec::with_capacity(forms.len());
                for stage in forms {
                    let prepared =
                        self.inspect_path(returns, |checker| checker.prepare_callable(stage, env));
                    returns &= prepared.returns();
                    stages.push(prepared);
                }
                Callable::Composition(Composition { stages, returns })
            }
        };
        PreparedCallable { kind, span }
    }

    pub(super) fn call_composition(
        &mut self,
        composition: &Composition<'_, '_>,
        args: Vec<(Value, Span)>,
        call: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        let mut stages = composition.stages.iter().rev();
        let rightmost = stages
            .next()
            .expect("a composition has at least two stages");
        let mut result = self.call_prepared(rightmost, args, call, env, invoked);
        let mut producer = rightmost.span;
        // The rightmost function receives all the arguments. Each earlier
        // function receives exactly one value, even when it is nil or false.
        for stage in stages {
            let reached = invoked && result.known != Type::Never;
            result = self.call_prepared(stage, vec![(result, producer)], call, env, reached);
            producer = stage.span;
        }
        result
    }
}
