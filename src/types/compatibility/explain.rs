//! Follow failed obligations after compatibility has been decided.
//! Normal type checking does not allocate paths or format types on this walk.
use super::{Compatibility, Signature, Type, signature};
use crate::types::{ExtraFields, MapSlot, MapType};
use crate::{ExternMismatch, ExternMismatchReason as Reason, ExternPathStep as Step};

pub(super) fn signature_failure(
    actual: &Signature,
    expected: &Signature,
    outcome: Compatibility,
    is_callback: bool,
) -> ExternMismatch {
    if actual.parameters.is_some() && expected.parameters.is_none() {
        return match actual.specialize_callback(expected) {
            Ok(specialized) => signature_failure(&specialized, expected, outcome, is_callback),
            Err(Compatibility::Mismatch) => ExternMismatch::new(Reason::GenericSpecialization),
            Err(_) => ExternMismatch::new(Reason::GenericUnproven),
        };
    }
    if actual.parameters.is_some() || expected.parameters.is_some() {
        return ExternMismatch::new(Reason::GenericUnproven);
    }
    if actual.arity() != expected.arity() {
        return ExternMismatch::new(Reason::CallbackArity);
    }
    // Arguments are contravariant: values accepted by the promise must fit
    // the implementation. This direction flips again inside callback inputs.
    for (index, (e, a)) in expected.args.iter().zip(&actual.args).enumerate() {
        if e.compatibility(a) == outcome {
            let index = index + 1;
            let step = if is_callback {
                Step::CallbackArgument { index }
            } else {
                Step::Argument { index }
            };
            return type_failure(e, a, outcome).at(step);
        }
    }
    if let Some((e, a)) = expected.rest.as_ref().zip(actual.rest.as_ref())
        && e.compatibility(a) == outcome
    {
        return type_failure(e, a, outcome).at(Step::CallbackRest);
    }
    debug_assert_eq!(actual.ret.compatibility(&expected.ret), outcome);
    type_failure(&actual.ret, &expected.ret, outcome).at(if is_callback {
        Step::CallbackResult
    } else {
        Step::Result
    })
}

fn type_failure(actual: &Type, expected: &Type, outcome: Compatibility) -> ExternMismatch {
    let deeper = match (actual, expected) {
        (Type::Union(parts), expected) => parts
            .iter()
            .find(|part| part.compatibility(expected) == outcome)
            .map(|part| type_failure(part, expected, outcome).at(Step::UnionMember)),
        // A target union is a choice. One incompatible target branch doesn't
        // explain rejecting the value, so report the entire set of alternatives.
        (_, Type::Union(_)) => None,
        (Type::Map(a), Type::Map(e)) => map_failure(a, e, outcome),
        (Type::Vector(a), Type::Vector(e))
        | (Type::Set(a), Type::Set(e))
        | (Type::Seq(a), Type::Seq(e)) => Some(type_failure(a, e, outcome).at(Step::Element)),
        (Type::Tuple(a), Type::Tuple(e)) | (Type::SeqTuple(a), Type::SeqTuple(e)) => {
            if a.len() != e.len() {
                Some(ExternMismatch::new(Reason::TupleLength {
                    expected: e.len(),
                    actual: a.len(),
                }))
            } else {
                a.iter().zip(e).enumerate().find_map(|(index, (a, e))| {
                    (a.compatibility(e) == outcome)
                        .then(|| type_failure(a, e, outcome).at(Step::Item { index }))
                })
            }
        }
        (Type::Tuple(items), Type::Vector(e)) | (Type::SeqTuple(items), Type::Seq(e)) => {
            items.iter().enumerate().find_map(|(index, a)| {
                (a.compatibility(e) == outcome)
                    .then(|| type_failure(a, e, outcome).at(Step::Item { index }))
            })
        }
        (Type::Function(a), Type::Function(e)) => match (a.as_ref(), e.as_ref()) {
            ([a], [e]) if signature(a, e) == outcome => {
                Some(signature_failure(a, e, outcome, true))
            }
            _ => None,
        },
        _ => None,
    };
    deeper.unwrap_or_else(|| {
        let actual = actual.to_string();
        let expected = expected.to_string();
        ExternMismatch::new(match outcome {
            Compatibility::Mismatch => Reason::TypeMismatch { expected, actual },
            _ => Reason::Unproven { expected, actual },
        })
    })
}

fn map_failure(
    actual: &MapType,
    expected: &MapType,
    outcome: Compatibility,
) -> Option<ExternMismatch> {
    if expected.extra.accepts_type(&actual.extra) == outcome {
        if let (ExtraFields::Typed(a), ExtraFields::Typed(e)) = (&actual.extra, &expected.extra) {
            let [actual_key, actual_value] = a.as_ref();
            let [expected_key, expected_value] = e.as_ref();
            if actual_key.compatibility(expected_key) == outcome {
                return Some(type_failure(actual_key, expected_key, outcome).at(Step::MapKey));
            }
            return Some(type_failure(actual_value, expected_value, outcome).at(Step::MapValue));
        }
        return Some(ExternMismatch::new(Reason::UnexpectedExtraEntries));
    }
    for (key, expected_field) in &expected.fields {
        let slot = actual.slot(key);
        if slot.compatibility(expected_field) != outcome {
            continue;
        }
        let failure = match slot {
            MapSlot::Present(ty) => type_failure(ty, &expected_field.ty, outcome),
            MapSlot::Optional(_) if expected_field.required => {
                ExternMismatch::new(Reason::OptionalField)
            }
            MapSlot::Optional(ty) => type_failure(ty, &expected_field.ty, outcome),
            MapSlot::Absent => ExternMismatch::new(Reason::MissingField),
            MapSlot::Undeclared => ExternMismatch::new(Reason::UndeclaredField),
        };
        return Some(failure.at(Step::Field {
            key: key.to_string(),
        }));
    }
    for (key, field) in actual.unlisted_fields(expected) {
        if expected.extra.accepts_field(key, &field.ty) != outcome {
            continue;
        }
        let failure = match &expected.extra {
            ExtraFields::Typed(types) => {
                let [expected_key, expected_value] = types.as_ref();
                if key.compatibility(expected_key) == outcome {
                    type_failure(key, expected_key, outcome).at(Step::MapKey)
                } else {
                    type_failure(&field.ty, expected_value, outcome)
                }
            }
            _ => ExternMismatch::new(Reason::UnexpectedField),
        };
        return Some(failure.at(Step::Field {
            key: key.to_string(),
        }));
    }
    None
}
