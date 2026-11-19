//! The numeric token grammar of the Clojure/JVM reader.
pub(super) fn starts_number(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    unsigned.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

pub(super) fn valid_number(token: &str) -> bool {
    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
    let integer = unsigned.strip_suffix('N').unwrap_or(unsigned);
    if digits(integer) {
        return !integer.starts_with('0')
            || integer.bytes().all(|digit| matches!(digit, b'0'..=b'7'));
    }
    if let Some(hex) = integer
        .strip_prefix("0x")
        .or_else(|| integer.strip_prefix("0X"))
    {
        return !hex.is_empty() && hex.bytes().all(|digit| digit.is_ascii_hexdigit());
    }
    if let Some((base, value)) = unsigned.split_once(['r', 'R']) {
        if base.len() > 2 || base.starts_with('0') {
            return false;
        }
        return base.parse::<u32>().is_ok_and(|base| {
            (2..=36).contains(&base)
                && !value.is_empty()
                && value.chars().all(|digit| digit.is_digit(base))
        });
    }
    if let Some((numerator, denominator)) = unsigned.split_once('/') {
        return digits(numerator)
            && digits(denominator)
            && denominator.bytes().any(|digit| digit != b'0');
    }
    decimal(unsigned)
}

pub(super) fn integer_key(token: &str) -> Option<(bool, u128)> {
    let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
    let (digits, radix) = if let Some((base, value)) = unsigned.split_once(['r', 'R']) {
        let base = base.parse().ok().filter(|base| (2..=36).contains(base))?;
        (value, base)
    } else {
        let integer = unsigned.strip_suffix('N').unwrap_or(unsigned);
        if let Some(hex) = integer
            .strip_prefix("0x")
            .or_else(|| integer.strip_prefix("0X"))
        {
            (hex, 16)
        } else if integer.starts_with('0') {
            (integer, 8)
        } else {
            (integer, 10)
        }
    };
    let magnitude = u128::from_str_radix(digits, radix).ok()?;
    Some((token.starts_with('-') && magnitude != 0, magnitude))
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
