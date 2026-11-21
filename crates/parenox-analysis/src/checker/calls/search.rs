//! Eager search over known sequence shapes.
use super::{Checker, Env, PreparedCallable, Value, sequences::iteration_may_throw};
use crate::reader::{Form, Span};
use crate::types::Type;

const MAX_SEARCH_STEPS: usize = 128;

#[derive(Clone, Copy)]
pub(super) enum Search {
    Some,
    Every,
}

impl Search {
    fn qualified(self) -> &'static str {
        match self {
            Self::Some => "clojure.core/some",
            Self::Every => "clojure.core/every?",
        }
    }

    fn name(self) -> &'static str {
        &self.qualified()["clojure.core/".len()..]
    }

    fn exhausted(self) -> Value {
        Value::new(match self {
            Self::Some => Type::Nil,
            Self::Every => Type::Bool(true),
        })
    }

    fn unchecked_result(self) -> Value {
        Value::new(match self {
            Self::Some => Type::Unknown,
            Self::Every => Type::Boolean,
        })
    }

    fn step(self, mut value: Value) -> (Value, bool) {
        let truthy = value.known.truthy();
        let can_fail = value.known.falsey() != Type::Never;
        match self {
            Self::Some => {
                value.narrow(truthy);
                (value, can_fail)
            }
            Self::Every => {
                // Rejection terminates the walk with false. Only an
                // uninterrupted sequence of truthy results can return true.
                let rejected = if can_fail {
                    Type::Bool(false)
                } else {
                    Type::Never
                };
                (Value::new(rejected), truthy != Type::Never)
            }
        }
    }
}

impl Checker<'_> {
    pub(super) fn search_form(
        &mut self,
        search: Search,
        forms: &[Form],
        span: Span,
        env: &Env,
    ) -> Value {
        if forms.len() != 2 {
            self.error(
                "PNX2554",
                span,
                format!("{} expects a predicate and a collection", search.name()),
            );
        }
        let Some((predicate, inputs)) = forms.split_first() else {
            self.arity_error();
            return Value::new(Type::Never);
        };
        self.with_callback_choice(predicate, env, |checker, selected, scope| {
            let callback = checker.prepare_callable(selected, scope);
            if let [source] = inputs {
                return checker.search_using(search, &callback, source, span, scope);
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

    fn search_using(
        &mut self,
        search: Search,
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
            self.unchecked_search(
                callback,
                source.span,
                span,
                env,
                &format!(
                    "{} cannot infer elements of a collection typed Any",
                    search.name()
                ),
            );
            return search.unchecked_result();
        }
        let start = self.diagnostics.len();
        if iteration_may_throw(&collection.known) {
            self.may_throw();
        }
        let sequence = self.seq_type(&collection.known, search.qualified(), source.span);
        self.note_builtin_contract(start, search.qualified());

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
                    self.search_items(search, callback, items, &collection.notes, span, env)
                }
                Type::Seq(item) | Type::List(item) => {
                    invoked = true;
                    // One abstract element represents every position. If its
                    // callback can continue, the walk can reach the end.
                    self.search_items(
                        search,
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
                    search.unchecked_result()
                }
                Type::Nil => search.exhausted(),
                Type::Never => Value::new(Type::Never),
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
        search: Search,
        callback: &PreparedCallable<'_, '_>,
        items: impl IntoIterator<Item = (&'item Type, Span)>,
        notes: &[String],
        span: Span,
        env: &Env,
    ) -> Value {
        let mut found = Value::new(Type::Never);
        for (item, argument) in items {
            if self.search_steps >= MAX_SEARCH_STEPS {
                self.unchecked_search(
                    callback,
                    span,
                    span,
                    env,
                    &format!(
                        "{} step analysis limit ({MAX_SEARCH_STEPS}) exceeded",
                        search.name()
                    ),
                );
                return found.join(search.unchecked_result());
            }
            self.search_steps += 1;
            let item = Value::with_notes(item.clone(), notes.to_vec());
            let value = self.call_prepared(callback, vec![(item, argument)], span, env, true);
            let (matched, continues) = search.step(value);
            found = found.join(matched);
            if !continues {
                return found;
            }
        }
        found.join(search.exhausted())
    }

    fn unchecked_search(
        &mut self,
        callback: &PreparedCallable<'_, '_>,
        source: Span,
        call: Span,
        env: &Env,
        reason: &str,
    ) {
        self.type_query = None;
        self.call_prepared(callback, Vec::new(), call, env, false);
        self.unchecked_form(source, reason);
    }
}
