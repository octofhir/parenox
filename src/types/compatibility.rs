//! Structural compatibility distinguishes a known mismatch from missing type information.
mod explain;

use std::borrow::Cow;

use super::{Signature, Type};
use crate::{ExternMismatch, ExternMismatchReason};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Compatibility {
    Mismatch,
    Unproven,
    Proven,
}

impl Compatibility {
    pub(super) fn and(self, other: impl FnOnce() -> Self) -> Self {
        if self == Self::Mismatch {
            self
        } else {
            self.min(other())
        }
    }

    pub(super) fn all(checks: impl Iterator<Item = Self>) -> Self {
        let mut result = Self::Proven;
        for check in checks {
            result = result.min(check);
            if result == Self::Mismatch {
                break;
            }
        }
        result
    }

    fn any(checks: impl Iterator<Item = Self>) -> Self {
        let mut result = Self::Mismatch;
        for check in checks {
            result = result.max(check);
            if result == Self::Proven {
                break;
            }
        }
        result
    }
}

impl From<bool> for Compatibility {
    fn from(proven: bool) -> Self {
        if proven { Self::Proven } else { Self::Mismatch }
    }
}

impl Type {
    pub fn subtype_of(&self, expected: &Type) -> bool {
        self.compatibility(expected) == Compatibility::Proven
    }

    pub(crate) fn compatibility(&self, expected: &Type) -> Compatibility {
        use Compatibility::{Mismatch, Proven, Unproven};
        match (self, expected) {
            (Type::Never, _) => Proven,
            (Type::Unknown, _) | (_, Type::Unknown) => Unproven,
            (Type::Union(parts), expected) => {
                Compatibility::all(parts.iter().map(|actual| actual.compatibility(expected)))
            }
            (actual, Type::Union(parts)) => {
                Compatibility::any(parts.iter().map(|expected| actual.compatibility(expected)))
            }
            (Type::Parameter(actual), Type::Parameter(expected)) => (actual == expected).into(),
            (Type::Parameter(actual), expected) => actual
                .upper
                .as_ref()
                .map_or(Mismatch, |upper| upper.compatibility(expected)),
            (Type::Integer(_), Type::Number)
            | (Type::Str(_), Type::String)
            | (Type::Char(_), Type::Character)
            | (Type::Key(_), Type::Keyword)
            | (Type::ExceptionInfo(_), Type::Throwable)
            | (Type::Bool(_), Type::Boolean) => Proven,
            (Type::MapEntry(a), Type::MapEntry(e)) => positions(a.as_slice(), e.as_slice()),
            (Type::MapEntry(a), Type::Tuple(e)) => positions(a.as_slice(), e),
            (Type::Tuple(a), Type::Tuple(e)) | (Type::SeqTuple(a), Type::SeqTuple(e)) => {
                positions(a, e)
            }
            (Type::MapEntry(items), Type::Vector(e)) => {
                Compatibility::all(items.iter().map(|item| item.compatibility(e)))
            }
            (Type::Tuple(items), Type::Vector(e)) | (Type::SeqTuple(items), Type::Seq(e)) => {
                if items.is_empty() && matches!(self, Type::SeqTuple(_)) {
                    Mismatch
                } else {
                    Compatibility::all(items.iter().map(|item| item.compatibility(e)))
                }
            }
            (Type::Vector(a), Type::Vector(e))
            | (Type::Set(a), Type::Set(e))
            | (Type::Seq(a), Type::Seq(e))
            | (Type::Reduced(a), Type::Reduced(e))
            | (Type::ExceptionInfo(a), Type::ExceptionInfo(e)) => a.compatibility(e),
            (Type::Function(actual), Type::Function(expected)) => Compatibility::all(
                expected
                    .iter()
                    .map(|e| Compatibility::any(actual.iter().map(|a| signature(a, e)))),
            ),
            (Type::Map(a), Type::Map(e)) => a.compatibility(e),
            _ => (self == expected).into(),
        }
    }
}

fn positions(actual: &[Type], expected: &[Type]) -> Compatibility {
    Compatibility::from(actual.len() == expected.len())
        .and(|| Compatibility::all(actual.iter().zip(expected).map(|(a, e)| a.compatibility(e))))
}

fn signature(actual: &Signature, expected: &Signature) -> Compatibility {
    if actual.parameters.is_some() && expected.parameters.is_none() {
        return match actual.specialize_callback(expected) {
            Ok(specialized) => signature(&specialized, expected),
            Err(result) => result,
        };
    }
    if actual.parameters.is_some() || expected.parameters.is_some() {
        return if actual.same_type(expected) && !actual.types().any(Type::contains_unknown) {
            Compatibility::Proven
        } else {
            Compatibility::Unproven
        };
    }
    Compatibility::from(actual.arity() == expected.arity())
        .and(|| positions(&expected.args, &actual.args))
        .and(|| {
            expected
                .rest
                .as_ref()
                .zip(actual.rest.as_ref())
                .map_or(Compatibility::Proven, |(e, a)| e.compatibility(a))
        })
        .and(|| actual.ret.compatibility(&expected.ret))
}

/// Check the clause actually selected at every arity promised by an extern.
/// Beyond the largest fixed prefix, both sides repeat the same rest types.
pub(crate) fn extern_drift(
    actual: &[Signature],
    promised: &[Signature],
) -> Option<(usize, Compatibility, ExternMismatch)> {
    let boundary = actual
        .iter()
        .chain(promised)
        .map(|sig| sig.args.len())
        .max()?
        + 1;
    let mut unproven = None;
    for count in 0..=boundary {
        let Some(expected) = active_clause(promised, count) else {
            continue;
        };
        let Some(active) = active_clause(actual, count) else {
            return Some((
                count,
                Compatibility::Mismatch,
                ExternMismatch::new(ExternMismatchReason::MissingArity),
            ));
        };
        match signature(&active, &expected) {
            outcome @ Compatibility::Mismatch => {
                return Some((
                    count,
                    outcome,
                    explain::signature_failure(&active, &expected, outcome, false),
                ));
            }
            Compatibility::Unproven => {
                unproven.get_or_insert(count);
            }
            Compatibility::Proven => {}
        }
    }
    unproven.map(|count| {
        let active = active_clause(actual, count).expect("unproven clause exists");
        let expected = active_clause(promised, count).expect("promised clause exists");
        let outcome = Compatibility::Unproven;
        (
            count,
            outcome,
            explain::signature_failure(&active, &expected, outcome, false),
        )
    })
}

fn active_clause(signatures: &[Signature], count: usize) -> Option<Cow<'_, Signature>> {
    // Contracts store fixed clauses before the variadic fallback, like a call.
    let signature = signatures.iter().find(|sig| sig.arity().accepts(count))?;
    match &signature.rest {
        None => Some(Cow::Borrowed(signature)),
        Some(rest) => {
            let mut args = signature.args.clone();
            args.resize(count, rest.clone());
            Some(Cow::Owned(Signature {
                args,
                rest: None,
                ret: signature.ret.clone(),
                parameters: None,
            }))
        }
    }
}
