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

pub(in crate::checker) fn boxed_array_element(class: JvmArrayClass) -> Type {
    use JvmArrayClass::*;
    match class {
        Any | Reference => Type::Any,
        Boolean => Type::Boolean,
        Char => Type::Character,
        Byte | Short | Int | Long => Type::Integral(IntegerType::FIXED),
        Float | Double => Type::NonIntegral,
    }
}

pub(super) fn array_index_in_bounds(index: i32, length: Option<usize>) -> Option<bool> {
    if index < 0 || index == i32::MAX {
        return Some(false);
    }
    length.map(|length| (index as usize) < length)
}

pub(in crate::checker) fn array_key_is_nonnumeric(key: &Type) -> bool {
    matches!(key, Type::JvmArray(_)) || key.known_intersection(&Type::Number) == Some(Type::Never)
}

pub(in crate::checker) fn stable_array_key(key: &Type) -> bool {
    key.subtype_of(&Type::Number) && has_closed_scalar_class(key)
}

fn array_with_length(array: Type, length: usize) -> Value {
    let mut value = Value::new(array);
    value.literal_container = Some(LiteralContainer::ArrayLength(length));
    value
}

impl Checker<'_> {
    pub(in crate::checker) fn array_lookup_presence(
        &mut self,
        key: &Type,
        length: Option<usize>,
    ) -> Option<bool> {
        if has_closed_scalar_class(key)
            && let Some(index) = key.exact_long_value()
        {
            // RT.get uses Number.intValue(), which can wrap an integral key.
            return array_index_in_bounds(index as i32, length)
                .or_else(|| (length == Some(0)).then_some(false));
        }
        if array_key_is_nonnumeric(key) {
            return Some(false);
        }
        if !has_closed_scalar_class(key) {
            // An abstract Number can supply its own intValue implementation.
            self.may_throw();
        }
        (length == Some(0)).then_some(false)
    }

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
            return array_with_length(array, 0);
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
            return array_with_length(array, size as usize);
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
            return array_with_length(array, items.len());
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
        if let Err(result) = self.check_array_receiver(receiver, span, hint, "clojure.core/alength")
        {
            return Value::new(result);
        }
        let length = receiver
            .known_array_length()
            .and_then(|size| i32::try_from(size).ok());
        Value::new(length.map_or(Type::Integral(IntegerType::FIXED_INT32), Type::Integer))
    }

    pub(super) fn array_get(
        &mut self,
        args: &[(Value, Span)],
        receiver_span: Span,
        call_span: Span,
        hint: Option<JavaTypeHint<&str>>,
        inlined: bool,
    ) -> Value {
        let [(receiver, _), (index, index_span)] = args else {
            self.may_throw();
            return self.unchecked_form(
                call_span,
                "Type analysis for multidimensional aget is unavailable",
            );
        };
        let class =
            match self.check_array_receiver(receiver, receiver_span, hint, "clojure.core/aget") {
                Ok(class) => class,
                Err(result) => return Value::new(result),
            };
        Value::new(self.aget_index(
            class,
            receiver.known_array_length(),
            &index.known,
            *index_span,
            inlined,
        ))
    }

    fn aget_index(
        &mut self,
        class: JvmArrayClass,
        length: Option<usize>,
        index: &Type,
        span: Span,
        inlined: bool,
    ) -> Type {
        if let Some(index) = index.exact_long_value() {
            if let Ok(index) = i32::try_from(index) {
                return self.read_array_at(class, length, Some(index), span, None);
            }
            self.arithmetic_exception();
            self.error(
                "PNX2801",
                span,
                format!("Index {index} is outside the JVM int range"),
            );
            return Type::Never;
        }
        match index {
            Type::Never => return Type::Never,
            Type::Unknown => {
                self.may_throw();
                return Type::Unknown;
            }
            Type::Union(parts) => {
                return Type::union(
                    parts
                        .iter()
                        .map(|part| self.aget_index(class, length, part, span, inlined)),
                );
            }
            _ => {}
        }
        // The inline expansion uses Clojure int, which accepts a Character.
        if inlined {
            match index {
                Type::Char(value) => {
                    return self.read_array_at(class, length, Some(*value as i32), span, None);
                }
                Type::Character => return self.read_array_at(class, length, None, span, None),
                _ => {}
            }
        }
        if array_key_is_nonnumeric(index) {
            if *index == Type::Nil {
                self.null_pointer();
            } else {
                self.class_cast();
            }
            self.diagnostics.push(crate::Diagnostic::type_mismatch(
                span,
                "clojure.core/aget index",
                if inlined {
                    "Number or Character"
                } else {
                    "Number"
                }
                .into(),
                index.inferred().to_string(),
            ));
            return Type::Never;
        }
        if !index.subtype_of(&Type::Number) || !has_closed_scalar_class(index) {
            self.may_throw();
        } else if !index.subtype_of(&Type::Integral(IntegerType::INT32)) {
            self.arithmetic_exception();
        }
        self.read_array_at(class, length, None, span, None)
    }

    fn check_array_receiver(
        &mut self,
        receiver: &Value,
        span: Span,
        hint: Option<JavaTypeHint<&str>>,
        operation: &str,
    ) -> Result<JvmArrayClass, Type> {
        let selected = if let Some(hint) = hint {
            if let Some(class) = self.array_hint(hint) {
                Some(class)
            } else if hint
                .name()
                .is_some_and(|name| self.model.known_java_class(name))
            {
                // A known non-array hint leaves reflective dispatch.
                None
            } else {
                self.may_throw();
                return Err(self
                    .unchecked_form(span, "JVM array overload for this type hint is unchecked")
                    .known);
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
                self.expect(receiver, &expected, span, operation);
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
            Ok(match non_nil {
                Type::JvmArray(class) if class != JvmArrayClass::Any => class,
                _ => selected.unwrap_or(JvmArrayClass::Any),
            })
        } else {
            Err(Type::Never)
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
