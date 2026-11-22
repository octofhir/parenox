//! Narrow contracts for instance methods on a known JVM String receiver.
use super::{Checker, Env, Form, Span, Type, Value, call_returns};
use crate::checker::model::{DeclaredValue, Model};
use crate::core_names::CoreBinding;
use crate::reader::Kind;
use crate::types::{Compatibility, IndexedItem};

#[derive(Clone, Copy)]
enum StringMethod {
    Length,
    CharAt,
    StartsWith,
    EndsWith,
    IndexOf,
}

impl StringMethod {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "length" => Some(Self::Length),
            "charAt" => Some(Self::CharAt),
            "startsWith" => Some(Self::StartsWith),
            "endsWith" => Some(Self::EndsWith),
            "indexOf" => Some(Self::IndexOf),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Length => "java.lang.String/length",
            Self::CharAt => "java.lang.String/charAt",
            Self::StartsWith => "java.lang.String/startsWith",
            Self::EndsWith => "java.lang.String/endsWith",
            Self::IndexOf => "java.lang.String/indexOf",
        }
    }

    fn arity(self) -> usize {
        match self {
            Self::Length => 0,
            Self::CharAt | Self::StartsWith | Self::EndsWith | Self::IndexOf => 1,
        }
    }

    fn unsupported_arity(self, arity: usize) -> bool {
        matches!(
            (self, arity),
            (Self::StartsWith, 2) | (Self::IndexOf, 2 | 3)
        )
    }
}

impl Checker<'_> {
    pub(super) fn jvm_call(
        &mut self,
        head: &str,
        operator: Span,
        forms: &[Form],
        span: Span,
        env: &Env,
    ) -> Option<Value> {
        if head != "." && !env.contains_key(head) {
            let declared = self
                .model
                .declared_value(head, self.globals, operator.start.offset);
            if matches!(
                declared,
                Some(
                    DeclaredValue::Macro(_)
                        | DeclaredValue::OpaqueMacro
                        | DeclaredValue::PrivateMacro
                )
            ) || (declared.is_none()
                && self
                    .model
                    .standard_core_binding(head, &self.model.resolve(head), self.globals)
                    == Some(CoreBinding::Macro))
            {
                return None;
            }
        }
        if let Some(value) = self.jvm_string_call(head, forms, span, env) {
            return Some(value);
        }
        let (receiver, args) = if head == "." {
            let Some((receiver, _, args)) = dot_form(forms) else {
                self.error(
                    "PNX1001",
                    span,
                    "A JVM member form requires a receiver and member name",
                );
                return Some(Value::new(Type::Unknown));
            };
            (Some(receiver), args)
        } else if head.starts_with('.') {
            let Some((receiver, args)) = forms.split_first() else {
                self.error("PNX1001", span, format!("{head} requires a receiver"));
                return Some(Value::new(Type::Unknown));
            };
            (Some(receiver), args)
        } else {
            (None, forms)
        };
        Some(self.unmodeled_jvm_call(receiver, args, span, env))
    }

    pub(super) fn jvm_new_form(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        let Some(args) = new_arguments(forms) else {
            self.error("PNX1001", span, "new requires a JVM class name");
            return Value::new(Type::Unknown);
        };
        self.unmodeled_jvm_call(None, args, span, env)
    }

    fn unmodeled_jvm_call(
        &mut self,
        receiver: Option<&Form>,
        forms: &[Form],
        span: Span,
        env: &Env,
    ) -> Value {
        let receiver_returns = receiver.is_none_or(|receiver| {
            let class_syntax = receiver.symbol().is_some_and(|name| {
                !env.contains_key(name)
                    && (self.model.known_java_class(name)
                        || (name.contains('.') && Model::possible_java_class(name)))
            });
            class_syntax || !self.infer(receiver, env).stops_evaluation()
        });
        let operands_return = self
            .eval_args(forms, env, receiver_returns)
            .last()
            .is_none_or(|(value, _)| !value.stops_evaluation());
        self.check_path(receiver_returns && operands_return, |checker| {
            checker.unchecked_form(span, "JVM call is outside the supported member set")
        })
    }

    fn jvm_string_call(
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
            let Some((receiver, args)) = args.split_first() else {
                self.error("PNX1001", span, format!("{head} requires a receiver"));
                return Some(Value::new(Type::Unknown));
            };
            (method, receiver, args)
        };

        let receiver_type = self.infer(receiver, env);
        let receiver_returns = !receiver_type.stops_evaluation();
        let typed: Vec<_> = self.eval_args(args, env, receiver_returns).collect();
        if !receiver_returns || !call_returns(&typed) {
            return Some(Value::new(Type::Never));
        }
        let tag = receiver
            .metadata(":tag")
            .and_then(tag_name)
            .or_else(|| receiver.symbol().and_then(|name| env.java_hint(name)));
        let string_receiver = match tag.map(|name| self.model.class_name(name)) {
            Some("String" | "java.lang.String") => true,
            Some(_) => false,
            None => receiver_type.known.subtype_of(&Type::String.nilable()),
        };
        if !string_receiver {
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
        if method.unsupported_arity(args.len()) {
            return self
                .unchecked_form(span, "This JVM String overload is not supported")
                .known;
        }
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
        // An explicit tag selects the method but does not prove a non-null
        // receiver, or even that the compiler's cast will succeed.
        if receiver.known.compatibility(&Type::String) == Compatibility::Unproven {
            self.unchecked_form(
                receiver_span,
                "Cannot verify that the hinted JVM receiver is a non-nil String",
            );
        } else {
            self.expect(receiver, &Type::String, receiver_span, method.name());
        }
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
                let (index, span) = &args[0];
                self.string_char_at(&receiver.known, index, *span)
            }
            StringMethod::StartsWith | StringMethod::EndsWith | StringMethod::IndexOf => {
                let (argument, argument_span) = &args[0];
                self.string_search(method, argument, *argument_span)
            }
        }
    }

    fn string_search(&mut self, method: StringMethod, argument: &Value, span: Span) -> Type {
        let compatibility = argument.known.compatibility(&Type::String);
        if compatibility == Compatibility::Unproven {
            return self
                .unchecked_form(span, "Cannot verify the argument to this JVM String method")
                .known;
        }
        if compatibility == Compatibility::Mismatch {
            let expected = if matches!(method, StringMethod::IndexOf) {
                // The JVM also has an int code-point overload. Recognize the
                // possible overload without trying to prove primitive conversion.
                let domain = Type::union([Type::String, Type::Number]);
                if argument.known.compatibility(&domain) != Compatibility::Mismatch {
                    return self
                        .unchecked_form(
                            span,
                            "Only the String overload of String.indexOf is supported",
                        )
                        .known;
                }
                domain
            } else {
                Type::String
            };
            self.expect(argument, &expected, span, method.name());
            self.may_throw();
            if argument.known == Type::Nil {
                return Type::Never;
            }
        }
        match method {
            StringMethod::IndexOf => Type::Number,
            _ => Type::Boolean,
        }
    }

    fn string_char_at(&mut self, text: &Type, index: &Value, span: Span) -> Type {
        self.expect(index, &Type::Number, span, "String.charAt index");
        self.may_throw();
        let Some(index) = index.known.exact_integer_value() else {
            return Type::Character;
        };
        if index >= 0 {
            if !matches!(text, Type::Str(_)) {
                return Type::Character;
            }
            match text.indexed_item(index) {
                Some(IndexedItem::Present(item)) => return item.clone(),
                Some(IndexedItem::PresentChar(value)) => return Type::Char(value),
                _ => {}
            }
        }
        self.error(
            "PNX2801",
            span,
            format!("String.charAt index {index} is out of bounds"),
        );
        Type::Never
    }
}

fn tag_name(tag: &Form) -> Option<&str> {
    match &tag.without_metadata().kind {
        Kind::String(name) => Some(name),
        _ => tag.symbol(),
    }
}

pub(in crate::checker) fn new_arguments(forms: &[Form]) -> Option<&[Form]> {
    let (class, args) = forms.split_first()?;
    class.symbol().map(|_| args)
}

pub(in crate::checker) fn dot_form(args: &[Form]) -> Option<(&Form, &str, &[Form])> {
    let [receiver, member, tail @ ..] = args else {
        return None;
    };
    match &member.without_metadata().kind {
        Kind::List(xs) => {
            // The nested method list owns its arguments in JVM dot forms.
            let (head, args) = xs.split_first()?;
            Some((receiver, head.symbol()?, args))
        }
        _ => Some((receiver, member.symbol()?, tail)),
    }
}
