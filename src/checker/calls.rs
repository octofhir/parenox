//! Function resolution, call-site inference and built-in calls.
mod inline;
mod vectors;

use super::{Checker, Env, Value, model::Function, nil_part};
use crate::Diagnostic;
use crate::expand::Threading;
use crate::reader::{Form, Span};
use crate::types::Type;

impl Checker<'_> {
    fn inferred_call(&mut self, function: &Function, args: Vec<Value>, span: Span) -> Value {
        if args.len() != function.params.len() {
            self.error(
                "PNX2554",
                span,
                format!(
                    "{} expects {} arguments, got {}",
                    function.name,
                    function.params.len(),
                    args.len()
                ),
            );
            return Value::new(Type::Unknown);
        }
        if self.active_helpers.contains(&function.name) {
            self.error(
                "PNX9002",
                span,
                format!(
                    "Recursive private function {} needs a :t/sig annotation",
                    function.name
                ),
            );
            return Value::new(Type::Unknown);
        }
        let key = (function.name.clone(), args);
        let start = self.diagnostics.len();
        let result = if let Some((value, diagnostics)) = self.specializations.get(&key) {
            self.diagnostics.extend(diagnostics.iter().cloned());
            value.clone()
        } else {
            if self.active_helpers.len() >= 32
                || self.specializations.len() + self.active_helpers.len() >= 256
            {
                self.error("PNX9002", span,
                    "Private inference limit reached (32 nested calls or 256 specializations); add :t/sig annotations");
                return Value::new(Type::Unknown);
            }
            self.active_helpers.push(function.name.clone());
            let env =
                self.bind_parameters(Env::new(), &function.params, |index| key.1[index].clone());
            let result = self.body(&function.body, &env);
            self.active_helpers.pop();
            self.specializations
                .insert(key, (result.clone(), self.diagnostics[start..].to_vec()));
            result
        };
        for diagnostic in &mut self.diagnostics[start..] {
            diagnostic.notes.push(format!(
                "While checking call to {} at {}:{}",
                function.name, span.start.line, span.start.column
            ));
        }
        result
    }

    pub(super) fn list(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if let Some(name) = xs[0].symbol()
            && !env.contains_key(name)
            && let Some(function) = self.model.local_function(name).filter(|f| f.inferred)
        {
            let args = xs[1..].iter().map(|x| self.infer(x, env)).collect();
            return self.inferred_call(function, args, span);
        }
        if let Some(key) = xs[0].keyword() {
            if !(2..=3).contains(&xs.len()) {
                self.error(
                    "PNX2554",
                    span,
                    "Keyword lookup requires a map or set and optional default",
                );
                return Value::new(Type::Unknown);
            }
            let target = self.infer(&xs[1], env);
            let default = xs.get(2).map(|x| self.infer(x, env));
            return self.lookup(
                &target,
                &Type::Key(self.model.key(key)),
                default.as_ref(),
                xs[0].span,
            );
        }
        if let Some(name) = xs[0].symbol()
            && !env.contains_key(name)
            && self.model.signature(name, self.globals).is_none()
        {
            let resolved = self.model.resolve(name);
            if let Some(threading) = Threading::from_core_symbol(&resolved) {
                return self.thread_form(xs, span, env, threading, &resolved);
            }
            match resolved.as_str() {
                "clojure.core/as->" => return self.named_thread(xs, span, env),
                "clojure.core/let" => return self.let_form(xs, span, env),
                "clojure.core/if" | "clojure.core/when" => {
                    return self.if_form(xs, span, env, resolved.ends_with("/when"));
                }
                "clojure.core/if-let"
                | "clojure.core/when-let"
                | "clojure.core/if-some"
                | "clojure.core/when-some" => {
                    return self.binding_conditional(xs, span, env, &resolved);
                }
                "clojure.core/do" => return self.body(&xs[1..], env),

                "clojure.core/or" | "clojure.core/and" => {
                    return self.short_circuit(xs, env, resolved.ends_with("/or"));
                }
                "clojure.core/get" | "clojure.core/req!" => {
                    return self.lookup_form(xs, span, env, &resolved);
                }
                "clojure.core/contains?" => return self.contains(xs, span, env),
                "clojure.core/get-in" => return self.get_in(xs, span, env),
                "clojure.core/assoc" => return self.assoc(xs, span, env),
                "clojure.core/assoc-in" => return self.assoc_in(xs, span, env),
                "clojure.core/fn" => {
                    return self.unsupported(
                        span,
                        "fn is currently supported in direct calls and inline callbacks",
                    );
                }
                "clojure.core/recur" => {
                    return self.unsupported(span, "recur is not supported yet");
                }
                "clojure.core/fnil" => {
                    return self.unsupported(
                        span,
                        "fnil is currently supported in direct calls and inline callbacks",
                    );
                }
                "clojure.core/update" | "clojure.core/update-in" => {
                    return self.update(xs, span, env, &resolved);
                }
                "clojure.core/mapv" | "clojure.core/filterv" => {
                    return self.vector_transform(xs, span, env, &resolved);
                }
                _ if is_builtin(&resolved) => {
                    let args = self.typed_args(&xs[1..], env);
                    return self.builtin(&resolved, &args, span);
                }
                _ => {}
            }
        }
        if let Some(parts) = self.inline_callable(&xs[0], env) {
            let args = self.typed_args(&xs[1..], env);
            return self.call_inline(parts, args, xs[0].span, span, env, true);
        }
        let function = self.infer(&xs[0], env);
        let args = self.typed_args(&xs[1..], env);
        self.call_value(function, &args, xs[0].span, span)
    }

    fn typed_args(&mut self, args: &[Form], env: &Env) -> Vec<(Value, Span)> {
        args.iter()
            .map(|arg| (self.infer(arg, env), arg.span))
            .collect()
    }

    fn call_value(
        &mut self,
        function: Value,
        args: &[(Value, Span)],
        head: Span,
        span: Span,
    ) -> Value {
        match function.known {
            Type::Function(expected, ret) => {
                if args.len() != expected.len() {
                    self.error(
                        "PNX2554",
                        span,
                        format!("Expected {} arguments, got {}", expected.len(), args.len()),
                    );
                }
                for (index, ((actual, span), expected)) in args.iter().zip(&expected).enumerate() {
                    self.expect(actual, expected, *span, &format!("Argument {}", index + 1));
                }
                Value::new(*ret)
            }
            Type::Set(_) => {
                if args.len() != 1 {
                    self.error(
                        "PNX2554",
                        span,
                        format!(
                            "A set used as a function expects 1 argument, got {}",
                            args.len()
                        ),
                    );
                    Value::new(Type::Unknown)
                } else {
                    Value::with_notes(args[0].0.known.clone().nilable(), vec![
                        "A set used as a function returns the matching value or Nil, not a Boolean.".into(),
                    ])
                }
            }
            Type::Unknown | Type::Never => Value::new(function.known),
            _ => self.unsupported(head, format!("Calling {} is not supported", function.ty)),
        }
    }

    pub(super) fn call_form(
        &mut self,
        callee: &Form,
        args: Vec<(Value, Span)>,
        span: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        if let Some(parts) = self.inline_callable(callee, env) {
            return self.call_inline(parts, args, callee.span, span, env, invoked);
        }
        if let Some(name) = callee.symbol()
            && !env.contains_key(name)
        {
            if let Some(function) = self.model.local_function(name).filter(|f| f.inferred) {
                return if invoked {
                    self.inferred_call(
                        function,
                        args.into_iter().map(|(value, _)| value).collect(),
                        span,
                    )
                } else {
                    Value::new(Type::Never)
                };
            }
            if self.model.signature(name, self.globals).is_none() {
                let resolved = self.model.resolve(name);
                if is_builtin(&resolved) {
                    return if invoked {
                        self.builtin(&resolved, &args, span)
                    } else {
                        Value::new(Type::Never)
                    };
                }
            }
        }
        let function = self.infer(callee, env);
        if invoked {
            self.call_value(function, &args, callee.span, span)
        } else {
            Value::new(Type::Never)
        }
    }

    fn builtin(&mut self, name: &str, args: &[(Value, Span)], span: Span) -> Value {
        let arity_ok = match name {
            "clojure.core/+" | "clojure.core/*" | "clojure.core/str" => true,
            "clojure.core/-" | "clojure.core/=" => !args.is_empty(),
            _ => args.len() == 1,
        };
        if !arity_ok {
            self.error(
                "PNX2554",
                span,
                format!("Invalid arity for {name}: {}", args.len()),
            );
            return Value::new(Type::Unknown);
        }
        let ty = match name {
            "clojure.core/+" | "clojure.core/-" | "clojure.core/*" | "clojure.core/inc"
            | "clojure.core/dec" => {
                for (value, span) in args {
                    self.expect(value, &Type::Number, *span, name);
                }
                Type::Number
            }
            "clojure.string/upper-case" | "clojure.string/lower-case" | "clojure.string/trim" => {
                self.expect(&args[0].0, &Type::String, args[0].1, name);
                Type::String
            }
            "clojure.core/str" => Type::String,
            "clojure.core/identity" => return args[0].0.clone(),
            "clojure.core/key" | "clojure.core/val" => {
                let (entry, span) = &args[0];
                let index = usize::from(name == "clojure.core/val");
                if let Some(item) = entry.known.entry_item(index, false) {
                    return Value::with_notes(item, entry.notes.clone());
                }
                let mut diagnostic =
                    Diagnostic::type_mismatch(*span, name, "MapEntry".into(), entry.ty.to_string());
                diagnostic.notes = entry.notes.clone();
                self.diagnostics.push(diagnostic);
                Type::Unknown
            }
            "clojure.core/count" => {
                fn countable(t: &Type) -> bool {
                    match t {
                        Type::Nil
                        | Type::String
                        | Type::Str(_)
                        | Type::Map(_)
                        | Type::MapEntry(_)
                        | Type::Vector(_)
                        | Type::Set(_)
                        | Type::Never
                        | Type::Unknown => true,
                        Type::Union(xs) => xs.iter().all(countable),
                        _ => false,
                    }
                }
                if !countable(&args[0].0.ty) {
                    self.error(
                        "PNX2322",
                        args[0].1,
                        format!(
                            "count expects a collection, string or Nil, got {}",
                            args[0].0.ty
                        ),
                    );
                }
                Type::Number
            }
            "clojure.core/nil?" => {
                if args[0].0.ty == Type::Nil {
                    Type::Bool(true)
                } else if nil_part(&args[0].0.ty) == Type::Never {
                    Type::Bool(false)
                } else {
                    Type::Boolean
                }
            }
            "clojure.core/some?" => {
                if args[0].0.ty == Type::Nil {
                    Type::Bool(false)
                } else if nil_part(&args[0].0.ty) == Type::Never {
                    Type::Bool(true)
                } else {
                    Type::Boolean
                }
            }
            "clojure.core/not" => {
                if args[0].0.ty.truthy() == Type::Never {
                    Type::Bool(true)
                } else if args[0].0.ty.falsey() == Type::Never {
                    Type::Bool(false)
                } else {
                    Type::Boolean
                }
            }
            _ => predicate_type(name).map_or(Type::Boolean, |expected| {
                predicate_result(&args[0].0.known, &expected)
            }),
        };
        Value::new(ty)
    }
}

pub(super) fn predicate_type(name: &str) -> Option<Type> {
    match name {
        "clojure.core/string?" => Some(Type::String),
        "clojure.core/char?" => Some(Type::Character),
        "clojure.core/number?" => Some(Type::Number),
        "clojure.core/boolean?" => Some(Type::Boolean),
        "clojure.core/keyword?" => Some(Type::Keyword),
        _ => None,
    }
}

fn predicate_result(actual: &Type, expected: &Type) -> Type {
    match actual {
        Type::Unknown => Type::Boolean,
        Type::Never => Type::Never,
        Type::Union(parts) => {
            Type::union(parts.iter().map(|part| predicate_result(part, expected)))
        }
        actual => Type::Bool(actual.subtype_of(expected)),
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "clojure.core/+"
            | "clojure.core/-"
            | "clojure.core/*"
            | "clojure.core/inc"
            | "clojure.core/dec"
            | "clojure.core/str"
            | "clojure.core/identity"
            | "clojure.core/nil?"
            | "clojure.core/some?"
            | "clojure.core/not"
            | "clojure.core/="
            | "clojure.core/key"
            | "clojure.core/val"
            | "clojure.core/count"
            | "clojure.string/upper-case"
            | "clojure.string/lower-case"
            | "clojure.string/trim"
    ) || predicate_type(name).is_some()
}
