//! JVM fixed-precision arithmetic shared by ordinary and prepared calls.
use super::{Checker, Env, Value};
use crate::reader::{Form, Span};
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

impl Checker<'_> {
    pub(super) fn direct_extremum(
        &mut self,
        name: &str,
        forms: &[Form],
        span: Span,
        env: &Env,
    ) -> Value {
        let Some((first, rest)) = forms.split_first() else {
            return self.builtin(name, &[], span);
        };
        let mut result = (self.infer(first, env), first.span);
        for form in rest {
            // The JVM compiler inlines direct variadic min/max as nested
            // comparisons. A failed pair prevents evaluation of later operands.
            let next = self.infer_path(form, env, result.0.known != Type::Never, false);
            if next.known == Type::Never {
                result.0 = next;
                continue;
            }
            let start = self.diagnostics.len();
            result.0 = self.extremum_pair(name, &result, &(next, form.span));
            self.note_builtin_contract(start, name);
        }
        result.0
    }

    pub(super) fn extremum(&mut self, name: &str, args: &[(Value, Span)]) -> Value {
        let (first, rest) = args.split_first().expect("min/max arity was checked");
        let mut result = first.clone();
        for next in rest {
            result.0 = self.extremum_pair(name, &result, next);
            if result.0.known == Type::Never {
                break;
            }
        }
        result.0
    }

    pub(super) fn open_extremum(
        &mut self,
        name: &str,
        prefix: &[(Value, Span)],
        element: &Value,
        minimum: usize,
    ) -> Value {
        // Realizing the spread can throw even when every comparison is safe.
        self.may_throw();
        let prefix_result = (!prefix.is_empty()).then(|| self.extremum(name, prefix));
        if let Some(failed) = prefix_result
            .as_ref()
            .filter(|value| value.known == Type::Never)
        {
            return failed.clone();
        }
        // An empty spread can leave any prefix arity. With no prefix, the
        // first spread item can be returned without a numeric comparison.
        let short_result = match &prefix_result {
            Some(value) if minimum == prefix.len() => value.clone(),
            None if minimum <= 1 => element.clone(),
            _ => Value::new(Type::Never),
        };
        let mut operands = prefix_result.iter().chain(std::iter::once(element));
        let multi = if let Some(fixed) = fixed_domain(operands.clone()) {
            fixed
        } else if operands.clone().all(excludes_nan) && operands.any(excludes_number) {
            Type::Never
        } else {
            Type::Number
        };
        Value::new(multi).join(short_result)
    }

    fn extremum_pair(&mut self, name: &str, left: &(Value, Span), right: &(Value, Span)) -> Value {
        let operands = [left, right];
        if let Some(fixed) = fixed_domain(operands.into_iter().map(|(value, _)| value)) {
            let ty = match (
                left.0.known.exact_long_value(),
                right.0.known.exact_long_value(),
            ) {
                (Some(left), Some(right)) => {
                    let selected = if name == "clojure.core/min" {
                        left.min(right)
                    } else {
                        left.max(right)
                    };
                    Type::exact_long(selected, IntegerClass::Fixed)
                }
                _ => fixed,
            };
            return Value::new(ty);
        }

        self.may_throw();
        // The JVM propagates NaN before casting the other operand. Avoid
        // definite type errors while either operand may be non-integral.
        if operands.iter().all(|(value, _)| excludes_nan(value)) {
            let mut invalid = false;
            for (value, span) in operands {
                self.expect(value, &Type::Number, *span, name);
                invalid |= excludes_number(value);
            }
            if invalid {
                return Value::new(Type::Never);
            }
        }
        Value::new(Type::Number)
    }
}

fn fixed_domain<'a>(mut operands: impl Iterator<Item = &'a Value> + Clone) -> Option<Type> {
    let narrow = Type::Integral(IntegerType::FIXED_INT32);
    let fixed = Type::Integral(IntegerType::FIXED);
    if operands
        .clone()
        .all(|value| value.known.subtype_of(&narrow))
    {
        Some(narrow)
    } else {
        operands
            .all(|value| value.known.subtype_of(&fixed))
            .then_some(fixed)
    }
}

fn excludes_nan(value: &Value) -> bool {
    value.literal_container.is_some()
        || value.known.known_intersection(&Type::NonIntegral) == Some(Type::Never)
}

pub(super) fn excludes_number(value: &Value) -> bool {
    value.literal_container.is_some()
        || value.known.known_intersection(&Type::Number) == Some(Type::Never)
}
