//! The numeric token grammar of the Clojure/JVM reader.
use std::num::IntErrorKind;

/// Numeric evidence from a reader-validated token, not its JVM storage class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericLiteral {
    Integer(i64),
    IntegerOutsideI64,
    NonInteger,
    Unclassified,
}

pub(super) fn starts_number(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    unsigned.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

pub(super) fn valid_number(token: &str) -> bool {
    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
    if let Some((value, radix)) = integer_parts(unsigned) {
        return !value.is_empty() && value.chars().all(|digit| digit.is_digit(radix));
    }
    if let Some((numerator, denominator)) = unsigned.split_once('/') {
        return digits(numerator)
            && digits(denominator)
            && denominator.bytes().any(|digit| digit != b'0');
    }
    decimal(unsigned)
}

/// Classify the value of a numeric token already accepted by the reader.
/// Oversized ratio operands remain unclassified rather than approximating
/// divisibility with a lossy floating-point calculation.
pub fn classify_number(token: &str) -> NumericLiteral {
    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
    let negative = token.starts_with('-');
    if let Some((value, radix)) = integer_parts(unsigned) {
        return match u128::from_str_radix(value, radix) {
            Ok(magnitude) => bounded_integer(negative, magnitude),
            Err(error) if *error.kind() == IntErrorKind::PosOverflow => {
                NumericLiteral::IntegerOutsideI64
            }
            Err(_) => NumericLiteral::Unclassified,
        };
    }
    if let Some((numerator, denominator)) = unsigned.split_once('/') {
        return match (numerator.parse::<u128>(), denominator.parse::<u128>()) {
            (Ok(numerator), Ok(denominator)) if denominator != 0 => {
                if numerator.is_multiple_of(denominator) {
                    bounded_integer(negative, numerator / denominator)
                } else {
                    NumericLiteral::NonInteger
                }
            }
            _ => NumericLiteral::Unclassified,
        };
    }
    NumericLiteral::NonInteger
}

fn bounded_integer(negative: bool, magnitude: u128) -> NumericLiteral {
    if negative && magnitude == i64::MAX as u128 + 1 {
        return NumericLiteral::Integer(i64::MIN);
    }
    i64::try_from(magnitude).map_or(NumericLiteral::IntegerOutsideI64, |value| {
        NumericLiteral::Integer(if negative { -value } else { value })
    })
}

pub(super) fn integer_key(token: &str) -> Option<(bool, u128)> {
    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
    let (digits, radix) = integer_parts(unsigned)?;
    let magnitude = u128::from_str_radix(digits, radix).ok()?;
    Some((token.starts_with('-') && magnitude != 0, magnitude))
}

fn integer_parts(unsigned: &str) -> Option<(&str, u32)> {
    let integer = unsigned.strip_suffix('N').unwrap_or(unsigned);
    if digits(integer) {
        Some((integer, if integer.starts_with('0') { 8 } else { 10 }))
    } else if let Some(hex) = integer
        .strip_prefix("0x")
        .or_else(|| integer.strip_prefix("0X"))
    {
        Some((hex, 16))
    } else {
        let (base, value) = unsigned.split_once(['r', 'R'])?;
        let radix = base.parse().ok().filter(|radix| {
            base.len() <= 2 && !base.starts_with('0') && (2..=36).contains(radix)
        })?;
        Some((value, radix))
    }
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|digit| digit.is_ascii_digit())
}

fn decimal(unsigned: &str) -> bool {
    let big = unsigned.strip_suffix('M');
    let text = big.unwrap_or(unsigned);
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let unsigned_exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
    if !digits(unsigned_exponent) {
        return false;
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if !digits(whole) || !(fraction.is_empty() || digits(fraction)) {
        return false;
    }
    big.is_none()
        || exponent.parse::<i32>().is_ok_and(|exponent| {
            // BigDecimal's scale is the number of fraction digits minus the exponent.
            i32::try_from(fraction.len())
                .is_ok_and(|fraction| fraction.checked_sub(exponent).is_some())
        })
}
