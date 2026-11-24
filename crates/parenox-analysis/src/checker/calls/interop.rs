//! Contracts for supported JVM instance methods.
use super::{Checker, Env, Form, Span, Type, Value, call_returns};
use crate::checker::LiteralContainer;
use crate::checker::flow::has_closed_scalar_class;
use crate::checker::model::{DeclaredValue, Model};
use crate::core_names::CoreBinding;
use crate::reader::Kind;
use crate::types::{Compatibility, IndexedItem, IntegerType, JvmArrayClass};

const INPUT_STREAM_READ: &str = "java.io.InputStream/.read";

#[derive(Clone, Copy)]
pub(super) enum QualifiedJvmMember {
    InputStreamRead,
    Iteration(IterationMember),
}

#[derive(Clone, Copy)]
pub(super) enum IterationMember {
    Iterator,
    HasNext,
    Next,
}

impl IterationMember {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "iterator" => Self::Iterator,
            "hasNext" => Self::HasNext,
            "next" => Self::Next,
            _ => return None,
        })
    }

    fn receiver_class(self) -> &'static str {
        match self {
            Self::Iterator => "java.lang.Iterable",
            Self::HasNext | Self::Next => "java.util.Iterator",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Iterator => "java.lang.Iterable/.iterator",
            Self::HasNext => "java.util.Iterator/.hasNext",
            Self::Next => "java.util.Iterator/.next",
        }
    }

    fn selects_class(self, class: &str) -> bool {
        class == self.receiver_class() || matches!((self, class), (Self::Iterator, "Iterable"))
    }

    fn known_receiver(self, value: &Value) -> bool {
        match self {
            Self::Iterator => {
                let ty = value.known.without_nil();
                crate::checker::maps::map_predicate(&ty) == Type::Bool(true)
                    || matches!(
                        value.literal_container,
                        Some(
                            LiteralContainer::Vector(_)
                                | LiteralContainer::NonemptyVector
                                | LiteralContainer::List(_)
                                | LiteralContainer::Set(_)
                        )
                    )
            }
            Self::HasNext | Self::Next => value.known.without_nil() == Type::JvmIterator,
        }
    }
}

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

pub(in crate::checker) fn is_modeled_string_member(name: &str) -> bool {
    StringMethod::parse(name).is_some()
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
                && (self.model.has_unavailable_wildcard_scope(head)
                    || self.model.standard_core_binding(
                        head,
                        &self.model.resolve(head),
                        self.globals,
                    ) == Some(CoreBinding::Macro)))
            {
                return None;
            }
        }
        if let Some(value) = self.jvm_string_call(head, forms, span, env) {
            return Some(value);
        }
        let (receiver, member, args) = if head == "." {
            let Some((receiver, member, args)) = dot_form(forms) else {
                self.error(
                    "PNX1001",
                    span,
                    "A JVM member form requires a receiver and member name",
                );
                return Some(Value::new(Type::Unknown));
            };
            (Some(receiver), Some(member), args)
        } else if let Some(member) = head.strip_prefix('.') {
            let Some((receiver, args)) = forms.split_first() else {
                self.error("PNX1001", span, format!("{head} requires a receiver"));
                return Some(Value::new(Type::Unknown));
            };
            (Some(receiver), Some(member), args)
        } else {
            (None, None, forms)
        };
        if let (Some(receiver), Some(member)) = (receiver, member)
            && !receiver
                .symbol()
                .is_some_and(|name| !env.contains_key(name) && self.model.known_java_class(name))
        {
            if member == "getException" {
                return Some(self.reflective_get_exception(receiver, args, span, env));
            }
            if args.is_empty()
                && let Some(method) = IterationMember::parse(member)
            {
                return Some(self.iteration_dot(method, receiver, span, env));
            }
        }
        Some(self.unmodeled_jvm_call(receiver, args, span, env))
    }

    pub(super) fn qualified_jvm_member(&self, operator: &Form) -> Option<QualifiedJvmMember> {
        let symbol = operator.symbol()?;
        if operator.metadata(":param-tags").is_some()
            || !self.model.known_java_member(symbol, self.globals)
        {
            return None;
        }
        let (class, member) = symbol.rsplit_once('/')?;
        let class = self.model.class_name(class);
        if class == "java.io.InputStream" && member == ".read" {
            return Some(QualifiedJvmMember::InputStreamRead);
        }
        let member = IterationMember::parse(member.strip_prefix('.')?)?;
        member
            .selects_class(class)
            .then_some(QualifiedJvmMember::Iteration(member))
    }

    pub(super) fn qualified_instance_call(
        &mut self,
        operator: &Form,
        forms: &[Form],
        span: Span,
        env: &Env,
    ) -> Option<Value> {
        let (class, member) = operator.symbol()?.rsplit_once('/')?;
        if let Some(member) = self.qualified_jvm_member(operator) {
            let args = self.eval_args(forms, env, true).collect::<Vec<_>>();
            return Some(self.call_jvm_member(member, &args, span));
        }
        if member == ".getException"
            && matches!(
                self.model.class_name(class),
                "Exception" | "java.lang.Exception"
            )
        {
            let args = self.eval_args(forms, env, true).collect::<Vec<_>>();
            if call_returns(&args) && self.path_reachable {
                self.error(
                    "PNX2339",
                    operator.without_metadata().span,
                    "java.lang.Exception has no instance method getException",
                );
                self.illegal_argument();
            }
            return Some(Value::new(Type::Never));
        }
        None
    }

    pub(super) fn call_jvm_member(
        &mut self,
        member: QualifiedJvmMember,
        args: &[(Value, Span)],
        span: Span,
    ) -> Value {
        if !call_returns(args) || !self.path_reachable {
            return Value::new(Type::Never);
        }
        let start = self.diagnostics.len();
        let (ty, name) = match member {
            QualifiedJvmMember::InputStreamRead => {
                (self.check_input_stream_read(args, span), INPUT_STREAM_READ)
            }
            QualifiedJvmMember::Iteration(method) => (
                self.check_qualified_iteration(method, args, span),
                method.name(),
            ),
        };
        self.note_builtin_contract(start, name);
        Value::new(ty)
    }

    fn iteration_dot(
        &mut self,
        method: IterationMember,
        receiver: &Form,
        span: Span,
        env: &Env,
    ) -> Value {
        let value = self.infer(receiver, env);
        if value.stops_evaluation() || !self.path_reachable {
            return Value::new(Type::Never);
        }
        // The JVM tag and the Clojure flow type carry different information.
        let selected = match env.java_hint(receiver) {
            Some(hint) => hint
                .name()
                .is_some_and(|name| method.selects_class(self.model.class_name(name))),
            None => method.known_receiver(&value),
        };
        if !selected {
            return self.unchecked_form(span, "JVM call is outside the supported member set");
        }
        let start = self.diagnostics.len();
        let result =
            self.check_iteration_receiver(method, &value, receiver.without_metadata().span);
        self.note_builtin_contract(start, method.name());
        Value::new(result)
    }

    fn check_qualified_iteration(
        &mut self,
        method: IterationMember,
        args: &[(Value, Span)],
        span: Span,
    ) -> Type {
        let [(receiver, site)] = args else {
            self.error(
                "PNX2554",
                span,
                format!(
                    "{} expects a receiver and no member arguments",
                    method.name()
                ),
            );
            self.illegal_argument();
            return Type::Never;
        };
        self.check_iteration_receiver(method, receiver, *site)
    }

    fn check_iteration_receiver(
        &mut self,
        method: IterationMember,
        receiver: &Value,
        span: Span,
    ) -> Type {
        let nullable = receiver.known.without_nil() != receiver.known;
        let disjoint = receiver.known == Type::Nil
            || has_closed_scalar_class(&receiver.known)
            || matches!(receiver.known, Type::JvmArray(_))
            || (!matches!(method, IterationMember::Iterator)
                && matches!(
                    receiver.literal_container,
                    Some(
                        LiteralContainer::Map(_)
                            | LiteralContainer::GroupBuckets
                            | LiteralContainer::Vector(_)
                            | LiteralContainer::NonemptyVector
                            | LiteralContainer::Set(_)
                            | LiteralContainer::List(_)
                    )
                ));
        if nullable || disjoint {
            self.diagnostics.push(crate::Diagnostic::type_mismatch(
                span,
                method.name(),
                method.receiver_class().into(),
                receiver.ty.to_string(),
            ));
        }
        if nullable {
            self.null_pointer();
        } else if disjoint {
            self.class_cast();
        }
        if disjoint {
            return Type::Never;
        }
        self.may_throw();
        match method {
            IterationMember::Iterator => Type::JvmIterator,
            IterationMember::HasNext => Type::Boolean,
            IterationMember::Next => Type::Any,
        }
    }

    fn check_input_stream_read(&mut self, args: &[(Value, Span)], span: Span) -> Type {
        let [(receiver, receiver_span), member_args @ ..] = args else {
            self.error(
                "PNX2554",
                span,
                format!("{INPUT_STREAM_READ} requires a receiver"),
            );
            self.arity_error();
            return Type::Never;
        };
        if !matches!(member_args.len(), 0 | 1 | 3) {
            self.error(
                "PNX2554",
                span,
                format!(
                    "{INPUT_STREAM_READ} expects 0, 1 or 3 member arguments, got {}",
                    member_args.len()
                ),
            );
            self.illegal_argument();
            return Type::Never;
        }
        if receiver.known == Type::Nil
            || has_closed_scalar_class(&receiver.known)
            || matches!(
                receiver.known,
                Type::JvmArray(_) | Type::Throwable | Type::ExceptionInfo(_)
            )
        {
            self.diagnostics.push(crate::Diagnostic::type_mismatch(
                *receiver_span,
                INPUT_STREAM_READ,
                "java.io.InputStream".into(),
                receiver.ty.to_string(),
            ));
            if receiver.known == Type::Nil {
                self.null_pointer();
            } else {
                self.class_cast();
            }
            return Type::Never;
        }
        if let Some(((buffer, buffer_span), indices)) = member_args.split_first() {
            self.expect(
                buffer,
                &Type::JvmArray(JvmArrayClass::Byte),
                *buffer_span,
                INPUT_STREAM_READ,
            );
            for (argument, site) in indices {
                self.expect(argument, &Type::Number, *site, INPUT_STREAM_READ);
            }
        }
        self.may_throw();
        Type::Integral(IntegerType::FIXED_INT32)
    }

    fn reflective_get_exception(
        &mut self,
        receiver: &Form,
        args: &[Form],
        span: Span,
        env: &Env,
    ) -> Value {
        let value = self.infer(receiver, env);
        let operands_return = self
            .eval_args(args, env, !value.stops_evaluation())
            .last()
            .is_none_or(|(value, _)| !value.stops_evaluation());
        if value.stops_evaluation() || !operands_return || !self.path_reachable {
            return Value::new(Type::Never);
        }
        if matches!(value.known, Type::ExceptionInfo(_)) {
            self.error(
                "PNX2339",
                span,
                "clojure.lang.ExceptionInfo has no instance method getException",
            );
            self.illegal_argument();
            return Value::new(Type::Never);
        }
        if env
            .java_hint(receiver)
            .and_then(|hint| hint.name())
            .is_some_and(|name| {
                matches!(
                    self.model.class_name(name),
                    "Exception" | "java.lang.Exception"
                )
            })
        {
            return self.unchecked_form(
                span,
                "java.lang.Exception has no getException; this reflective call depends on the runtime subclass",
            );
        }
        self.unchecked_form(span, "JVM call is outside the supported member set")
    }

    pub(super) fn jvm_new_form(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        let Some(args) = new_arguments(forms) else {
            self.error("PNX1001", span, "new requires a JVM class name");
            return Value::new(Type::Unknown);
        };
        self.unmodeled_jvm_call(None, args, span, env)
    }

    pub(super) fn unmodeled_jvm_call(
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
        let tag = env.java_hint(receiver);
        let string_receiver = match tag {
            Some(hint) => hint.name().is_some_and(|name| {
                matches!(self.model.class_name(name), "String" | "java.lang.String")
            }),
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
