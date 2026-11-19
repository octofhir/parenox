//! Field presence and value types after a finite map projection.
use std::collections::{BTreeMap, BTreeSet};

use super::{Checker, Value};
use crate::reader::Span;
use crate::types::{ExtraFields, Field, MapSlot, MapType, Type};

impl Checker<'_> {
    pub(in crate::checker) fn select_keys(
        &mut self,
        (target, target_span): &(Value, Span),
        (keys, keys_span): &(Value, Span),
    ) -> Value {
        let sequence = self.seq_type(&keys.known, "clojure.core/select-keys", *keys_span);
        if matches!(sequence, Type::Never | Type::Unknown) {
            return Value::new(sequence);
        }
        let Some(selection) = selected_keys(&sequence) else {
            return self.unsupported(
                *keys_span,
                "select-keys requires a finite set of possible keyword or string keys",
            );
        };
        if selection.is_empty() {
            return Value::new(closed_map(BTreeMap::new()));
        }

        let mut undeclared = BTreeSet::new();
        let projected = self.project_map(&target.known, &selection, *target_span, &mut undeclared);
        for key in undeclared {
            self.undeclared_key(key, *keys_span);
        }
        Value::with_notes(projected, target.notes.clone())
    }

    fn project_map<'a>(
        &mut self,
        target: &Type,
        selection: &BTreeMap<&'a Type, bool>,
        span: Span,
        undeclared: &mut BTreeSet<&'a Type>,
    ) -> Type {
        match target {
            Type::Nil => closed_map(BTreeMap::new()),
            Type::Map(map) => closed_map(
                selection
                    .iter()
                    .filter_map(|(&key, &selected)| {
                        let (ty, required) = match map.slot(key) {
                            MapSlot::Present(ty) => (ty.clone(), selected),
                            MapSlot::Optional(ty) => (ty.clone(), false),
                            MapSlot::Absent => return None,
                            MapSlot::Undeclared => {
                                undeclared.insert(key);
                                (Type::Unknown, false)
                            }
                        };
                        Some((key.clone(), Field::new(ty, required)))
                    })
                    .collect(),
            ),
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.project_map(part, selection, span, undeclared)),
            ),
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.project_map(upper, selection, span, undeclared)
            }
            Type::Unknown => Type::Unknown,
            _ => {
                self.unsupported(span, "select-keys currently analyzes map or Nil targets")
                    .known
            }
        }
    }
}

fn closed_map(fields: BTreeMap<Type, Field>) -> Type {
    Type::Map(MapType {
        fields,
        extra: ExtraFields::Closed,
    })
}

fn selected_keys(sequence: &Type) -> Option<BTreeMap<&Type, bool>> {
    let alternatives = match sequence {
        Type::Union(parts) => parts.as_slice(),
        _ => std::slice::from_ref(sequence),
    };
    let mut counts = BTreeMap::new();
    for ty in alternatives {
        let items = match ty {
            Type::Nil => &[],
            Type::SeqTuple(items) | Type::ListTuple(items) => items.as_ref(),
            // A nonempty homogeneous sequence guarantees its key only when
            // every element has the same literal type.
            Type::Seq(item) | Type::List(item) => std::slice::from_ref(item.as_ref()),
            _ => return None,
        };
        let mut present = BTreeSet::new();
        for item in items {
            match item {
                Type::Union(choices) if choices.iter().all(Type::is_literal_map_key) => {
                    for choice in choices {
                        counts.entry(choice).or_insert(0);
                    }
                }
                key if key.is_literal_map_key() => {
                    present.insert(key);
                }
                _ => return None,
            }
        }
        for key in present {
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    Some(
        counts
            .into_iter()
            .map(|(key, count)| (key, count == alternatives.len()))
            .collect(),
    )
}
