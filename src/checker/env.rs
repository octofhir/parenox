//! Lexical locals and presence facts scoped to the current bindings.
use std::collections::BTreeMap;

use super::Value;
use crate::reader::Form;
use crate::types::Type;

#[derive(Clone, Default)]
pub(super) struct Env {
    bindings: BTreeMap<String, Value>,
    presence: Vec<Presence>,
    selectors: BTreeMap<String, Selector>,
    impossible: bool,
}

#[derive(Clone)]
pub(super) struct Selector {
    pub root: String,
    pub keys: Vec<Type>,
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
        self.selectors
            .retain(|alias, selector| alias != &name && selector.root != name);
        self.bindings.insert(name, value);
    }

    pub(super) fn selector(&self, name: &str) -> Option<&Selector> {
        self.selectors.get(name)
    }

    pub(super) fn set_selector(&mut self, name: &str, selector: Selector) {
        // The initializer was read before binding name. If it reuses the
        // owner's name, that old owner can no longer be narrowed here.
        if name != selector.root && self.contains_key(name) {
            self.selectors.insert(name.into(), selector);
        }
    }

    pub(super) fn refresh_selectors(
        &mut self,
        root: &str,
        project: impl Fn(&[Type]) -> Option<Type>,
    ) {
        for (name, selector) in &self.selectors {
            if selector.root == root
                && let Some(ty) = project(&selector.keys)
                && let Some(value) = self.bindings.get_mut(name)
                && ty != value.known
                && ty.subtype_of(&value.known)
            {
                value.narrow(ty);
            }
        }
    }

    pub(super) fn reachable(&self) -> bool {
        !self.impossible
    }

    pub(super) fn mark_impossible(&mut self) {
        self.impossible = true;
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
