//! JVM results and exceptions of the core string parsers.
use super::{Checker, Span, Type, Value};
use crate::reader::decimal_digit;
use crate::types::{IntegerClass, IntegerType};

pub(super) enum Parser {
    Long,
    Boolean,
    Double,
    Uuid,
}

impl Parser {
    pub(super) fn for_name(name: &str) -> Option<Self> {
        match name {
            "clojure.core/parse-long" => Some(Self::Long),
            "clojure.core/parse-boolean" => Some(Self::Boolean),
            "clojure.core/parse-double" => Some(Self::Double),
            "clojure.core/parse-uuid" => Some(Self::Uuid),
            _ => None,
        }
    }

    fn result(&self, input: &Type) -> Type {
        match input {
            Type::Str(text) => self.literal(text),
            Type::Union(parts) => Type::union(parts.iter().map(|part| self.result(part))),
            input if input.known_intersection(&Type::String) == Some(Type::Never) => Type::Never,
            _ => match self {
                Self::Long => Type::Integral(IntegerType {
                    class: IntegerClass::IntOrLong,
                    ..IntegerType::FIXED
                }),
                Self::Boolean => Type::Boolean,
                Self::Double => Type::NonIntegral,
                Self::Uuid => Type::Uuid,
            }
            .nilable(),
        }
    }

    fn literal(&self, text: &str) -> Type {
        match self {
            Self::Long => decimal_long(text).map_or(Type::Nil, |value| {
                Type::exact_long(value, IntegerClass::IntOrLong)
            }),
            Self::Boolean => match text {
                "true" => Type::Bool(true),
                "false" => Type::Bool(false),
                _ => Type::Nil,
            },
            Self::Double if double_syntax(text) => Type::NonIntegral,
            Self::Double => Type::Nil,
            Self::Uuid => uuid_result(text),
        }
    }
}

impl Checker<'_> {
    pub(super) fn parse_string(
        &mut self,
        name: &str,
        parser: Parser,
        (value, span): &(Value, Span),
    ) -> Value {
        if value.stops_evaluation() {
            return Value::new(Type::Never);
        }
        self.expect(value, &Type::String, *span, name);
        if matches!(parser, Parser::Uuid) {
            if value.known.known_intersection(&Type::Nil) != Some(Type::Never) {
                self.null_pointer();
            }
            if !value.known.subtype_of(&Type::String.nilable()) {
                self.class_cast();
            }
        } else if !value.known.subtype_of(&Type::String) {
            self.illegal_argument();
        }
        Value::new(parser.result(&value.known))
    }
}

fn uuid_result(text: &str) -> Type {
    if text.encode_utf16().take(37).count() > 36 {
        return Type::Nil;
    }
    if !text.is_ascii() {
        // The JVM fallback also accepts certain Unicode digits.
        return Type::Uuid.nilable();
    }
    let mut groups = text.split('-');
    // UUID.fromString parses five signed-long-sized hex groups, including
    // short groups and leading plus signs, then masks them to field width.
    if (0..5).all(|_| {
        groups
            .next()
            .is_some_and(|group| i64::from_str_radix(group, 16).is_ok())
    }) && groups.next().is_none()
    {
        Type::Uuid
    } else {
        Type::Nil
    }
}

fn decimal_long(text: &str) -> Option<i64> {
    if text.is_ascii() {
        return text.parse().ok();
    }
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    let value = digits.chars().try_fold(0_i64, |value, digit| {
        // Accumulate negatively to accommodate Long/MIN_VALUE.
        value
            .checked_mul(10)?
            .checked_sub(i64::from(decimal_digit(digit)?))
    })?;
    if text.starts_with('-') {
        Some(value)
    } else {
        value.checked_neg()
    }
}

fn double_syntax(text: &str) -> bool {
    let text = text.trim_matches(|ch: char| ch <= '\u{20}');
    let text = strip_sign(text);
    if matches!(text, "NaN" | "Infinity") {
        return true;
    }
    let text = text.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(text);
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return hex.split_once(['p', 'P']).is_some_and(|(mantissa, exp)| {
            significand(mantissa, u8::is_ascii_hexdigit) && exponent(exp)
        });
    }
    let mantissa = match text.split_once(['e', 'E']) {
        Some((mantissa, exp)) if exponent(exp) => mantissa,
        Some(_) => return false,
        None => text,
    };
    significand(mantissa, u8::is_ascii_digit)
}

fn strip_sign(text: &str) -> &str {
    text.strip_prefix(['+', '-']).unwrap_or(text)
}

fn exponent(text: &str) -> bool {
    let digits = strip_sign(text).as_bytes();
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fn significand(text: &str, is_digit: fn(&u8) -> bool) -> bool {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    !(whole.is_empty() && fraction.is_empty())
        && whole
            .as_bytes()
            .iter()
            .chain(fraction.as_bytes())
            .all(is_digit)
}
