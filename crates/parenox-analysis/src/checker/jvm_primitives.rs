//! JVM compiler classes and primitive function boundaries.
use super::metadata::java_type_hint;
use super::{Checker, Env, Value, nil_part};
use crate::reader::{
    Form, IntegerLiteralClass, Kind, NumericLiteral, Span, classify_number_with_class,
};
use crate::types::{Compatibility, IntegerClass, IntegerRange, IntegerType, Type};

pub(super) const MAX_NATIVE_ARGS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum JvmValueClass {
    Unknown,
    Boxed,
    Primitive(JvmPrimitive),
    Recur,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum JvmPrimitive {
    Byte,
    Short,
    Char,
    Int,
    Long,
    Float,
    Double,
}

impl JvmPrimitive {
    pub(super) fn function_argument(form: &Form) -> Option<Self> {
        form.symbol()?;
        Self::function_return(form)
    }

    pub(super) fn function_return(form: &Form) -> Option<Self> {
        match java_type_hint(form)?.name()? {
            "long" => Some(Self::Long),
            "double" => Some(Self::Double),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Byte => "byte",
            Self::Short => "short",
            Self::Char => "char",
            Self::Int => "int",
            Self::Long => "long",
            Self::Float => "float",
            Self::Double => "double",
        }
    }

    pub(super) fn accepts_unboxed(self, source: Self) -> bool {
        self == source
            || matches!(
                (self, source),
                (Self::Long, Self::Int) | (Self::Double, Self::Float)
            )
    }

    pub(super) fn function_value(self, input: &Type) -> Type {
        if *input == Type::Never {
            return Type::Never;
        }
        match self {
            Self::Long => {
                let range = match input {
                    Type::Integer(value) => return Type::Integer(*value),
                    Type::Integral(IntegerType {
                        class: IntegerClass::Fixed | IntegerClass::IntOrLong | IntegerClass::BigInt,
                        range,
                    }) if *range != IntegerRange::OutsideInt64 => *range,
                    _ => IntegerRange::Any,
                };
                Type::Integral(IntegerType {
                    class: IntegerClass::IntOrLong,
                    range,
                })
            }
            Self::Double => Type::NonIntegral,
            _ => unreachable!("JVM functions only accept primitive long and double"),
        }
    }
}

impl Checker<'_> {
    pub(super) fn compiler_value_class(&self, form: &Form, env: &Env) -> JvmValueClass {
        use JvmPrimitive::*;
        use JvmValueClass::*;
        let form = form.without_metadata();
        match &form.kind {
            Kind::Number => {
                let Some(text) = self
                    .source
                    .get(form.span.start.offset..form.span.end.offset)
                else {
                    return Unknown;
                };
                match classify_number_with_class(text) {
                    (_, Some(IntegerLiteralClass::Long)) => Primitive(Long),
                    (NumericLiteral::NonInteger, _)
                        if !text.contains('/') && !text.ends_with('M') =>
                    {
                        Primitive(Double)
                    }
                    _ => Boxed,
                }
            }
            Kind::SymbolicNumber(_) => Primitive(Double),
            Kind::Symbol(name) | Kind::AnonymousArgument(name) => env.jvm_class(name),
            Kind::Quote(inner) => self.quoted_value_class(inner, env),
            Kind::Bool(_)
            | Kind::Nil
            | Kind::Keyword(_)
            | Kind::Character(_)
            | Kind::String(_)
            | Kind::Utf16String(_)
            | Kind::Regex { .. }
            | Kind::Vector(_)
            | Kind::Map(_)
            | Kind::AutoNamespacedMap { .. }
            | Kind::Set(_)
            | Kind::AnonymousFunction(_) => Boxed,
            Kind::List(items) => {
                let Some((operator, operands)) = items.split_first() else {
                    return Boxed;
                };
                match (operator.symbol(), operands) {
                    (Some("do"), _) => {
                        return operands
                            .last()
                            .map_or(Boxed, |form| self.compiler_value_class(form, env));
                    }
                    (Some("if"), [_, yes] | [_, yes, _]) => {
                        let yes = self.compiler_value_class(yes, env);
                        let no = operands
                            .get(2)
                            .map_or(Boxed, |form| self.compiler_value_class(form, env));
                        return match (yes, no) {
                            (a, b) if a == b => a,
                            (Recur, class) | (class, Recur) => class,
                            (Boxed, _) | (_, Boxed) | (Primitive(_), Primitive(_)) => Boxed,
                            _ => Unknown,
                        };
                    }
                    (Some("recur"), _) => return Recur,
                    (Some("throw"), [_]) => return Boxed,
                    (Some("quote"), [inner]) => return self.quoted_value_class(inner, env),
                    (_, [_]) => {}
                    _ => return Unknown,
                }
                match self.standard_core_name(operator, env).as_deref() {
                    Some("clojure.core/byte") => Primitive(Byte),
                    Some("clojure.core/short") => Primitive(Short),
                    Some("clojure.core/char") => Primitive(Char),
                    Some("clojure.core/int") => Primitive(Int),
                    Some("clojure.core/long") => Primitive(Long),
                    Some("clojure.core/float") => Primitive(Float),
                    Some("clojure.core/double") => Primitive(Double),
                    Some("clojure.core/identity") => Boxed,
                    _ => Unknown,
                }
            }
            _ => Unknown,
        }
    }

    fn quoted_value_class(&self, form: &Form, env: &Env) -> JvmValueClass {
        match form.kind {
            Kind::Number | Kind::SymbolicNumber(_) => self.compiler_value_class(form, env),
            _ => JvmValueClass::Boxed,
        }
    }

    pub(super) fn convert_primitive_argument(
        &mut self,
        primitive: JvmPrimitive,
        span: Span,
        value: &Value,
    ) -> Value {
        self.convert_primitive_number(primitive, span, value, true)
    }

    pub(super) fn primitive_return(
        &mut self,
        primitive: JvmPrimitive,
        class: JvmValueClass,
        span: Span,
        value: Value,
    ) -> Value {
        match class {
            JvmValueClass::Primitive(source) if primitive.accepts_unboxed(source) => {
                Value::with_notes(primitive.function_value(&value.known), value.notes)
            }
            JvmValueClass::Primitive(source) => {
                self.error(
                    "PNX1001",
                    span,
                    format!(
                        "JVM {} return cannot use a primitive {}",
                        primitive.name(),
                        source.name()
                    ),
                );
                Value::new(Type::Never)
            }
            JvmValueClass::Boxed => self.convert_primitive_number(primitive, span, &value, false),
            JvmValueClass::Recur => value,
            JvmValueClass::Unknown if value.known == Type::Never => value,
            JvmValueClass::Unknown => self.unchecked_form(
                span,
                &format!(
                    "JVM {} compiler type for this return expression is unchecked",
                    primitive.name()
                ),
            ),
        }
    }

    fn convert_primitive_number(
        &mut self,
        primitive: JvmPrimitive,
        span: Span,
        value: &Value,
        is_argument: bool,
    ) -> Value {
        if value.known == Type::Never {
            return Value::new(Type::Never);
        }
        let boundary = if is_argument { "parameter" } else { "return" };
        self.expect(
            value,
            &Type::Number,
            span,
            &format!("JVM {} {boundary}", primitive.name()),
        );
        let alternatives = match &value.known {
            Type::Union(parts) => parts.as_ref(),
            ty => std::slice::from_ref(ty),
        };
        let result = Type::union(alternatives.iter().filter_map(|ty| {
            if nil_part(ty) != Type::Never {
                self.null_pointer();
            }
            if *ty == Type::Nil {
                return None;
            }
            // Each alternative may contain numbers even when its domain also
            // contains non-numbers. Removing nil from Any introduces a union
            // whose all-members compatibility would discard the numeric path.
            let compatibility = ty.compatibility(&Type::Number);
            if compatibility != Compatibility::Proven {
                self.class_cast();
            }
            if compatibility == Compatibility::Mismatch {
                return None;
            }
            let fixed = Type::Integral(IntegerType::FIXED);
            if !ty.subtype_of(&fixed) {
                // Numeric wrappers may call overridden Number methods. Long
                // conversion can also fail its checked range test.
                if !matches!(
                    ty,
                    Type::Integral(IntegerType {
                        class: IntegerClass::BigInt,
                        ..
                    })
                ) {
                    self.may_throw();
                }
                if is_argument && primitive == JvmPrimitive::Long && ty.exact_long_value().is_none()
                {
                    self.illegal_argument();
                }
            }
            Some(primitive.function_value(ty))
        }));
        Value::with_notes(result, value.notes.clone())
    }
}
