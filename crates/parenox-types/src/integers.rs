use std::fmt;

use super::Type;
use parenox_frontend::reader::{
    Form, IntegerLiteralClass, Kind, NumericLiteral, classify_number_with_class,
};

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
    /// The range guaranteed by java.lang.Integer.valueOf.
    IntCache,
    Exact(i32),
    /// An exact signed long outside the int range.
    ExactLong(i64),
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
        let range = self.range.intersection(other.range)?;
        Some(Self { class, range })
    }

    pub fn without_value(self) -> Self {
        match self.range {
            IntegerRange::Exact(value) => Self {
                range: IntegerRange::for_int(value),
                ..self
            },
            IntegerRange::ExactLong(_) => Self {
                range: IntegerRange::OutsideInt32,
                ..self
            },
            _ => self,
        }
    }
}

impl IntegerRange {
    pub(super) fn intersection(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Any, range) | (range, Self::Any) => Some(range),
            (Self::Int32, Self::IntCache) | (Self::IntCache, Self::Int32) => Some(Self::IntCache),
            (Self::Int32, exact @ Self::Exact(_)) | (exact @ Self::Exact(_), Self::Int32) => {
                Some(exact)
            }
            (Self::IntCache, exact @ Self::Exact(value))
            | (exact @ Self::Exact(value), Self::IntCache)
                if Self::for_int(value) == Self::IntCache =>
            {
                Some(exact)
            }
            (Self::OutsideInt32, exact @ Self::ExactLong(_))
            | (exact @ Self::ExactLong(_), Self::OutsideInt32) => Some(exact),
            (a, b) if a == b => Some(a),
            _ => None,
        }
    }

    fn for_int(value: i32) -> Self {
        if i8::try_from(value).is_ok() {
            Self::IntCache
        } else {
            Self::Int32
        }
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
            IntegerRange::Int32 | IntegerRange::IntCache if self.class == IntegerClass::Any => {
                f.write_str("Int32")
            }
            IntegerRange::Int32 | IntegerRange::IntCache => {
                write!(f, "{class} in the JVM int range")
            }
            IntegerRange::Exact(value) => write!(f, "{class} with value {value}"),
            IntegerRange::ExactLong(value) => write!(f, "{class} with value {value}"),
            IntegerRange::OutsideInt32 => write!(f, "{class} outside the JVM int range"),
            IntegerRange::OutsideInt64 => write!(f, "{class} outside the JVM long range"),
        }
    }
}

impl Type {
    pub fn exact_integer(value: i32, class: IntegerClass) -> Self {
        if class == IntegerClass::Fixed {
            Self::Integer(value)
        } else {
            Self::Integral(IntegerType {
                class,
                range: IntegerRange::Exact(value),
            })
        }
    }

    /// Keep exact values in the ordinary int representation when they fit.
    pub fn exact_long(value: i64, class: IntegerClass) -> Self {
        match i32::try_from(value) {
            Ok(value) => Self::exact_integer(value, class),
            Err(_) => Self::Integral(IntegerType {
                class,
                range: IntegerRange::ExactLong(value),
            }),
        }
    }

    pub fn integer_literal(form: &Form, source: &str) -> Option<Self> {
        let form = form.without_metadata();
        if !matches!(form.kind, Kind::Number) {
            return None;
        }
        let ty = Self::from_numeric_token(&source[form.span.start.offset..form.span.end.offset]);
        matches!(ty, Self::Integer(_)).then_some(ty)
    }

    /// Extract a canonical base-ten literal in the JVM signed long range.
    pub fn decimal_long_literal(form: &Form, source: &str) -> Option<i64> {
        let form = form.without_metadata();
        if !matches!(form.kind, Kind::Number) {
            return None;
        }
        let token = &source[form.span.start.offset..form.span.end.offset];
        token
            .parse::<i64>()
            .ok()
            .filter(|value| value.to_string() == token)
    }

    pub fn from_numeric_token(token: &str) -> Self {
        match classify_number_with_class(token) {
            (NumericLiteral::Integer(value), Some(class)) => {
                let class = match class {
                    IntegerLiteralClass::Long => IntegerClass::Fixed,
                    IntegerLiteralClass::BigInt => IntegerClass::Arbitrary,
                };
                let Ok(narrow) = i32::try_from(value) else {
                    return Self::Integral(IntegerType {
                        class,
                        range: IntegerRange::ExactLong(value),
                    });
                };
                let digits = token.strip_prefix(['+', '-']).unwrap_or(token);
                // Keep the existing exact decimal index model.
                if class == IntegerClass::Fixed
                    && (digits.len() == 1 || !digits.starts_with('0'))
                    && digits.bytes().all(|byte| byte.is_ascii_digit())
                {
                    Self::Integer(narrow)
                } else {
                    Self::Integral(IntegerType {
                        class,
                        range: IntegerRange::Exact(narrow),
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
            Self::Integer(value) => Some(IntegerType {
                class: IntegerClass::Fixed,
                range: IntegerRange::Exact(*value),
            }),
            Self::Integral(ty) => Some(*ty),
            _ => None,
        }
    }

    pub fn fits_integer_cache(&self) -> bool {
        self.integer_type()
            .is_some_and(|ty| ty.without_value().range == IntegerRange::IntCache)
    }

    pub fn exact_integer_value(&self) -> Option<i32> {
        match self.integer_type()?.range {
            IntegerRange::Exact(value) => Some(value),
            _ => None,
        }
    }

    pub fn exact_long_value(&self) -> Option<i64> {
        match self.integer_type()?.range {
            IntegerRange::Exact(value) => Some(i64::from(value)),
            IntegerRange::ExactLong(value) => Some(value),
            _ => None,
        }
    }

    pub fn integer_intersection(&self, other: &Self) -> Option<Self> {
        let intersection = self.integer_type()?.intersection(other.integer_type()?);
        Some(match intersection {
            None => Self::Never,
            Some(IntegerType {
                class,
                range: IntegerRange::Exact(value),
            }) => Self::exact_integer(value, class),
            Some(ty) => Self::Integral(ty),
        })
    }

    pub fn is_arbitrary_integer(&self) -> bool {
        self.integer_type()
            .is_some_and(|ty| ty.class == IntegerClass::Arbitrary)
    }
}
