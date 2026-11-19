//! Narrow contracts for instance methods on a known JVM String receiver.
use super::{Checker, Env, Form, Span, Type, Value, call_returns};
use crate::reader::Kind;

#[derive(Clone, Copy)]
enum StringMethod {
    Length,
    CharAt,
}

impl StringMethod {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "length" => Some(Self::Length),
            "charAt" => Some(Self::CharAt),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Length => "java.lang.String/length",
            Self::CharAt => "java.lang.String/charAt",
        }
    }

    fn arity(self) -> usize {
        match self {
            Self::Length => 0,
            Self::CharAt => 1,
        }
    }
}

impl Checker<'_> {
    pub(super) fn jvm_string_call(
        &mut self,
        head: &str,
        args: &[Form],
        span: Span,
        env: &Env,
    ) -> Option<Value> {
        let (method, receiver, args) = if head == "." {
            let (receiver, name, args) = dot_form(args)?;
            (StringMethod::parse(name)?, receiver, args)
        } else {
            let method = StringMethod::parse(head.strip_prefix('.')?)?;
            if env.contains_key(head)
                || self.model.local_function(head).is_some()
                || self.model.declared_value(head, self.globals).is_some()
            {
                return None;
            }
            let Some((receiver, args)) = args.split_first() else {
                self.error("PNX1001", span, format!("{head} requires a receiver"));
                return Some(Value::new(Type::Unknown));
            };
            (method, receiver, args)
        };

        let receiver_type = self.infer(receiver, env);
        let typed: Vec<_> = self
            .eval_args(args, env, receiver_type.known != Type::Never)
            .collect();
        if receiver_type.known == Type::Never || !call_returns(&typed) {
            return Some(Value::new(Type::Never));
        }
        if !receiver_type.known.subtype_of(&Type::String.nilable()) {
            return Some(
                self.unchecked_form(span, "JVM member analysis requires a known String receiver"),
            );
        }
        let start = self.diagnostics.len();
        let result = self.check_string_method(
            method,
            &receiver_type,
            receiver.without_metadata().span,
            &typed,
            span,
        );
        self.note_builtin_contract(start, method.name());
        Some(Value::new(result))
    }

    fn check_string_method(
        &mut self,
        method: StringMethod,
        receiver: &Value,
        receiver_span: Span,
        args: &[(Value, Span)],
        span: Span,
    ) -> Type {
        if args.len() != method.arity() {
            self.error(
                "PNX2554",
                span,
                format!(
                    "{} expects {} member arguments, got {}",
                    method.name(),
                    method.arity(),
                    args.len()
                ),
            );
            self.may_throw();
            return Type::Unknown;
        }
        self.expect(receiver, &Type::String, receiver_span, method.name());
        if !receiver.known.subtype_of(&Type::String) {
            self.may_throw();
            if receiver.known == Type::Nil {
                return Type::Never;
            }
        }
        match method {
            StringMethod::Length => match &receiver.known {
                Type::Str(text) => {
                    i32::try_from(text.encode_utf16().count()).map_or(Type::Number, Type::Integer)
                }
                _ => Type::Number,
            },
            StringMethod::CharAt => {
                self.expect(&args[0].0, &Type::Number, args[0].1, "String.charAt index");
                self.may_throw();
                Type::Character
            }
        }
    }
}

fn dot_form(args: &[Form]) -> Option<(&Form, &str, &[Form])> {
    let [receiver, member, tail @ ..] = args else {
        return None;
    };
    match &member.without_metadata().kind {
        Kind::List(xs) if tail.is_empty() => {
            let (head, args) = xs.split_first()?;
            Some((receiver, head.symbol()?, args))
        }
        _ => Some((receiver, member.symbol()?, tail)),
    }
}
