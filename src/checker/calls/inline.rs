//! Contextual inference for directly applied functions and fnil wrappers.
use super::super::{
    Checker, Env, Value,
    function_forms::{function_clauses, pack_arguments},
    nil_part,
};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

pub(super) enum InlineCallable<'a> {
    Function(&'a [Form]),
    Fnil(&'a [Form]),
}

impl Checker<'_> {
    pub(super) fn inline_callable<'a>(
        &self,
        form: &'a Form,
        env: &Env,
    ) -> Option<InlineCallable<'a>> {
        let Kind::List(forms) = &form.without_metadata().kind else {
            return None;
        };
        let (head, parts) = forms.split_first()?;
        let name = head.symbol()?;
        if env.contains_key(name)
            || self.model.local_function(name).is_some()
            || self.model.declared_value(name, self.globals).is_some()
        {
            return None;
        }
        match self.model.resolve(name).as_str() {
            "clojure.core/fn" => Some(InlineCallable::Function(parts)),
            "clojure.core/fnil" => Some(InlineCallable::Fnil(parts)),
            _ => None,
        }
    }

    pub(super) fn call_inline(
        &mut self,
        callable: InlineCallable<'_>,
        args: Vec<(Value, Span)>,
        constructor: Span,
        call: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        match callable {
            InlineCallable::Function(parts) => {
                self.call_inline_fn(parts, args, constructor, call, env, invoked)
            }
            InlineCallable::Fnil(parts) => {
                self.call_fnil(parts, args, constructor, call, env, invoked)
            }
        }
    }

    fn call_inline_fn(
        &mut self,
        parts: &[Form],
        args: Vec<(Value, Span)>,
        constructor: Span,
        call: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        if let Some(name) = parts.first().filter(|form| form.symbol().is_some()) {
            return self.unsupported(name.span, "Named fn forms are not supported yet");
        }
        let clauses = match function_clauses(parts, constructor) {
            Ok(clauses) => clauses,
            Err(diagnostic) => {
                self.diagnostics.push(*diagnostic);
                return Value::new(Type::Unknown);
            }
        };
        let selected_arity = clauses
            .keys()
            .copied()
            .find(|arity| arity.accepts(args.len()));
        let missing_arity = invoked && selected_arity.is_none();
        if missing_arity {
            self.error(
                "PNX2554",
                call,
                format!(
                    "fn expects {} arguments, got {}",
                    super::arity_counts(clauses.keys().copied()),
                    args.len()
                ),
            );
        }
        let start = self.diagnostics.len();
        let mut result = Value::new(if missing_arity {
            Type::Unknown
        } else {
            Type::Never
        });
        for (arity, clause) in clauses {
            let selected = invoked && Some(arity) == selected_arity;
            let checked = self.check_path(selected, |checker| {
                let parameters = if selected {
                    pack_arguments(arity, args.iter().map(|(value, _)| value.clone()).collect())
                } else {
                    vec![Value::new(Type::Never); arity.bindings()]
                };
                checker.function_body(
                    &clause.bindings(),
                    clause.body,
                    parameters,
                    env,
                    clause.span,
                )
            });
            if selected {
                result = checked;
            }
        }
        for diagnostic in &mut self.diagnostics[start..] {
            diagnostic.notes.push(format!(
                "While checking inline fn called at {}:{}",
                call.start.line, call.start.column
            ));
        }
        result
    }

    fn call_fnil(
        &mut self,
        parts: &[Form],
        mut args: Vec<(Value, Span)>,
        constructor: Span,
        call: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        self.may_throw();
        if !(2..=4).contains(&parts.len()) {
            self.error(
                "PNX2554",
                constructor,
                "fnil requires a function and one to three defaults",
            );
            return Value::new(Type::Unknown);
        }
        let defaults = self.typed_args(&parts[1..], env);
        // The three-default version also has a two-argument overload.
        if invoked && args.len() < defaults.len().min(2) {
            self.error(
                "PNX2554",
                call,
                format!(
                    "fnil wrapper expects at least {} arguments",
                    defaults.len().min(2)
                ),
            );
            return Value::new(Type::Unknown);
        }
        for ((arg, span), (default, default_span)) in args.iter_mut().zip(defaults) {
            if nil_part(&arg.known) == Type::Never {
                continue;
            }
            let non_nil = arg.known.without_nil();
            if non_nil == Type::Never {
                *arg = default;
                *span = default_span;
            } else {
                arg.narrow(Type::union([non_nil, default.known]));
                arg.notes.extend(default.notes);
            }
        }
        self.call_form(&parts[0], args, call, env, invoked)
    }
}
