//! Lexical locals and presence facts scoped to the current bindings.
use std::collections::BTreeMap;

use super::Value;
use crate::reader::Form;
use crate::types::Type;

#[derive(Clone, Default)]
pub(super) struct Env {
    bindings: BTreeMap<String, Binding>,
    next_binding: usize,
    shadowed_owners: BTreeMap<BindingId, Value>,
    java_hints: BTreeMap<String, String>,
    presence: Vec<Presence>,
    selectors: BTreeMap<String, Selector>,
    impossible: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BindingId(usize);

#[derive(Clone)]
struct Binding {
    id: BindingId,
    value: Value,
}

#[derive(Clone)]
pub(super) struct SelectorRoot {
    name: String,
    id: BindingId,
}

#[derive(Clone)]
pub(super) struct Selector {
    pub root: SelectorRoot,
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
        self.bindings.get(name).map(|binding| &binding.value)
    }

    pub(super) fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.bindings
            .get_mut(name)
            .map(|binding| &mut binding.value)
    }

    pub(super) fn contains_key(&self, name: &str) -> bool {
        self.bindings.contains_key(name)
    }

    pub(super) fn insert(&mut self, name: String, value: Value) {
        self.insert_selected(name, value, None);
    }

    pub(super) fn insert_selected(&mut self, name: String, value: Value, source: Option<Selector>) {
        self.java_hints.remove(&name);
        self.presence
            .retain(|fact| fact.collection != name && fact.key != name);
        let old_selection = match source {
            Some(source) => self.selectors.insert(name.clone(), source),
            None => self.selectors.remove(&name),
        };
        let id = BindingId(self.next_binding);
        self.next_binding += 1;
        // An initializer can capture an old owner even when this binding
        // uses its name again. Only keep values with surviving selectors.
        if let Some(old) = self.bindings.insert(name, Binding { id, value })
            && self.has_alias_to(old.id)
        {
            self.shadowed_owners.insert(old.id, old.value);
        }
        if let Some(old) = old_selection
            && self.shadowed_owners.contains_key(&old.root.id)
            && !self.has_alias_to(old.root.id)
        {
            self.shadowed_owners.remove(&old.root.id);
        }
    }

    fn has_alias_to(&self, owner: BindingId) -> bool {
        self.selectors
            .values()
            .any(|selector| selector.root.id == owner)
    }

    pub(super) fn java_hint(&self, name: &str) -> Option<&str> {
        self.java_hints.get(name).map(String::as_str)
    }

    pub(super) fn set_java_hint(&mut self, name: &str, hint: &str) {
        self.java_hints.insert(name.into(), hint.into());
    }

    pub(super) fn is_alias(&self, name: &str) -> bool {
        self.selectors.contains_key(name)
    }

    pub(super) fn selector_for(&self, name: &str) -> Option<Selector> {
        self.selectors.get(name).cloned().or_else(|| {
            let binding = self.bindings.get(name)?;
            Some(Selector {
                root: SelectorRoot {
                    name: name.into(),
                    id: binding.id,
                },
                keys: Vec::new(),
            })
        })
    }

    pub(super) fn owner(&self, root: &SelectorRoot) -> Option<&Value> {
        match self.bindings.get(&root.name) {
            Some(binding) if binding.id == root.id => Some(&binding.value),
            _ => self.shadowed_owners.get(&root.id),
        }
    }

    pub(super) fn owner_mut(&mut self, root: &SelectorRoot) -> Option<&mut Value> {
        match self.bindings.get_mut(&root.name) {
            Some(binding) if binding.id == root.id => Some(&mut binding.value),
            _ => self.shadowed_owners.get_mut(&root.id),
        }
    }

    pub(super) fn refresh_selectors(
        &mut self,
        root: &SelectorRoot,
        project: impl Fn(&[Type]) -> Option<Type>,
    ) {
        for (name, selector) in &self.selectors {
            if selector.root.id == root.id
                && let Some(ty) = project(&selector.keys)
                && let Some(binding) = self.bindings.get_mut(name)
                && ty != binding.value.known
                && ty.subtype_of(&binding.value.known)
            {
                binding.value.narrow(ty);
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
