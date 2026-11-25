//! Eager counting with Clojure map key equality.
use std::collections::BTreeMap;

use super::equivalent_integer_map_key;
use crate::checker::{
    Checker, LiteralContainer, Value,
    calls::{element_type, sequence_lengths},
};
use crate::reader::Span;
use crate::types::{ExtraFields, Field, IntegerClass, IntegerType, MAX_TUPLE_ITEMS, MapType, Type};

impl Checker<'_> {
    pub(in crate::checker) fn frequencies(&mut self, (source, span): &(Value, Span)) -> Value {
        let diagnostic_start = self.diagnostics.len();
        let sequence = self.eager_sequence("clojure.core/frequencies", source, *span);
        if sequence.known == Type::Never {
            return sequence;
        }
        if sequence.known.contains_unknown() {
            if self.diagnostics.len() == diagnostic_start && !source.known.contains_unknown() {
                return self.unchecked_form(
                    *span,
                    "Type analysis for frequencies needs a known reducible source shape",
                );
            }
            return Value::with_notes(Type::Unknown, sequence.notes);
        }
        let lengths = sequence_lengths(&source.known, &sequence.known);
        let fixed = source.exact_set_members().or(match &sequence.known {
            Type::ListTuple(items) | Type::SeqTuple(items) => Some(items.as_ref()),
            Type::Nil => Some(&[]),
            _ => None,
        });
        let map = if let Some(map) = fixed.and_then(fixed_frequencies) {
            map
        } else {
            let element = element_type(sequence.known)
                .unwrap_or_else(|_| unreachable!("a sequence has an element type"));
            if element.is_literal_map_key() {
                let count = count_type(lengths.min.max(1), lengths.max);
                MapType {
                    fields: BTreeMap::from([(element, Field::new(count, lengths.min > 0))]),
                    extra: ExtraFields::Closed,
                }
            } else {
                // Hashing or comparison of an abstract key can force user code.
                self.may_throw();
                let max = source.exact_set_members().map_or(lengths.max, |_| Some(1));
                MapType::dictionary(element, count_type(1, max))
            }
        };
        let mut result = Value::with_notes(Type::Map(map), sequence.notes);
        result.literal_container = Some(LiteralContainer::Map(None));
        result
    }
}

fn fixed_frequencies(items: &[Type]) -> Option<MapType> {
    if items.len() > MAX_TUPLE_ITEMS {
        return None;
    }
    let mut counts = BTreeMap::new();
    for key in items {
        let address = if key.is_literal_map_key() {
            key.clone()
        } else {
            // An equivalent integer updates the count without replacing the stored key.
            let address = equivalent_integer_map_key(key)?;
            if !counts.contains_key(&address) {
                return None;
            }
            address
        };
        *counts.entry(address).or_insert(0) += 1;
    }
    Some(MapType {
        fields: counts
            .into_iter()
            .map(|(key, count)| (key, Field::new(count_type(count, Some(count)), true)))
            .collect(),
        extra: ExtraFields::Closed,
    })
}

fn count_type(min: usize, max: Option<usize>) -> Type {
    if let Some(max) = max
        && let Some(width) = max.checked_sub(min)
        && width < MAX_TUPLE_ITEMS
        && let (Ok(lower), Ok(upper)) = (i64::try_from(min), i64::try_from(max))
    {
        return Type::union(
            (lower..=upper).map(|count| Type::exact_long(count, IntegerClass::IntOrLong)),
        );
    }
    Type::Integral(IntegerType::FIXED)
}
