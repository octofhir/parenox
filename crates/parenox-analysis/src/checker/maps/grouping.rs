//! Eager grouping of typed collection elements into vectors.
use std::collections::BTreeMap;

use super::super::{
    Checker, Env, LiteralContainer, Value,
    calls::{PreparedCallable, iteration_may_throw},
    vector_type,
};
use super::MAX_MAP_CALLBACK_CHECKS;
use crate::reader::{Form, Span};
use crate::types::{ExtraFields, Field, MapType, Type};

struct GroupBy<'a, 'form, 'model> {
    callable: &'a PreparedCallable<'form, 'model>,
    callback: Span,
    source: &'a Form,
    span: Span,
    env: &'a Env,
    notes: &'a [String],
}

impl Checker<'_> {
    pub(in crate::checker) fn group_by(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        let [callback, source] = forms else {
            self.error(
                "PNX2554",
                span,
                "group-by expects a callback and a collection",
            );
            return Value::new(Type::Unknown);
        };
        let callable = self.prepare_callable(callback, env);
        let collection = self.infer_path(source, env, callable.returns(), false);
        if collection.known == Type::Never {
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }
        if collection.unproven_tuple_traversal() {
            self.withhold_callback_facts(&callable, env);
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return self.unchecked_tuple_traversal("clojure.core/group-by", source.span);
        }
        if iteration_may_throw(&collection.known) {
            self.may_throw();
        }
        let items = self.seq_type(&collection.known, "group-by", source.span);
        let call = GroupBy {
            callable: &callable,
            callback: callback.span,
            source,
            span,
            env,
            notes: &collection.notes,
        };
        let start = self.diagnostics.len();
        let result = self.group_items(&items, &call);
        self.dedup_diagnostics(start);
        let mut value = Value::new(result);
        let proof = LiteralContainer::GroupBuckets;
        if proof.matches_type(&value.known) {
            value.literal_container = Some(proof);
        }
        value
    }

    fn group_items(&mut self, items: &Type, call: &GroupBy<'_, '_, '_>) -> Type {
        match items {
            Type::Nil => {
                self.call_prepared(call.callable, Vec::new(), call.span, call.env, false);
                group_fields(BTreeMap::new())
            }
            Type::Never | Type::Unknown => items.clone(),
            Type::Union(shapes) => {
                Type::union(shapes.iter().map(|shape| self.group_items(shape, call)))
            }
            Type::Seq(item) | Type::List(item) => {
                let key = self.group_key(item, call.source.span, call);
                if matches!(key, Type::Never | Type::Unknown) {
                    return key;
                }
                let group = Type::Vector(item.clone());
                if key.is_literal_map_key() {
                    return group_fields(BTreeMap::from([(key, group)]));
                }
                Type::Map(MapType::dictionary(key, group))
            }
            Type::SeqTuple(items) | Type::ListTuple(items) => self.group_fixed(items, call),
            _ => unreachable!("seq supplies sequence views"),
        }
    }

    fn group_fixed(&mut self, items: &[Type], call: &GroupBy<'_, '_, '_>) -> Type {
        let literal = call
            .source
            .vector()
            .filter(|forms| forms.len() == items.len());
        let mut keys = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let span = literal.map_or(call.source.span, |forms| forms[index].span);
            let key = self.group_key(item, span, call);
            if matches!(key, Type::Never | Type::Unknown) {
                return key;
            }
            keys.push(key);
        }
        if !keys.iter().all(Type::is_literal_map_key) {
            let elements = Type::union(items.iter().cloned());
            return Type::Map(MapType::dictionary(
                Type::union(keys),
                Type::Vector(Box::new(elements)),
            ));
        }

        let mut groups: BTreeMap<Type, Vec<Type>> = BTreeMap::new();
        for (key, item) in keys.into_iter().zip(items) {
            groups.entry(key).or_default().push(item.clone());
        }
        group_fields(
            groups
                .into_iter()
                .map(|(key, items)| (key, vector_type(items)))
                .collect(),
        )
    }

    fn group_key(&mut self, item: &Type, span: Span, call: &GroupBy<'_, '_, '_>) -> Type {
        if !self.take_map_callback_step() {
            return self
                .unsupported(
                    call.span,
                    format!(
                        "group-by callback analysis limit ({MAX_MAP_CALLBACK_CHECKS}) exceeded",
                    ),
                )
                .known;
        }
        let item = Value::with_notes(item.clone(), call.notes.to_vec());
        let start = self.diagnostics.len();
        let result =
            self.call_prepared(call.callable, vec![(item, span)], call.span, call.env, true);
        if span == call.source.span {
            for diagnostic in &mut self.diagnostics[start..] {
                if diagnostic.code == "PNX2322" && diagnostic.span == span {
                    diagnostic.suggestions.clear();
                }
            }
        }
        let key = result.known;
        if key.contains_unknown() && self.diagnostics.len() != start {
            return Type::Unknown;
        }
        if !key.is_lookup_key(false) || key.contains_unknown() {
            return self
                .unsupported(
                    call.callback,
                    "group-by requires a supported scalar key type",
                )
                .known;
        }
        key
    }
}

fn group_fields(groups: BTreeMap<Type, Type>) -> Type {
    Type::Map(MapType {
        fields: groups
            .into_iter()
            .map(|(key, ty)| (key, Field::new(ty, true)))
            .collect(),
        extra: ExtraFields::Closed,
    })
}
