//! Field presence and value types after a finite map projection.
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{Checker, LiteralContainer, Value, equivalent_integer_map_key};
use crate::reader::Span;
use crate::types::{ExtraFields, Field, MapSlot, MapType, Type};

const FINITE_SELECTION: &str = "select-keys requires a finite set of supported literal keys";

impl Checker<'_> {
    pub(in crate::checker) fn select_keys(
        &mut self,
        (target, target_span): &(Value, Span),
        (keys, keys_span): &(Value, Span),
    ) -> Value {
        let sequence = self.seq_value_type(keys, "clojure.core/select-keys", *keys_span);
        if matches!(sequence, Type::Never | Type::Unknown) {
            return Value::new(sequence);
        }
        match selected_keys(&sequence) {
            Ok(selection) => self.select_proven_keys(target, *target_span, *keys_span, &selection),
            Err(message) => self.unsupported(*keys_span, message),
        }
    }

    fn select_proven_keys(
        &mut self,
        target: &Value,
        target_span: Span,
        keys_span: Span,
        selection: &BTreeMap<&Type, bool>,
    ) -> Value {
        if selection.is_empty() {
            return Value::new(closed_map(BTreeMap::new()));
        }

        let mut undeclared = BTreeSet::new();
        let projected = self.project_map(
            &target.known,
            selection,
            target_span,
            &mut undeclared,
            false,
        );
        for key in undeclared {
            self.undeclared_key(key, keys_span);
        }
        target.retaining_map_projection(projected)
    }

    pub(in crate::checker) fn select_destructured_keys(
        &mut self,
        target: &Value,
        keys: &[Type],
        span: Span,
    ) -> Value {
        if !is_closed_map_or_nil(&target.known) {
            return self.unsupported(span, ":select requires a known closed map or Nil");
        }
        let selection = keys.iter().map(|key| (key, true)).collect();
        let projected =
            self.project_map(&target.known, &selection, span, &mut BTreeSet::new(), true);
        target.retaining_map_projection(projected)
    }

    pub(in crate::checker) fn merge_destructuring_defaults<'a>(
        &mut self,
        target: &'a Value,
        values: BTreeMap<Type, &Value>,
        span: Option<Span>,
    ) -> Option<Cow<'a, Value>> {
        if !is_closed_map_or_nil(&target.known) {
            return span.map(|span| {
                Cow::Owned(
                    self.unsupported(span, ":select and :all require a known closed map or Nil"),
                )
            });
        }
        if values.is_empty() {
            return Some(Cow::Borrowed(target));
        }
        let defaults = MapType {
            fields: values
                .iter()
                .map(|(key, value)| (key.clone(), Field::new(value.known.clone(), true)))
                .collect(),
            extra: ExtraFields::Closed,
        };
        let mut result = Value::with_notes(
            merge_defaults(&target.known, &defaults),
            target.notes.clone(),
        );
        if let Type::Map(merged) = &result.known {
            result.literal_container = Some(LiteralContainer::Map(default_children(
                target, merged, &values,
            )));
        }
        Some(Cow::Owned(result))
    }

    fn project_map<'a>(
        &mut self,
        target: &Type,
        selection: &BTreeMap<&'a Type, bool>,
        span: Span,
        undeclared: &mut BTreeSet<&'a Type>,
        preserve_nil: bool,
    ) -> Type {
        match target {
            Type::Nil if preserve_nil => Type::Nil,
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
                    .map(|part| self.project_map(part, selection, span, undeclared, preserve_nil)),
            ),
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.project_map(upper, selection, span, undeclared, preserve_nil)
            }
            Type::Unknown => Type::Unknown,
            _ => {
                self.unsupported(span, "select-keys currently analyzes map or Nil targets")
                    .known
            }
        }
    }
}

impl Value {
    pub(in crate::checker) fn excess_destructured_keys(
        &self,
        keys: &[Type],
        nested: Vec<(Type, Value)>,
    ) -> Option<Value> {
        if !is_closed_map_or_nil(&self.known) {
            return None;
        }
        let base = self.retaining_map_projection(excess_type(&self.known, keys));
        base.merge_destructured_children(nested)
    }

    pub(in crate::checker) fn missing_destructured_keys(
        &self,
        keys: &[Type],
        nested: Vec<(Type, Value)>,
    ) -> Option<Value> {
        let direct = if keys.is_empty() {
            Type::Nil
        } else if is_closed_map_or_nil(&self.known) {
            missing_type(&self.known, keys.iter())
        } else {
            return None;
        };
        let mut base = Value::with_notes(direct, self.notes.clone());
        if matches!(base.known, Type::Map(_)) {
            base.literal_container = Some(LiteralContainer::Map(None));
        }
        base.merge_destructured_children(nested)
    }

    pub(in crate::checker) fn merge_destructured_children(
        self,
        nested: Vec<(Type, Value)>,
    ) -> Option<Value> {
        let base = self;
        if nested.is_empty() {
            return Some(base);
        }
        let ordinary = base.known == Type::Nil || base.is_literal_map();
        let mut fields = BTreeMap::new();
        let mut children = match &base.literal_container {
            Some(LiteralContainer::Map(Some(children))) => (**children).clone(),
            _ => BTreeMap::new(),
        };
        for (key, value) in nested {
            if value.known.contains_unknown() {
                return None;
            }
            let non_nil = value.known.without_nil();
            if non_nil == Type::Never {
                continue;
            }
            let required = non_nil == value.known;
            // An optional child may leave the original field untouched.
            let inherited =
                matches!(&base.known, Type::Map(map) if !matches!(map.slot(&key), MapSlot::Absent));
            children.remove(&key);
            if ordinary
                && (required || !inherited)
                && let Some(proof) = value.literal_container
                && proof.matches_type(&non_nil)
            {
                children.insert(key.clone(), proof);
            }
            fields.insert(key, Field::new(non_nil, required));
        }
        if fields.is_empty() {
            return Some(base);
        }
        let ty = merge_nested_maps(
            base.known,
            &MapType {
                fields,
                extra: ExtraFields::Closed,
            },
        );
        let mut result = Value::with_notes(ty, base.notes);
        if ordinary && let Type::Map(map) = &result.known {
            children.retain(|key, proof| {
                map.fields
                    .get(key)
                    .is_some_and(|field| proof.matches_type(&field.ty))
            });
            result.literal_container = Some(LiteralContainer::Map(
                (!children.is_empty()).then(|| Arc::new(children)),
            ));
        }
        Some(result)
    }
}

fn excess_type(target: &Type, keys: &[Type]) -> Type {
    match target {
        Type::Nil => Type::Nil,
        Type::Map(map) => {
            let mut excess = map.clone();
            for key in keys {
                excess.dissociate(key);
            }
            if excess.fields.values().any(|field| field.required) {
                Type::Map(excess)
            } else if excess.fields.values().any(|field| field.ty != Type::Never) {
                Type::Map(excess).nilable()
            } else {
                Type::Nil
            }
        }
        Type::Union(parts) => Type::union(parts.iter().map(|part| excess_type(part, keys))),
        _ => Type::Unknown,
    }
}

fn merge_defaults(target: &Type, defaults: &MapType) -> Type {
    match target {
        Type::Nil => Type::Map(defaults.clone()),
        Type::Map(map) => Type::Map(defaults.merge(map)),
        Type::Union(parts) => Type::union(parts.iter().map(|part| merge_defaults(part, defaults))),
        _ => Type::Unknown,
    }
}

fn missing_type<'a>(target: &Type, keys: impl Iterator<Item = &'a Type> + Clone) -> Type {
    let map = match target {
        Type::Nil => None,
        Type::Map(map) => Some(map),
        Type::Union(parts) => {
            return Type::union(parts.iter().map(|part| missing_type(part, keys.clone())));
        }
        _ => return Type::Unknown,
    };
    let fields: BTreeMap<_, _> = keys
        .filter_map(|key| {
            let required = match map.map_or(MapSlot::Absent, |map| map.slot(key)) {
                MapSlot::Absent => true,
                MapSlot::Optional(_) => false,
                MapSlot::Present(_) => return None,
                MapSlot::Undeclared => unreachable!("missing_type requires a closed map"),
            };
            Some((key.clone(), Field::new(Type::Nil, required)))
        })
        .collect();
    if fields.is_empty() {
        Type::Nil
    } else if fields.values().any(|field| field.required) {
        closed_map(fields)
    } else {
        closed_map(fields).nilable()
    }
}

fn merge_nested_maps(direct: Type, nested: &MapType) -> Type {
    match direct {
        Type::Nil if nested.fields.values().any(|field| field.required) => {
            Type::Map(nested.clone())
        }
        Type::Nil => Type::Map(nested.clone()).nilable(),
        Type::Map(direct) => Type::Map(direct.merge(nested)),
        Type::Union(parts) => Type::union(
            parts
                .into_iter()
                .map(|part| merge_nested_maps(part, nested)),
        ),
        _ => Type::Unknown,
    }
}

fn is_closed_map_or_nil(ty: &Type) -> bool {
    match ty {
        Type::Nil => true,
        Type::Map(map) => map.extra == ExtraFields::Closed,
        Type::Union(parts) => parts.iter().all(is_closed_map_or_nil),
        _ => false,
    }
}

fn closed_map(fields: BTreeMap<Type, Field>) -> Type {
    Type::Map(MapType {
        fields,
        extra: ExtraFields::Closed,
    })
}

fn default_children(
    target: &Value,
    merged: &MapType,
    defaults: &BTreeMap<Type, &Value>,
) -> Option<Arc<BTreeMap<Type, LiteralContainer>>> {
    let original = match &target.known {
        Type::Nil => None,
        Type::Map(map) => Some(map),
        _ => return None,
    };
    let stored_children = match &target.literal_container {
        Some(LiteralContainer::Map(children)) => children.as_deref(),
        _ => None,
    };
    if stored_children.is_none()
        && defaults
            .values()
            .all(|value| value.literal_container.is_none())
    {
        return None;
    }
    let children: BTreeMap<_, _> = merged
        .fields
        .iter()
        .filter_map(|(key, field)| {
            let fallback = defaults
                .get(key)
                .and_then(|value| value.literal_container.as_ref());
            let stored = stored_children.and_then(|children| children.get(key));
            let child = match original.map_or(MapSlot::Absent, |map| map.slot(key)) {
                MapSlot::Present(_) => stored,
                MapSlot::Absent => fallback,
                MapSlot::Optional(_) if !defaults.contains_key(key) || stored == fallback => stored,
                _ => None,
            }?;
            child
                .matches_type(&field.ty)
                .then(|| (key.clone(), child.clone()))
        })
        .collect();
    (!children.is_empty()).then(|| Arc::new(children))
}

fn selection_key(key: &Type) -> Result<&Type, &'static str> {
    if key.is_literal_map_key() {
        Ok(key)
    } else if equivalent_integer_map_key(key).is_some() {
        // select-keys stores the selector object, rather than the key found in the input.
        Err("select-keys cannot model the selector's stored integer class")
    } else {
        Err(FINITE_SELECTION)
    }
}

fn selected_keys(sequence: &Type) -> Result<BTreeMap<&Type, bool>, &'static str> {
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
            _ => return Err(FINITE_SELECTION),
        };
        let mut present = BTreeSet::new();
        for item in items {
            match item {
                Type::Union(choices) => {
                    let mut choices = choices.iter().map(selection_key);
                    let first = choices.next().ok_or(FINITE_SELECTION)??;
                    let mut same_key = true;
                    for choice in choices {
                        let choice = choice?;
                        if choice != first {
                            counts.entry(choice).or_insert(0);
                            same_key = false;
                        }
                    }
                    if same_key {
                        present.insert(first);
                    } else {
                        counts.entry(first).or_insert(0);
                    }
                }
                key => {
                    present.insert(selection_key(key)?);
                }
            }
        }
        for key in present {
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    Ok(counts
        .into_iter()
        .map(|(key, count)| (key, count == alternatives.len()))
        .collect())
}
