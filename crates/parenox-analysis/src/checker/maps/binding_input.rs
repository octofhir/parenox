//! The sequence-to-map conversion performed by Clojure's map destructuring.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{ExtraFields, LiteralContainer, MapType, Type, Value, associate_literal_child};
use super::{equivalent_integer_map_key, unresolved_integer_key_class};

const MAX_SEQUENCE_ALTERNATIVES: usize = 64;

impl Value {
    pub(in crate::checker) fn may_convert_for_map_binding(&self) -> bool {
        may_convert_sequence(&self.known)
    }

    pub(in crate::checker) fn destructuring_sequence(self) -> Option<Self> {
        if let Type::Union(parts) = self.known {
            return convert_alternatives(parts, self.literal_container.as_ref()).map(|value| {
                Self {
                    notes: self.notes,
                    ..value
                }
            });
        }
        let (Type::SeqTuple(items) | Type::ListTuple(items)) = self.known else {
            return None;
        };
        let positions = self.literal_container.as_ref();
        // A single item becomes the saved value without another conversion.
        if let [only] = items.as_ref() {
            let mut value = Self::with_notes(only.clone(), self.notes);
            value.literal_container = positions.and_then(|proof| proof.position_child(0)).cloned();
            return Some(value);
        }

        let (pairs, tail) = items.as_chunks::<2>();
        let mut map = MapType {
            fields: BTreeMap::new(),
            extra: ExtraFields::Closed,
        };
        let mut children = None;
        for (index, [key, value]) in pairs.iter().enumerate() {
            let child = positions.and_then(|proof| proof.position_child(index * 2 + 1));
            append_entry(&mut map, &mut children, key, value, child)?;
        }
        // createAsIfByAssoc appends the trailing collection before resolving
        // duplicate keys. The first key object and the last value survive.
        if let [tail] = tail {
            let proof = positions.and_then(|proof| proof.position_child(items.len() - 1));
            append_tail(&mut map, &mut children, tail, proof)?;
        }
        let mut value = Self::with_notes(Type::Map(map), self.notes);
        value.literal_container = Some(LiteralContainer::Map(children));
        Some(value)
    }
}

type ChildProofs = Option<Arc<BTreeMap<Type, LiteralContainer>>>;

fn append_entry(
    map: &mut MapType,
    children: &mut ChildProofs,
    key: &Type,
    value: &Type,
    proof: Option<&LiteralContainer>,
) -> Option<()> {
    let address =
        equivalent_integer_map_key(key).filter(|address| map.fields.contains_key(address));
    let key = address.as_ref().unwrap_or(key);
    if !key.is_literal_map_key() || unresolved_integer_key_class(map, key) {
        return None;
    }
    map.associate(key, value);
    associate_literal_child(children, key, value, proof);
    Some(())
}

fn append_tail(
    map: &mut MapType,
    children: &mut ChildProofs,
    tail: &Type,
    proof: Option<&LiteralContainer>,
) -> Option<()> {
    match (tail, proof) {
        (Type::Nil, _) => Some(()),
        (Type::Tuple(pair), Some(proof @ LiteralContainer::Vector(_)))
            if let [key, value] = pair.as_ref() =>
        {
            append_entry(map, children, key, value, proof.position_child(1))
        }
        (Type::MapEntry(pair), proof) => {
            let [key, value] = pair.as_ref();
            append_entry(
                map,
                children,
                key,
                value,
                proof.and_then(|proof| proof.position_child(1)),
            )
        }
        (Type::SeqTuple(items) | Type::ListTuple(items), proof) => {
            for (index, item) in items.iter().enumerate() {
                if !matches!(item, Type::MapEntry(_)) {
                    return None;
                }
                let entry_proof = proof.and_then(|proof| proof.position_child(index));
                append_tail(map, children, item, entry_proof)?;
            }
            Some(())
        }
        (Type::Map(tail), Some(proof @ LiteralContainer::Map(_)))
            if tail.extra == ExtraFields::Closed
                && tail.fields.values().all(|field| field.required) =>
        {
            for (key, field) in &tail.fields {
                append_entry(map, children, key, &field.ty, proof.map_field(key))?;
            }
            Some(())
        }
        _ => None,
    }
}

fn convert_alternatives(parts: Vec<Type>, proof: Option<&LiteralContainer>) -> Option<Value> {
    if parts.len() > MAX_SEQUENCE_ALTERNATIVES {
        return None;
    }
    let values = parts
        .into_iter()
        .map(|ty| {
            let literal_container = proof.filter(|proof| proof.matches_type(&ty)).cloned();
            let branch = Value {
                literal_container,
                ..Value::new(ty)
            };
            if branch.may_convert_for_map_binding() {
                branch.destructuring_sequence()
            } else {
                Some(branch)
            }
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Value::union_branches(values))
}

fn may_convert_sequence(ty: &Type) -> bool {
    match ty {
        Type::Seq(_)
        | Type::List(_)
        | Type::SeqTuple(_)
        | Type::ListTuple(_)
        | Type::SpecCollection(_) => true,
        Type::Parameter(parameter) => parameter.upper.as_ref().is_none_or(may_convert_sequence),
        Type::Union(types) => types.iter().any(may_convert_sequence),
        _ => false,
    }
}
