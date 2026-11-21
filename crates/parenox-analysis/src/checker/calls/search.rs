//! Eager search over known sequence shapes.
use super::{Checker, Env, PreparedCallable, Value, sequences::iteration_may_throw};
use crate::reader::{Form, Span};
use crate::types::Type;

const SOME: &str = "clojure.core/some";
const MAX_SEARCH_STEPS: usize = 128;

impl Checker<'_> {
    pub(super) fn some_form(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        if forms.len() != 2 {
            self.error("PNX2554", span, "some expects a predicate and a collection");
        }
        let Some((predicate, inputs)) = forms.split_first() else {
            self.arity_error();
            return Value::new(Type::Never);
        };
        self.with_callback_choice(predicate, env, |checker, selected, scope| {
            let callback = checker.prepare_callable(selected, scope);
            if let [source] = inputs {
                return checker.some_using(&callback, source, span, scope);
            }
            let returns = checker
                .eval_args(inputs, scope, callback.returns())
                .last()
                .map_or_else(
                    || callback.returns(),
                    |(value, _)| value.known != Type::Never,
                );
            checker.call_prepared(&callback, Vec::new(), span, scope, false);
            if returns {
                checker.arity_error();
            }
            Value::new(Type::Never)
        })
    }

    fn some_using(
        &mut self,
        callback: &PreparedCallable<'_, '_>,
        source: &Form,
        span: Span,
        env: &Env,
    ) -> Value {
        let collection = self.infer_path(source, env, callback.returns(), false);
        if !self.path_reachable || collection.known == Type::Never {
            self.call_prepared(callback, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }
        if collection.known == Type::Any {
            return self.unchecked_search(
                callback,
                source.span,
                span,
                env,
                "some cannot infer elements of a collection typed Any",
            );
        }
        let start = self.diagnostics.len();
        if iteration_may_throw(&collection.known) {
            self.may_throw();
        }
        let sequence = self.seq_type(&collection.known, SOME, source.span);
        self.note_builtin_contract(start, SOME);

        let shapes = match &sequence {
            Type::Union(shapes) => shapes.as_ref(),
            shape => std::slice::from_ref(shape),
        };
        let mut invoked = false;
        let mut result = Value::new(Type::Never);
        for shape in shapes {
            let found = match shape {
                Type::SeqTuple(items) | Type::ListTuple(items) => {
                    invoked |= !items.is_empty();
                    let literal = source.vector().filter(|forms| forms.len() == items.len());
                    let items = items.iter().enumerate().map(|(index, item)| {
                        let arg = literal.map_or(source.span, |forms| forms[index].span);
                        (item, arg)
                    });
                    self.search_items(callback, items, &collection.notes, span, env)
                }
                Type::Seq(item) | Type::List(item) => {
                    invoked = true;
                    // One abstract element represents every position. A falsey
                    // result can continue to the end without finding a match.
                    self.search_items(
                        callback,
                        [(item.as_ref(), source.span)],
                        &collection.notes,
                        span,
                        env,
                    )
                }
                Type::Unknown => {
                    // This shape could invoke a saved callback outside the
                    // call's source range with inputs we have not checked.
                    self.type_query = None;
                    Value::new(Type::Unknown)
                }
                Type::Nil | Type::Never => Value::new(shape.clone()),
                _ => unreachable!("sequence start produced {shape:?}"),
            };
            result = result.join(found);
        }
        if !invoked {
            self.call_prepared(callback, Vec::new(), span, env, false);
        }
        self.dedup_diagnostics(start);
        result
    }

    fn search_items<'item>(
        &mut self,
        callback: &PreparedCallable<'_, '_>,
        items: impl IntoIterator<Item = (&'item Type, Span)>,
        notes: &[String],
        span: Span,
        env: &Env,
    ) -> Value {
        let mut found = Value::new(Type::Never);
        for (item, argument) in items {
            if self.search_steps >= MAX_SEARCH_STEPS {
                return found.join(self.unchecked_search(
                    callback,
                    span,
                    span,
                    env,
                    &format!("some step analysis limit ({MAX_SEARCH_STEPS}) exceeded"),
                ));
            }
            self.search_steps += 1;
            let item = Value::with_notes(item.clone(), notes.to_vec());
            let mut matched = self.call_prepared(callback, vec![(item, argument)], span, env, true);
            let continues = matched.known.falsey() != Type::Never;
            matched.narrow(matched.known.truthy());
            found = found.join(matched);
            if !continues {
                return found;
            }
        }
        found.join(Value::new(Type::Nil))
    }

    fn unchecked_search(
        &mut self,
        callback: &PreparedCallable<'_, '_>,
        source: Span,
        call: Span,
        env: &Env,
        reason: &str,
    ) -> Value {
        self.type_query = None;
        self.call_prepared(callback, Vec::new(), call, env, false);
        self.unchecked_form(source, reason)
    }
}
