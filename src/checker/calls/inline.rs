//! Contextual inference for directly applied functions and fnil wrappers.
use super::super::{
    Checker, Env, Value,
    function_forms::{MAX_FIXED_PARAMETERS, function_clauses, pack_arguments},
    nil_part,
};
use super::{Callable, PreparedCallable, call_returns};
use crate::reader::{AnonymousFunction, Form, Kind, Span};
use crate::types::Type;

#[derive(Clone, Copy)]
pub(super) enum InlineFunction<'a> {
    Function(&'a [Form]),
    Literal(&'a AnonymousFunction),
}

pub(super) struct Fnil<'form, 'model> {
    target: PreparedCallable<'form, 'model>,
    defaults: Vec<(Value, Span)>,
    pub(super) returns: bool,
}

impl<'model> Checker<'model> {
    pub(super) fn prepare_inline<'form>(
        &mut self,
        form: &'form Form,
        env: &Env,
    ) -> Option<Callable<'form, 'model>> {
        let forms = match &form.without_metadata().kind {
            Kind::List(forms) => forms,
            Kind::AnonymousFunction(function) => {
                return Some(Callable::Inline(InlineFunction::Literal(function)));
            }
            _ => return None,
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
            "clojure.core/fn" => Some(Callable::Inline(InlineFunction::Function(parts))),
            "clojure.core/fnil" => Some(self.prepare_fnil(parts, form.span, env)),
            _ => None,
        }
    }

    pub(super) fn call_inline(
        &mut self,
        callable: InlineFunction<'_>,
        args: Vec<(Value, Span)>,
        constructor: Span,
        call: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        match callable {
            InlineFunction::Function(parts) => {
                self.call_inline_fn(parts, args, constructor, call, env, invoked)
            }
            InlineFunction::Literal(function) => {
                if function.required > MAX_FIXED_PARAMETERS {
                    self.error(
                        "PNX2554",
                        constructor,
                        "Clojure/JVM supports at most 20 fixed parameters",
                    );
                    return Value::new(Type::Unknown);
                }
                let clauses = literal_clause(function, constructor);
                self.call_inline_fn(&clauses, args, constructor, call, env, invoked)
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

    fn prepare_fnil<'form>(
        &mut self,
        parts: &'form [Form],
        span: Span,
        env: &Env,
    ) -> Callable<'form, 'model> {
        if !(2..=4).contains(&parts.len()) {
            self.error(
                "PNX2554",
                span,
                "fnil requires a function and one to three defaults",
            );
            return Callable::Value(Value::new(Type::Unknown));
        }
        let target = self.prepare_callable(&parts[0], env);
        let target_returns = target.returns();
        let defaults: Vec<_> = self.eval_args(&parts[1..], env, target_returns).collect();
        let returns = target_returns && call_returns(&defaults);
        Callable::Fnil(Box::new(Fnil {
            target,
            defaults,
            returns,
        }))
    }

    pub(super) fn call_fnil(
        &mut self,
        wrapper: &Fnil<'_, '_>,
        mut args: Vec<(Value, Span)>,
        call: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        // The three-default version also has a two-argument overload.
        let required = wrapper.defaults.len().min(2);
        if invoked && args.len() < required {
            self.error(
                "PNX2554",
                call,
                format!("fnil wrapper expects at least {required} arguments"),
            );
            return Value::new(Type::Unknown);
        }
        if invoked {
            for ((arg, span), (default, default_span)) in args.iter_mut().zip(&wrapper.defaults) {
                if nil_part(&arg.known) == Type::Never {
                    continue;
                }
                let non_nil = arg.known.without_nil();
                if non_nil == Type::Never {
                    *arg = default.clone();
                    *span = *default_span;
                } else {
                    arg.narrow(Type::union([non_nil, default.known.clone()]));
                    arg.notes.extend(default.notes.iter().cloned());
                }
            }
        }
        self.call_prepared(&wrapper.target, args, call, env, invoked)
    }
}

fn literal_clause(function: &AnonymousFunction, span: Span) -> [Form; 2] {
    let symbol = |name| Form {
        kind: Kind::Symbol(name),
        span,
    };
    let mut bindings: Vec<_> = (1..=function.required)
        .map(|index| symbol(format!("%{index}")))
        .collect();
    if function.variadic {
        bindings.extend([symbol("&".into()), symbol("%&".into())]);
    }
    [
        Form {
            kind: Kind::Vector(bindings),
            span,
        },
        Form {
            kind: Kind::List(function.body.clone()),
            span,
        },
    ]
}
