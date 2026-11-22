//! The numeric token grammar of the Clojure/JVM reader.
use std::num::IntErrorKind;

use num_bigint::BigUint;
use num_integer::Integer;

/// Numeric evidence from a reader-validated token, not its JVM storage class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericLiteral {
    Integer(i64),
    IntegerOutsideI64,
    NonInteger,
    Unclassified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegerLiteralClass {
    Long,
    BigInt,
}

pub(super) fn starts_number(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    match unsigned.as_bytes().first() {
        Some(b @ 0..=127) => b.is_ascii_digit(),
        Some(_) => unsigned.chars().next().and_then(decimal_digit).is_some(),
        None => false,
    }
}

pub(super) fn decimal_digit(ch: char) -> Option<u32> {
    if ch.is_ascii() {
        return ch.to_digit(10);
    }
    // The JVM reader classifies one UTF-16 unit. These are the non-ASCII
    // decimal zeroes in the BMP.
    const ZEROES: [u32; 36] = [
        0x0660, 0x06f0, 0x07c0, 0x0966, 0x09e6, 0x0a66, 0x0ae6, 0x0b66, 0x0be6, 0x0c66, 0x0ce6,
        0x0d66, 0x0de6, 0x0e50, 0x0ed0, 0x0f20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0,
        0x1a80, 0x1a90, 0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0,
        0xaa50, 0xabf0, 0xff10,
    ];
    let ch = u32::from(ch);
    let index = ZEROES.partition_point(|&zero| zero <= ch);
    let value = ch - ZEROES[index.checked_sub(1)?];
    (value <= 9).then_some(value)
}

pub(super) fn escape_digit(ch: char, radix: u32) -> Option<u32> {
    let value = match ch {
        ch if ch.is_ascii() => return ch.to_digit(radix),
        'Ａ'..='Ｆ' => u32::from(ch) - u32::from('Ａ') + 10,
        'ａ'..='ｆ' => u32::from(ch) - u32::from('ａ') + 10,
        _ => decimal_digit(ch)?,
    };
    (value < radix).then_some(value)
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
/// Ratio classification uses exact integer reduction at every operand width.
pub fn classify_number(token: &str) -> NumericLiteral {
    classify_number_with_class(token).0
}

/// Return JVM storage evidence for reader-validated integer literals.
pub fn classify_number_with_class(token: &str) -> (NumericLiteral, Option<IntegerLiteralClass>) {
    use IntegerLiteralClass::{BigInt, Long};
    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
    let negative = token.starts_with('-');
    if let Some((value, radix)) = integer_parts(unsigned) {
        let number = match u128::from_str_radix(value, radix) {
            Ok(magnitude) => bounded_integer(negative, magnitude),
            Err(error) if *error.kind() == IntErrorKind::PosOverflow => {
                NumericLiteral::IntegerOutsideI64
            }
            Err(_) => NumericLiteral::Unclassified,
        };
        let class = match number {
            NumericLiteral::Integer(_)
                if !unsigned.contains(['r', 'R']) && unsigned.ends_with('N') =>
            {
                Some(BigInt)
            }
            NumericLiteral::Integer(_) => Some(Long),
            NumericLiteral::IntegerOutsideI64 => Some(BigInt),
            _ => None,
        };
        return (number, class);
    }
    if let Some((numerator, denominator)) = unsigned.split_once('/') {
        return match (numerator.parse::<u128>(), denominator.parse::<u128>()) {
            (Ok(numerator), Ok(denominator)) if denominator != 0 => {
                if numerator.is_multiple_of(denominator) {
                    // LispReader divides through Numbers. Even integral ratios
                    // take its BigInt path with oversized operands or Long/MIN_VALUE.
                    let class = if numerator > i64::MAX as u128 || denominator > i64::MAX as u128 {
                        BigInt
                    } else {
                        Long
                    };
                    (
                        bounded_integer(negative, numerator / denominator),
                        Some(class),
                    )
                } else {
                    (NumericLiteral::NonInteger, None)
                }
            }
            _ => classify_big_ratio(negative, numerator, denominator)
                .unwrap_or((NumericLiteral::Unclassified, None)),
        };
    }
    (NumericLiteral::NonInteger, None)
}

fn bounded_integer(negative: bool, magnitude: u128) -> NumericLiteral {
    if negative && magnitude == i64::MAX as u128 + 1 {
        return NumericLiteral::Integer(i64::MIN);
    }
    i64::try_from(magnitude).map_or(NumericLiteral::IntegerOutsideI64, |value| {
        NumericLiteral::Integer(if negative { -value } else { value })
    })
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum NumberKey<'a> {
    Integer(bool, u128),
    BigInteger(bool, BigUint),
    Ratio(bool, u128, u128),
    BigRatio(bool, BigUint, BigUint),
    Double(u64),
    Decimal(bool, String, i64),
    Spelling(&'a str),
}

pub(super) fn number_key(token: &str) -> NumberKey<'_> {
    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
    let negative = token.starts_with('-');
    if let Some((digits, radix)) = integer_parts(unsigned) {
        return match u128::from_str_radix(digits, radix) {
            Ok(magnitude) => NumberKey::Integer(negative && magnitude != 0, magnitude),
            Err(_) => BigUint::parse_bytes(digits.as_bytes(), radix)
                .map_or(NumberKey::Spelling(token), |value| {
                    NumberKey::BigInteger(negative, value)
                }),
        };
    }
    if let Some((numerator, denominator)) = unsigned.split_once('/') {
        return match (numerator.parse::<u128>(), denominator.parse::<u128>()) {
            (Ok(numerator), Ok(denominator)) if denominator != 0 => {
                let divisor = numerator.gcd(&denominator);
                if denominator == divisor {
                    NumberKey::Integer(negative && numerator != 0, numerator / divisor)
                } else {
                    NumberKey::Ratio(negative, numerator / divisor, denominator / divisor)
                }
            }
            _ => big_ratio_key(negative, numerator, denominator)
                .unwrap_or(NumberKey::Spelling(token)),
        };
    }
    if let Some(decimal) = unsigned.strip_suffix('M') {
        return decimal_key(decimal, negative).unwrap_or(NumberKey::Spelling(token));
    }
    token
        .parse::<f64>()
        .map(double_key)
        .unwrap_or(NumberKey::Spelling(token))
}

pub(super) fn double_key(value: f64) -> NumberKey<'static> {
    NumberKey::Double(if value == 0.0 { 0 } else { value.to_bits() })
}

#[cold]
fn classify_big_ratio(
    negative: bool,
    numerator: &str,
    denominator: &str,
) -> Option<(NumericLiteral, Option<IntegerLiteralClass>)> {
    let (numerator, denominator) = big_ratio_operands(numerator, denominator)?;
    let (quotient, remainder) = numerator.div_rem(&denominator);
    if remainder != BigUint::ZERO {
        return Some((NumericLiteral::NonInteger, None));
    }
    let number = u128::try_from(quotient).map_or(NumericLiteral::IntegerOutsideI64, |magnitude| {
        bounded_integer(negative, magnitude)
    });
    Some((number, Some(IntegerLiteralClass::BigInt)))
}

fn big_ratio_operands(numerator: &str, denominator: &str) -> Option<(BigUint, BigUint)> {
    let numerator = BigUint::parse_bytes(numerator.as_bytes(), 10)?;
    let denominator = BigUint::parse_bytes(denominator.as_bytes(), 10)?;
    (denominator != BigUint::ZERO).then_some((numerator, denominator))
}

#[cold]
fn big_ratio_key(negative: bool, numerator: &str, denominator: &str) -> Option<NumberKey<'static>> {
    let (mut numerator, mut denominator) = big_ratio_operands(numerator, denominator)?;
    let divisor = numerator.gcd(&denominator);
    numerator /= &divisor;
    denominator /= divisor;
    let small_numerator = u128::try_from(&numerator).ok();
    let small_denominator = u128::try_from(&denominator).ok();
    Some(match (small_numerator, small_denominator) {
        (Some(value), Some(1)) => NumberKey::Integer(negative && value != 0, value),
        (None, Some(1)) => NumberKey::BigInteger(negative, numerator),
        (Some(num), Some(den)) => NumberKey::Ratio(negative, num, den),
        _ => NumberKey::BigRatio(negative, numerator, denominator),
    })
}

fn decimal_key(text: &str, negative: bool) -> Option<NumberKey<'_>> {
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let without_trailing_zeroes = digits.trim_end_matches('0');
    if without_trailing_zeroes.is_empty() {
        return Some(NumberKey::Decimal(false, String::new(), 0));
    }
    let power = exponent.parse::<i64>().ok()? - fraction.len() as i64
        + (digits.len() - without_trailing_zeroes.len()) as i64;
    Some(NumberKey::Decimal(
        negative,
        without_trailing_zeroes.trim_start_matches('0').into(),
        power,
    ))
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
