//! JVM array creation and the overload chosen by an array type hint.
use super::super::{LiteralContainer, metadata::JavaTypeHint};
use super::{Checker, Value, has_closed_scalar_class, nil_part};
use crate::reader::{Span, array_class_component};
use crate::types::{Compatibility, IntegerType, JvmArrayClass, Type};

pub(super) fn array_constructor(name: &str) -> Option<JvmArrayClass> {
    use JvmArrayClass::*;
    Some(match name {
        "clojure.core/object-array" => Reference,
        "clojure.core/boolean-array" => Boolean,
        "clojure.core/byte-array" => Byte,
        "clojure.core/char-array" => Char,
        "clojure.core/short-array" => Short,
        "clojure.core/int-array" => Int,
        "clojure.core/long-array" => Long,
        "clojure.core/float-array" => Float,
        "clojure.core/double-array" => Double,
        _ => return None,
    })
}

impl Checker<'_> {
    pub(super) fn jvm_array(
        &mut self,
        name: &str,
        class: JvmArrayClass,
        args: &[(Value, Span)],
        span: Span,
    ) -> Value {
        let [(input, input_span)] = args else {
            self.may_throw();
            return self.unchecked_form(
                span,
                &format!("Two-argument {name} initialization is unchecked"),
            );
        };
        let array = Type::JvmArray(class);
        if input.known == Type::Nil {
            return Value::new(array);
        }
        if has_closed_scalar_class(&input.known)
            && let Some(size) = input.known.exact_long_value()
        {
            // The one-argument JVM constructors call Number.intValue().
            let size = size as i32;
            if size < 0 {
                self.error(
                    "PNX2801",
                    *input_span,
                    format!("{name} length {size} is negative"),
                );
                self.negative_array_size();
                return Value::new(Type::Never);
            }
            return Value::new(array);
        }
        if input.known.subtype_of(&Type::Number) {
            self.may_throw();
            return Value::new(array);
        }
        let element = match class {
            JvmArrayClass::Reference => Type::Any,
            JvmArrayClass::Boolean => Type::Boolean,
            JvmArrayClass::Char => Type::Character,
            _ => Type::Number,
        };
        if matches!(input.literal_container, Some(LiteralContainer::Vector(_)))
            && let Type::Tuple(items) = &input.known
            && items.iter().all(|item| item.subtype_of(&element))
        {
            if element == Type::Number && !items.iter().all(has_closed_scalar_class) {
                self.may_throw();
            }
            return Value::new(array);
        }
        self.may_throw();
        self.unchecked_form(
            *input_span,
            &format!("{name} requires a known size or ordinary vector of {element}"),
        )
    }

    pub(super) fn array_length(
        &mut self,
        receiver: &Value,
        span: Span,
        hint: Option<JavaTypeHint<&str>>,
    ) -> Value {
        let selected = if let Some(hint) = hint {
            if let Some(class) = self.array_hint(hint) {
                Some(class)
            } else if hint
                .name()
                .is_some_and(|name| self.model.known_java_class(name))
            {
                // A known non-array hint leaves RT.alength reflectively dispatched.
                None
            } else {
                self.may_throw();
                return self
                    .unchecked_form(span, "JVM array overload for this type hint is unchecked");
            }
        } else {
            None
        };
        let expected = Type::JvmArray(selected.unwrap_or(JvmArrayClass::Any));
        let actual = &receiver.known;
        match actual.compatibility(&expected) {
            Compatibility::Proven => {}
            Compatibility::Unproven => {
                self.unchecked_form(
                    span,
                    &format!("Cannot verify the JVM array receiver as {expected}"),
                );
            }
            Compatibility::Mismatch => {
                self.expect(receiver, &expected, span, "clojure.core/alength");
            }
        }
        if nil_part(actual) != Type::Never {
            self.null_pointer();
        }
        let non_nil = actual.without_nil();
        if !non_nil.subtype_of(&expected) {
            if selected.is_some() {
                self.class_cast();
            } else {
                self.illegal_argument();
            }
        }
        if possible_array(&non_nil, &expected) {
            Value::new(Type::Integral(IntegerType::FIXED_INT32))
        } else {
            Value::new(Type::Never)
        }
    }

    fn array_hint(&self, hint: JavaTypeHint<&str>) -> Option<JvmArrayClass> {
        use JvmArrayClass::*;
        if let JavaTypeHint::Symbol(name) = hint
            && self.model.known_java_array_class(name, self.globals)
        {
            if !name.ends_with("/1") {
                return Some(Reference);
            }
            return Some(match array_class_component(name)? {
                "boolean" => Boolean,
                "byte" => Byte,
                "char" => Char,
                "short" => Short,
                "int" => Int,
                "long" => Long,
                "float" => Float,
                "double" => Double,
                _ => Reference,
            });
        }
        if let JavaTypeHint::Text(descriptor) = hint
            && descriptor.starts_with('[')
        {
            return self.array_descriptor(descriptor);
        }
        Some(match hint.name()? {
            "booleans" => Boolean,
            "bytes" => Byte,
            "chars" => Char,
            "shorts" => Short,
            "ints" => Int,
            "longs" => Long,
            "floats" => Float,
            "doubles" => Double,
            "objects" => Reference,
            _ => return None,
        })
    }

    fn array_descriptor(&self, descriptor: &str) -> Option<JvmArrayClass> {
        use JvmArrayClass::*;
        let component = descriptor.trim_start_matches('[');
        let rank = descriptor.len() - component.len();
        if rank > 255 {
            return None;
        }
        let class = match component {
            "Z" => Boolean,
            "B" => Byte,
            "C" => Char,
            "S" => Short,
            "I" => Int,
            "J" => Long,
            "F" => Float,
            "D" => Double,
            descriptor => {
                let name = descriptor.strip_prefix('L')?.strip_suffix(';')?;
                if !name.contains('.') || !self.model.known_java_class(name) {
                    return None;
                }
                Reference
            }
        };
        Some(if rank == 1 { class } else { Reference })
    }
}

fn possible_array(actual: &Type, expected: &Type) -> bool {
    match actual {
        Type::Never => false,
        Type::Union(parts) => parts.iter().any(|part| possible_array(part, expected)),
        _ => actual.compatibility(expected) != Compatibility::Mismatch,
    }
}
