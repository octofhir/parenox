//! Contextual inference for directly applied functions and fnil wrappers.
use super::super::{Checker, Env, Value, nil_part};
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
            || self.model.signature(name, self.globals).is_some()
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
    ) -> Value {
        match callable {
            InlineCallable::Function(parts) => {
                self.call_inline_fn(parts, args, constructor, call, env)
            }
            InlineCallable::Fnil(parts) => self.call_fnil(parts, args, constructor, call, env),
        }
    }

    fn call_inline_fn(
        &mut self,
        parts: &[Form],
        args: Vec<(Value, Span)>,
        constructor: Span,
        call: Span,
        env: &Env,
    ) -> Value {
        let Some((params_form, body)) = parts.split_first() else {
            self.error("PNX1001", constructor, "fn requires a parameter vector");
            return Value::new(Type::Unknown);
        };
        let Some(params) = params_form.vector() else {
            if matches!(
                params_form.without_metadata().kind,
                Kind::List(_) | Kind::Symbol(_)
            ) {
                return self.unsupported(
                    params_form.span,
                    "named and multi-arity fn forms are not supported yet",
                );
            }
            self.error(
                "PNX1001",
                params_form.span,
                "fn requires a parameter vector",
            );
            return Value::new(Type::Unknown);
        };
        for param in params {
            match &param.without_metadata().kind {
                Kind::Symbol(name) if name == "&" => {
                    return self.unsupported(param.span, "variadic fn forms are not supported yet");
                }
                Kind::Symbol(name) if !name.contains('/') => {}
                Kind::Map(_) | Kind::Vector(_) => {}
                _ => {
                    self.error("PNX1001", param.span, "Invalid fn parameter binding");
                    return Value::new(Type::Unknown);
                }
            }
        }
        // In a multi-expression body a leading map is the condition map.
        let body = if let [first, rest @ ..] = body
            && !rest.is_empty()
            && let Some(entries) = first.map()
        {
            if entries
                .as_chunks::<2>()
                .0
                .iter()
                .any(|[key, _]| matches!(key.keyword(), Some(":pre" | ":post")))
            {
                return self.unsupported(
                    first.span,
                    "fn preconditions and postconditions are not supported yet",
                );
            }
            rest
        } else {
            if [":pre", ":post"]
                .into_iter()
                .any(|key| params_form.metadata(key).is_some())
            {
                return self.unsupported(
                    params_form.span,
                    "fn preconditions and postconditions are not supported yet",
                );
            }
            body
        };
        if args.len() != params.len() {
            self.error(
                "PNX2554",
                call,
                format!("fn expects {} arguments, got {}", params.len(), args.len()),
            );
            return Value::new(Type::Unknown);
        }
        let start = self.diagnostics.len();
        let local = self.bind_parameters(env.clone(), params, |index| args[index].0.clone());
        let result = self.body(body, &local);
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
    ) -> Value {
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
        if args.len() < defaults.len().min(2) {
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
        self.call_form(&parts[0], args, call, env)
    }
}
