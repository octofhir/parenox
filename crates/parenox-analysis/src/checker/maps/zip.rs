//! Ordered map construction from two sequence views.
use std::collections::BTreeMap;

use super::super::{Checker, Value, calls::iteration_may_throw};
use crate::reader::Span;
use crate::types::{ExtraFields, Field, MapType, Type};

const MAX_ZIP_ALTERNATIVES: usize = 64;

enum ZipInput<'a> {
    Fixed(&'a [Type]),
    Repeated(&'a Type),
}

impl<'a> ZipInput<'a> {
    fn from_seq(ty: &'a Type) -> Option<Self> {
        match ty {
            Type::Nil => Some(Self::Fixed(&[])),
            Type::SeqTuple(items) | Type::ListTuple(items) => Some(Self::Fixed(items)),
            Type::Seq(item) | Type::List(item) => Some(Self::Repeated(item)),
            _ => None,
        }
    }

    fn len(&self) -> Option<usize> {
        match self {
            Self::Fixed(items) => Some(items.len()),
            Self::Repeated(_) => None,
        }
    }

    fn at(&self, index: usize) -> &'a Type {
        match self {
            Self::Fixed(items) => &items[index],
            Self::Repeated(item) => item,
        }
    }
}

impl Checker<'_> {
    pub(in crate::checker) fn zipmap(
        &mut self,
        (keys, keys_span): &(Value, Span),
        (values, values_span): &(Value, Span),
        span: Span,
    ) -> Value {
        if iteration_may_throw(&keys.known) {
            self.may_throw();
        }
        let keys = self.seq_value_type(keys, "zipmap keys", *keys_span);
        if keys == Type::Never {
            return Value::new(Type::Never);
        }
        if iteration_may_throw(&values.known) {
            self.may_throw();
        }
        let values = self.seq_value_type(values, "zipmap values", *values_span);
        if values == Type::Never {
            return Value::new(Type::Never);
        }
        let key_shapes = alternatives(&keys);
        let value_shapes = alternatives(&values);
        if key_shapes.len().saturating_mul(value_shapes.len()) > MAX_ZIP_ALTERNATIVES {
            return self.unsupported(span, "zipmap has too many possible sequence shapes");
        }
        let mut results = Vec::new();
        for keys in key_shapes {
            for values in value_shapes {
                let (Some(keys), Some(values)) =
                    (ZipInput::from_seq(keys), ZipInput::from_seq(values))
                else {
                    return Value::new(Type::Unknown);
                };
                let Some(map) = zipped_map(keys, values) else {
                    return self.unsupported(
                        *keys_span,
                        "zipmap currently supports keyword or string keys",
                    );
                };
                results.push(Type::Map(map));
            }
        }
        Value::new(Type::union(results))
    }
}

fn alternatives(ty: &Type) -> &[Type] {
    match ty {
        Type::Union(parts) => parts,
        ty => std::slice::from_ref(ty),
    }
}

fn zipped_map(keys: ZipInput<'_>, values: ZipInput<'_>) -> Option<MapType> {
    let lengths = (keys.len(), values.len());
    let pairs = match lengths {
        (Some(keys), Some(values)) => keys.min(values),
        (Some(length), None) | (None, Some(length)) => length,
        // Repeated views have uniform item types. One abstract pair is enough
        // for an unbounded number of overwrites with the same types.
        (None, None) => 1,
    };
    let mut map = MapType {
        fields: BTreeMap::new(),
        extra: ExtraFields::Closed,
    };
    for index in 0..pairs {
        let key = keys.at(index);
        let value = values.at(index);
        if !supported_key(key) {
            return None;
        }
        let required = index == 0 || matches!(lengths, (Some(_), Some(_)));
        if required || !key.is_literal_map_key() {
            map.associate(key, value);
        } else {
            // A later pair may be absent, so an earlier value can survive.
            let optional = MapType {
                fields: BTreeMap::from([(key.clone(), Field::new(value.clone(), false))]),
                extra: ExtraFields::Closed,
            };
            map = map.merge(&optional);
        }
    }
    Some(map)
}

fn supported_key(key: &Type) -> bool {
    match key {
        Type::Union(parts) => parts.iter().all(supported_key),
        Type::Key(_) | Type::Str(_) | Type::Keyword | Type::SpecKeyword | Type::String => true,
        _ => false,
    }
}
