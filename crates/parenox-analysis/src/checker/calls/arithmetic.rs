//! JVM fixed-precision arithmetic shared by ordinary and prepared calls.
use super::Value;
use crate::reader::Span;
use crate::types::{IntegerClass, IntegerType, Type};

pub(super) struct FixedResult {
    pub ty: Type,
    pub may_overflow: bool,
}

pub(super) fn fixed_result(name: &str, args: &[(Value, Span)]) -> Option<FixedResult> {
    let operation = match name {
        "clojure.core/+" => Operation::Add,
        "clojure.core/-" => Operation::Subtract,
        "clojure.core/*" => Operation::Multiply,
        "clojure.core/inc" => Operation::Increment,
        "clojure.core/dec" => Operation::Decrement,
        _ => return None,
    };
    let fixed = Type::Integral(IntegerType::FIXED);
    if !args.iter().all(|(value, _)| value.known.subtype_of(&fixed)) {
        return None;
    }
    if matches!(operation, Operation::Add | Operation::Multiply)
        && let [(value, _)] = args
    {
        return Some(FixedResult {
            ty: value.known.clone(),
            may_overflow: false,
        });
    }
    let exact = args.iter().map(|(value, _)| value.known.exact_long_value());
    match operation.fold(exact) {
        Some(value) => Some(FixedResult {
            ty: Type::exact_long(value, IntegerClass::Fixed),
            may_overflow: false,
        }),
        // Both checked and unchecked compilation keep the fixed class on
        // a normal return. The compile-time flag controls overflow behavior.
        None => Some(FixedResult {
            ty: fixed,
            may_overflow: true,
        }),
    }
}

enum Operation {
    Add,
    Subtract,
    Multiply,
    Increment,
    Decrement,
}

impl Operation {
    fn fold(self, mut values: impl ExactSizeIterator<Item = Option<i64>>) -> Option<i64> {
        match self {
            Self::Add => values.try_fold(0_i64, |sum, value| sum.checked_add(value?)),
            Self::Multiply => values.try_fold(1_i64, |product, value| product.checked_mul(value?)),
            Self::Subtract => {
                let first = values.next()??;
                if values.len() == 0 {
                    first.checked_neg()
                } else {
                    values.try_fold(first, |difference, value| difference.checked_sub(value?))
                }
            }
            Self::Increment => values.next()??.checked_add(1),
            Self::Decrement => values.next()??.checked_sub(1),
        }
    }
}
