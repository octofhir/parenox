//! Lexical locals and presence facts scoped to the current bindings.
use std::collections::BTreeMap;

use super::Value;
use crate::reader::Form;

#[derive(Clone, Default)]
pub(super) struct Env {
    bindings: BTreeMap<String, Value>,
    presence: Vec<Presence>,
}

#[derive(Clone)]
struct Presence {
    collection: String,
    key: String,
    present: bool,
}

impl Env {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn get(&self, name: &str) -> Option<&Value> {
        self.bindings.get(name)
    }

    pub(super) fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.bindings.get_mut(name)
    }

    pub(super) fn contains_key(&self, name: &str) -> bool {
        self.bindings.contains_key(name)
    }

    pub(super) fn insert(&mut self, name: String, value: Value) {
        self.presence
            .retain(|fact| fact.collection != name && fact.key != name);
        self.bindings.insert(name, value);
    }

    pub(super) fn presence(&self, collection: &Form, key: &Form) -> Option<bool> {
        if self.presence.is_empty() {
            return None;
        }
        let (collection, key) = (collection.symbol()?, key.symbol()?);
        self.presence.iter().rev().find_map(|fact| {
            (fact.collection == collection && fact.key == key).then_some(fact.present)
        })
    }

    pub(super) fn set_presence(&mut self, collection: &str, key: &str, present: bool) {
        self.presence.push(Presence {
            collection: collection.into(),
            key: key.into(),
            present,
        });
    }
}
