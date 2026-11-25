//! Structural compatibility distinguishes a known mismatch from missing type information.
mod explain;

use std::borrow::Cow;

use super::{JvmArrayClass, SPEC_INVALID, Signature, Type};
use crate::{ExternMismatch, ExternMismatchReason};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Compatibility {
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

    /// Combine checks for a domain whose alternatives must all be compatible.
    pub fn all(checks: impl Iterator<Item = Self>) -> Self {
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

    /// Follow one failed obligation in a definite mismatch.
    pub fn explain_mismatch(&self, expected: &Type) -> Option<ExternMismatch> {
        (self.compatibility(expected) == Compatibility::Mismatch)
            .then(|| explain::type_failure(self, expected, Compatibility::Mismatch))
    }

    /// Intersect types whose overlap can be represented precisely.
    pub fn known_intersection(&self, other: &Type) -> Option<Type> {
        if self.subtype_of(other) {
            return Some(self.clone());
        }
        if other.subtype_of(self) {
            return Some(other.clone());
        }
        match (self, other) {
            (Type::SpecTruthy, ty) | (ty, Type::SpecTruthy) => return ty.spec_truthy(),
            (Type::SpecValue, ty) | (ty, Type::SpecValue) => return ty.spec_value(),
            _ => {}
        }
        let left = scalar_members(self)?;
        let right = scalar_members(other)?;
        if left.len().saturating_mul(right.len()) > 256 {
            return None;
        }
        Some(Type::union(left.iter().flat_map(|a| {
            right.iter().filter_map(move |b| {
                a.integer_intersection(b)
                    .or_else(|| a.subtype_of(b).then(|| a.clone()))
                    .or_else(|| b.subtype_of(a).then(|| b.clone()))
            })
        })))
    }

    /// Possible stored values matching a lookup in an ordinary hash set.
    pub fn hash_set_member_type(&self, query: &Type) -> Type {
        if matches!(self, Type::Never) || matches!(query, Type::Never) {
            return Type::Never;
        }
        let (Some(stored), Some(queries)) = (scalar_members(self), scalar_members(query)) else {
            // Equality can cross different collection representations, e.g.
            // persistent lists and vectors. Keep the stored element bound.
            return self.clone();
        };
        if stored.len().saturating_mul(queries.len()) > 256 {
            return self.clone();
        }
        let numeric = |ty: &Type| {
            matches!(
                ty,
                Type::Number | Type::Integral(_) | Type::Integer(_) | Type::NonIntegral
            )
        };
        Type::union(stored.iter().flat_map(|element| {
            queries.iter().filter_map(move |key| {
                if numeric(element) && numeric(key) {
                    // Equality can cross JVM integer classes. Returning the
                    // searched key would lose the stored representation.
                    element.key_overlaps(key).then(|| element.clone())
                } else {
                    element.known_intersection(key)
                }
            })
        }))
    }

    pub fn compatibility(&self, expected: &Type) -> Compatibility {
        use Compatibility::{Mismatch, Proven, Unproven};
        match (self, expected) {
            (Type::Never, _) | (_, Type::Any) => Proven,
            (Type::Unknown, _) | (_, Type::Unknown) => Unproven,
            (Type::Any, Type::Never) => Mismatch,
            (Type::Any, _) => Unproven,
            (
                Type::BoundedCollection(actual, actual_bounds),
                Type::BoundedCollection(expected, bounds),
            ) => actual
                .compatibility(expected)
                .and(|| actual_bounds.compatibility(*bounds)),
            (Type::BoundedCollection(actual, _), Type::Union(parts)) => {
                let bounded = Compatibility::any(parts.iter().map(|part| self.compatibility(part)));
                if bounded == Proven || !matches!(actual.as_ref(), Type::Union(_)) {
                    bounded
                } else {
                    // The collection's shapes may match different union alternatives.
                    bounded.max(actual.compatibility(expected))
                }
            }
            (Type::BoundedCollection(actual, _), expected) => actual.compatibility(expected),
            (Type::Union(parts), expected) => {
                Compatibility::all(parts.iter().map(|actual| actual.compatibility(expected)))
            }
            (actual, Type::Union(parts)) => {
                Compatibility::any(parts.iter().map(|expected| actual.compatibility(expected)))
            }
            (Type::Parameter(actual), Type::Parameter(expected)) => (actual == expected).into(),
            (Type::Parameter(actual), expected) => actual.upper.as_ref().map_or_else(
                || {
                    if matches!(expected, Type::SpecValue | Type::SpecTruthy) {
                        Unproven
                    } else {
                        Mismatch
                    }
                },
                |upper| upper.compatibility(expected),
            ),
            (Type::Nil | Type::Bool(false), Type::SpecTruthy)
            | (Type::SpecTruthy, Type::Nil | Type::Bool(false))
            | (Type::SpecValue | Type::SpecTruthy, Type::Never) => Mismatch,
            (Type::Key(key), Type::SpecValue | Type::SpecTruthy)
            | (Type::SpecValue | Type::SpecTruthy, Type::Key(key))
                if key == SPEC_INVALID =>
            {
                Mismatch
            }
            (Type::Keyword, Type::SpecValue | Type::SpecTruthy)
            | (Type::Boolean | Type::SpecValue, Type::SpecTruthy) => Unproven,
            (_, Type::SpecValue | Type::SpecTruthy) => Proven,
            (Type::SpecValue | Type::SpecTruthy, _) => Unproven,
            (actual, Type::BoundedCollection(expected, bounds)) => actual
                .compatibility(expected)
                .and(|| actual.length_bounds().compatibility(*bounds)),
            (actual, Type::SpecCollection(element)) => collection(actual, element),
            (
                Type::SpecCollection(_),
                Type::Map(_)
                | Type::MapEntry(_)
                | Type::Vector(_)
                | Type::Tuple(_)
                | Type::Set(_)
                | Type::Seq(_)
                | Type::SeqTuple(_)
                | Type::List(_)
                | Type::ListTuple(_),
            ) => Unproven,
            (Type::Integral(_) | Type::Integer(_) | Type::NonIntegral, Type::Number)
            | (Type::Str(_), Type::String)
            | (Type::Char(_), Type::Character)
            | (Type::Key(_) | Type::SpecKeyword, Type::Keyword)
            | (Type::SimpleSymbol | Type::QualifiedSymbol | Type::Sym(_), Type::Symbol)
            | (Type::ExceptionInfo(_), Type::Throwable)
            | (Type::Bool(_), Type::Boolean) => Proven,
            (Type::Sym(_), Type::SimpleSymbol | Type::QualifiedSymbol) => {
                (self.inferred() == *expected).into()
            }
            (Type::Key(key), Type::SpecKeyword) => (key != SPEC_INVALID).into(),
            (Type::Keyword, Type::SpecKeyword) => Unproven,
            (Type::Number, Type::Integral(_) | Type::Integer(_)) => Unproven,
            (Type::JvmArray(_), Type::JvmArray(JvmArrayClass::Any)) => Proven,
            (Type::JvmArray(JvmArrayClass::Any), Type::JvmArray(_)) => Unproven,
            (Type::Integral(_) | Type::Integer(_), Type::Integral(_) | Type::Integer(_)) => {
                let actual = self.integer_type().unwrap();
                match actual.intersection(expected.integer_type().unwrap()) {
                    Some(overlap) if overlap == actual => Proven,
                    Some(_) => Unproven,
                    None => Mismatch,
                }
            }
            (Type::MapEntry(a), Type::MapEntry(e)) => positions(a.as_slice(), e.as_slice()),
            (Type::MapEntry(a), Type::Tuple(e)) => positions(a.as_slice(), e),
            (Type::Tuple(a), Type::Tuple(e))
            | (Type::SeqTuple(a) | Type::ListTuple(a), Type::SeqTuple(e))
            | (Type::ListTuple(a), Type::ListTuple(e)) => positions(a, e),
            (Type::MapEntry(items), Type::Vector(e)) => {
                Compatibility::all(items.iter().map(|item| item.compatibility(e)))
            }
            (Type::Tuple(items), Type::Vector(e))
            | (Type::SeqTuple(items) | Type::ListTuple(items), Type::Seq(e))
            | (Type::ListTuple(items), Type::List(e)) => {
                if items.is_empty() && !matches!(self, Type::Tuple(_)) {
                    Mismatch
                } else {
                    Compatibility::all(items.iter().map(|item| item.compatibility(e)))
                }
            }
            (Type::Vector(a), Type::Vector(e))
            | (Type::Set(a), Type::Set(e))
            | (Type::Seq(a) | Type::List(a), Type::Seq(e))
            | (Type::List(a), Type::List(e))
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

fn scalar_members(ty: &Type) -> Option<&[Type]> {
    let members = match ty {
        Type::Union(xs) => xs,
        atom => std::slice::from_ref(atom),
    };
    members
        .iter()
        .all(|ty| {
            matches!(
                ty,
                Type::Nil
                    | Type::Boolean
                    | Type::Bool(_)
                    | Type::Number
                    | Type::Integral(_)
                    | Type::NonIntegral
                    | Type::Integer(_)
                    | Type::String
                    | Type::Str(_)
                    | Type::Character
                    | Type::Char(_)
                    | Type::Keyword
                    | Type::SpecKeyword
                    | Type::Key(_)
                    | Type::Symbol
                    | Type::SimpleSymbol
                    | Type::QualifiedSymbol
                    | Type::Sym(_)
                    | Type::Pattern
            )
        })
        .then_some(members)
}

fn collection(actual: &Type, element: &Type) -> Compatibility {
    use Compatibility::{Mismatch, Proven, Unproven};
    match actual {
        Type::SpecCollection(actual) => actual.compatibility(element),
        Type::Map(_) if matches!(element, Type::Any | Type::SpecValue) => Proven,
        Type::Map(map) => match (map.emptiness(), element) {
            (Some(true), _) => Proven,
            // A nonempty map yields entries rather than its keys or values.
            (Some(false), element) if element.excludes_map_entry() => Mismatch,
            _ => Unproven,
        },
        Type::MapEntry(items) => {
            Compatibility::all(items.iter().map(|item| item.compatibility(element)))
        }
        Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => {
            Compatibility::all(items.iter().map(|item| item.compatibility(element)))
        }
        Type::Vector(item) | Type::Set(item) | Type::Seq(item) | Type::List(item) => {
            item.compatibility(element)
        }
        _ => Mismatch,
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
    let specialized = expected
        .args
        .first()
        .and_then(|input| actual.specialize_nil_result(input));
    let result = specialized.as_ref().map_or(&actual.ret, |call| &call.ret);
    Compatibility::from(actual.arity() == expected.arity())
        .and(|| positions(&expected.args, &actual.args))
        .and(|| {
            expected
                .rest
                .as_ref()
                .zip(actual.rest.as_ref())
                .map_or(Compatibility::Proven, |(e, a)| e.compatibility(a))
        })
        .and(|| result.compatibility(&expected.ret))
        .and(|| {
            if expected.nil_preserving && !actual.nil_preserving {
                Compatibility::Unproven
            } else {
                Compatibility::Proven
            }
        })
}

/// Check the clause actually selected at every arity promised by an extern.
/// Beyond the largest fixed prefix, both sides repeat the same rest types.
pub fn extern_drift(
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
                nil_preserving: false,
            }))
        }
    }
}
