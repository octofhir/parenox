use std::fmt;

use super::Type;
use crate::reader::{Form, IntegerLiteralClass, Kind, NumericLiteral, classify_number_with_class};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntegerClass {
    Any,
    Fixed,
    Arbitrary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntegerRange {
    Any,
    Int32,
    /// Fits a signed long, but outside the int range.
    OutsideInt32,
    OutsideInt64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct IntegerType {
    pub class: IntegerClass,
    pub range: IntegerRange,
}

impl IntegerType {
    pub const ANY: Self = Self {
        class: IntegerClass::Any,
        range: IntegerRange::Any,
    };
    pub const FIXED: Self = Self {
        class: IntegerClass::Fixed,
        range: IntegerRange::Any,
    };
    pub const INT32: Self = Self {
        class: IntegerClass::Any,
        range: IntegerRange::Int32,
    };
    pub const FIXED_INT32: Self = Self {
        class: IntegerClass::Fixed,
        range: IntegerRange::Int32,
    };
    pub const OUTSIDE_INT64: Self = Self {
        class: IntegerClass::Arbitrary,
        range: IntegerRange::OutsideInt64,
    };

    pub(super) fn intersection(self, other: Self) -> Option<Self> {
        let class = match (self.class, other.class) {
            (IntegerClass::Any, class) | (class, IntegerClass::Any) => class,
            (a, b) if a == b => a,
            _ => return None,
        };
        let range = match (self.range, other.range) {
            (IntegerRange::Any, range) | (range, IntegerRange::Any) => range,
            (a, b) if a == b => a,
            _ => return None,
        };
        Some(Self { class, range })
    }
}

impl fmt::Display for IntegerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let class = match self.class {
            IntegerClass::Any => "Integer",
            IntegerClass::Fixed => "JVM fixed-precision integer",
            IntegerClass::Arbitrary => "Arbitrary-precision integer",
        };
        match self.range {
            IntegerRange::Any => f.write_str(class),
            IntegerRange::Int32 if self.class == IntegerClass::Any => f.write_str("Int32"),
            IntegerRange::Int32 => write!(f, "{class} in the JVM int range"),
            IntegerRange::OutsideInt32 => write!(f, "{class} outside the JVM int range"),
            IntegerRange::OutsideInt64 => write!(f, "{class} outside the JVM long range"),
        }
    }
}

impl Type {
    pub(crate) fn integer_literal(form: &Form, source: &str) -> Option<Self> {
        let form = form.without_metadata();
        if !matches!(form.kind, Kind::Number) {
            return None;
        }
        let ty = Self::from_numeric_token(&source[form.span.start.offset..form.span.end.offset]);
        matches!(ty, Self::Integer(_)).then_some(ty)
    }

    pub(crate) fn from_numeric_token(token: &str) -> Self {
        match classify_number_with_class(token) {
            (NumericLiteral::Integer(value), Some(class)) => {
                let class = match class {
                    IntegerLiteralClass::Long => IntegerClass::Fixed,
                    IntegerLiteralClass::BigInt => IntegerClass::Arbitrary,
                };
                let Ok(value) = i32::try_from(value) else {
                    return Self::Integral(IntegerType {
                        class,
                        range: IntegerRange::OutsideInt32,
                    });
                };
                let digits = token.strip_prefix(['+', '-']).unwrap_or(token);
                // Keep the existing exact decimal index model.
                if class == IntegerClass::Fixed
                    && (digits.len() == 1 || !digits.starts_with('0'))
                    && digits.bytes().all(|byte| byte.is_ascii_digit())
                {
                    Self::Integer(value)
                } else {
                    Self::Integral(IntegerType {
                        class,
                        range: IntegerRange::Int32,
                    })
                }
            }
            (NumericLiteral::IntegerOutsideI64, _) => Self::Integral(IntegerType::OUTSIDE_INT64),
            (NumericLiteral::NonInteger, _) => Self::NonIntegral,
            _ => Self::Number,
        }
    }

    pub(super) fn integer_type(&self) -> Option<IntegerType> {
        match self {
            Self::Integer(_) => Some(IntegerType::FIXED_INT32),
            Self::Integral(ty) => Some(*ty),
            _ => None,
        }
    }

    pub(crate) fn integer_intersection(&self, other: &Self) -> Option<Self> {
        let intersection = self.integer_type()?.intersection(other.integer_type()?);
        Some(match (intersection, self, other) {
            (None, _, _) => Self::Never,
            (Some(_), Self::Integer(a), Self::Integer(b)) if a != b => Self::Never,
            (Some(_), exact @ Self::Integer(_), _) | (Some(_), _, exact @ Self::Integer(_)) => {
                exact.clone()
            }
            (Some(ty), _, _) => Self::Integral(ty),
        })
    }
}
