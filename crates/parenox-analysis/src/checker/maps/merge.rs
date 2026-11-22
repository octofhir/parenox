//! Right-biased and callback-based merging of map entries.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{Checker, Env, LiteralContainer, MAX_MAP_CALLBACK_CHECKS, Value};
use crate::checker::calls::{PreparedCallable, call_returns};
use crate::reader::{Form, Span};
use crate::types::{ExtraFields, MapSlot, MapType, Type};

const MAX_MERGE_ALTERNATIVES: usize = 64;

struct MergeCall<'a, 'form, 'model> {
    callable: &'a PreparedCallable<'form, 'model>,
    span: Span,
    env: &'a Env,
}

impl Checker<'_> {
    pub(in crate::checker) fn merge(&mut self, args: &[(Value, Span)], span: Span) -> Value {
        let mut result = Type::Nil;
        for (value, arg_span) in args {
            let right = alternatives(&value.known);
            if right.contains(&Type::Unknown) {
                return self.unsupported(*arg_span, "Cannot determine the map type for merge");
            }
            if !right
                .iter()
                .all(|ty| matches!(ty, Type::Nil | Type::Map(_)))
            {
                return self.unsupported(
                    *arg_span,
                    format!(
                        "merge currently supports only map or Nil operands, got {}",
                        value.ty
                    ),
                );
            }
            let left = alternatives(&result);
            if left.len().saturating_mul(right.len()) > MAX_MERGE_ALTERNATIVES {
                return self.unsupported(span, "Too many map alternatives to analyze merge");
            }
            result = Type::union(left.iter().flat_map(|a| {
                right.iter().map(move |b| match (a, b) {
                    (Type::Map(a), Type::Map(b)) => Type::Map(a.merge(b)),
                    (a, Type::Nil) => a.clone(),
                    (Type::Nil, b) => b.clone(),
                    _ => unreachable!("merge operands were checked"),
                })
            }));
        }
        for ty in alternatives(&result) {
            if let Type::Map(map) = ty
                && let Some((key, _)) = map
                    .fields
                    .iter()
                    .find(|(_, field)| field.ty == Type::Unknown)
            {
                self.error(
                    "PNX9003",
                    span,
                    format!("Cannot determine the type of {key} after merge"),
                );
                break;
            }
        }
        let mut result = Value::new(result);
        if let Type::Map(map) = &result.known {
            result.literal_container = literal_merge_proof(map, args);
        }
        result
    }

    pub(in crate::checker) fn merge_with(
        &mut self,
        forms: &[Form],
        span: Span,
        env: &Env,
    ) -> Value {
        let [callback, operands @ ..] = forms else {
            self.error(
                "PNX2554",
                span,
                "merge-with expects a callback followed by maps",
            );
            return Value::new(Type::Unknown);
        };
        let callable = self.prepare_callable(callback, env);
        let values: Vec<_> = self.eval_args(operands, env, callable.returns()).collect();
        if !callable.returns() || !call_returns(&values) {
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }
        let start = self.diagnostics.len();
        let steps = self.map_callback_checks;
        let call = MergeCall {
            callable: &callable,
            span,
            env,
        };
        let result = self.merge_with_operands(&values, operands, &call);
        if self.map_callback_checks == steps {
            self.call_prepared(&callable, Vec::new(), span, env, false);
        }
        self.dedup_diagnostics(start);
        Value::new(result)
    }

    fn merge_with_operands(
        &mut self,
        values: &[(Value, Span)],
        operands: &[Form],
        call: &MergeCall<'_, '_, '_>,
    ) -> Type {
        let mut result = Type::Nil;
        for ((value, span), source) in values.iter().zip(operands) {
            let right = alternatives(&value.known);
            if !right
                .iter()
                .all(|ty| matches!(ty, Type::Nil | Type::Map(_)))
            {
                return self
                    .unsupported(
                        *span,
                        format!(
                            "merge-with currently supports only map or Nil operands, got {}",
                            value.ty,
                        ),
                    )
                    .known;
            }
            let left = alternatives(&result);
            if left.len().saturating_mul(right.len()) > MAX_MERGE_ALTERNATIVES {
                return self
                    .unsupported(call.span, "Too many map alternatives to analyze merge-with")
                    .known;
            }
            let mut merged = Vec::new();
            for a in left {
                for b in right {
                    let result = match (a, b) {
                        (Type::Map(a), Type::Map(b)) => self.merge_pair(a, b, source, call),
                        (a, Type::Nil) => a.clone(),
                        (Type::Nil, b) => b.clone(),
                        _ => unreachable!("merge-with operands were checked"),
                    };
                    if result == Type::Unknown {
                        return result;
                    }
                    merged.push(result);
                }
            }
            result = Type::union(merged);
            if result == Type::Never {
                break;
            }
        }
        result
    }

    fn merge_pair(
        &mut self,
        left: &MapType,
        right: &MapType,
        source: &Form,
        call: &MergeCall<'_, '_, '_>,
    ) -> Type {
        if known_empty(left) {
            return Type::Map(right.clone());
        }
        if known_empty(right) {
            return Type::Map(left.clone());
        }
        if unresolved_overlap(left, right) || unresolved_overlap(right, left) {
            return self.unsupported(
                source.span,
                "merge-with cannot determine both value types for a possible overlap with an open map",
            ).known;
        }

        let literal_values: std::collections::BTreeMap<_, _> = source
            .map()
            .unwrap_or_default()
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[key, value]| Some((self.literal_map_key(key)?, value.span)))
            .collect();
        let mut result = left.merge(right);
        let mut returns = true;
        for (key, field) in &mut result.fields {
            let (Some((a, a_required)), Some((b, b_required))) =
                (known_slot(left.slot(key)), known_slot(right.slot(key)))
            else {
                continue;
            };
            let value_span = literal_values.get(key).copied().unwrap_or(source.span);
            let Some(combined) = self.combine_values(a, b, Some(key), value_span, call) else {
                return Type::Unknown;
            };
            field.replace_type(Type::union(
                std::iter::once(combined)
                    .chain((!b_required).then(|| a.clone()))
                    .chain((!a_required).then(|| b.clone())),
            ));
            returns &= !field.required || field.ty != Type::Never;
        }
        if let (ExtraFields::Typed(a), ExtraFields::Typed(b)) = (&left.extra, &right.extra)
            && matches!(result.extra, ExtraFields::Typed(_))
            && unlisted_overlap(&a[0], &b[0], &result)
        {
            let Some(combined) = self.combine_values(&a[1], &b[1], None, source.span, call) else {
                return Type::Unknown;
            };
            if let ExtraFields::Typed(types) = &mut result.extra {
                types[1] = Type::union([a[1].clone(), b[1].clone(), combined]);
            }
        }
        if returns {
            Type::Map(result)
        } else {
            Type::Never
        }
    }

    fn combine_values(
        &mut self,
        left: &Type,
        right: &Type,
        key: Option<&Type>,
        right_span: Span,
        call: &MergeCall<'_, '_, '_>,
    ) -> Option<Type> {
        if !self.take_map_callback_step() {
            self.unsupported(
                call.span,
                format!("merge-with callback analysis limit ({MAX_MAP_CALLBACK_CHECKS}) exceeded",),
            );
            return None;
        }
        let args = vec![
            (Value::new(left.clone()), call.span),
            (Value::new(right.clone()), right_span),
        ];
        let start = self.diagnostics.len();
        let value = self.call_prepared(call.callable, args, call.span, call.env, true);
        for diagnostic in &mut self.diagnostics[start..] {
            diagnostic.notes.push(key.map_or_else(
                || "While combining overlapping dictionary entries".into(),
                |key| format!("While combining values at {key}"),
            ));
        }
        Some(value.known)
    }
}

fn known_empty(map: &MapType) -> bool {
    map.extra == ExtraFields::Closed && map.fields.values().all(|field| field.ty == Type::Never)
}

fn unresolved_overlap(open: &MapType, other: &MapType) -> bool {
    open.extra == ExtraFields::Open
        && (other.extra != ExtraFields::Closed
            || other.fields.iter().any(|(key, field)| {
                field.ty != Type::Never && matches!(open.slot(key), MapSlot::Undeclared)
            }))
}

fn known_slot(slot: MapSlot<'_>) -> Option<(&Type, bool)> {
    match slot {
        MapSlot::Present(value) => Some((value, true)),
        MapSlot::Optional(value) => Some((value, false)),
        MapSlot::Absent | MapSlot::Undeclared => None,
    }
}

fn unlisted_overlap(left: &Type, right: &Type, result: &MapType) -> bool {
    match (left, right) {
        (Type::Union(parts), _) => parts
            .iter()
            .any(|part| unlisted_overlap(part, right, result)),
        (_, Type::Union(parts)) => parts
            .iter()
            .any(|part| unlisted_overlap(left, part, result)),
        _ => {
            [left, right]
                .iter()
                .all(|key| !key.is_literal_map_key() || !result.fields.contains_key(*key))
                && left.key_overlaps(right)
        }
    }
}

fn alternatives(ty: &Type) -> &[Type] {
    match ty {
        Type::Union(parts) => parts,
        ty => std::slice::from_ref(ty),
    }
}

fn literal_merge_proof(map: &MapType, args: &[(Value, Span)]) -> Option<LiteralContainer> {
    let ordinary = args.iter().all(|(value, _)| match &value.known {
        Type::Nil => true,
        Type::Map(map) => map.extra == ExtraFields::Closed && value.is_literal_map(),
        _ => false,
    });
    if !ordinary {
        return None;
    }
    let has_children = args.iter().any(|(value, _)| {
        matches!(
            value.literal_container,
            Some(LiteralContainer::Map(Some(_)))
        )
    });
    if !has_children {
        return Some(LiteralContainer::Map(None));
    }
    let children: BTreeMap<_, _> = map
        .fields
        .iter()
        .filter_map(|(key, field)| {
            let proof = merged_child(args, key)?;
            proof
                .matches_type(&field.ty)
                .then(|| (key.clone(), proof.clone()))
        })
        .collect();
    Some(LiteralContainer::Map(
        (!children.is_empty()).then(|| Arc::new(children)),
    ))
}

fn merged_child<'a>(args: &'a [(Value, Span)], key: &Type) -> Option<&'a LiteralContainer> {
    for (value, _) in args.iter().rev() {
        if let Type::Map(map) = &value.known {
            match map.slot(key) {
                MapSlot::Present(_) => return value.literal_container.as_ref()?.map_field(key),
                MapSlot::Absent => {}
                // A right operand might overwrite the child with a different object.
                MapSlot::Optional(_) | MapSlot::Undeclared => return None,
            }
        }
    }
    None
}
