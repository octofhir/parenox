//! Structural compatibility distinguishes a known mismatch from missing type information.
use super::{Signature, Type};

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
