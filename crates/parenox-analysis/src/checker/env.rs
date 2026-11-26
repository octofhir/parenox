//! Lexical locals and presence facts scoped to the current bindings.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::Value;
use super::jvm_primitives::JvmValueClass;
use super::metadata::{JavaTypeHint, java_type_hint};
use crate::reader::{Form, Kind};
use crate::types::Type;

pub(super) const MAX_SELECTOR_KEYS: usize = 32;

#[derive(Clone, Default)]
pub(super) struct Env {
    bindings: BTreeMap<String, Binding>,
    inline_functions: BTreeMap<String, CapturedInline>,
    next_binding: usize,
    retained_owners: BTreeMap<BindingId, Value>,
    pinned_owners: Vec<BindingId>,
    java_hints: BTreeMap<String, JavaTypeHint<String>>,
    presence: Vec<Presence>,
    selectors: BTreeMap<String, Selector>,
    non_nil_results: BTreeMap<String, Arc<ResultGuard>>,
    impossible: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BindingId(usize);

#[derive(Clone)]
struct Binding {
    id: BindingId,
    value: Value,
    jvm_class: JvmValueClass,
}

#[derive(Clone)]
pub(super) enum CapturedInline {
    Single(Arc<CapturedFn>),
    Group {
        group: Arc<LetFnGroup>,
        index: usize,
    },
}

pub(super) struct CapturedFn {
    pub form: Form,
    pub scope: Env,
}

pub(super) struct LetFnGroup {
    pub scope: Env,
    pub definitions: Box<[(String, Form)]>,
}

#[derive(Clone)]
pub(super) struct SelectorRoot {
    name: Option<String>,
    id: BindingId,
}

#[derive(Clone)]
pub(super) struct Selector {
    pub root: SelectorRoot,
    pub keys: Vec<Type>,
}

pub(super) struct ResultGuard {
    pub subject: Selector,
    pub ty: Type,
}

impl Selector {
    pub(super) fn field(&self, key: Type) -> Option<Self> {
        if self.keys.len() >= MAX_SELECTOR_KEYS {
            return None;
        }
        let mut child = self.clone();
        child.keys.push(key);
        Some(child)
    }
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

    pub(super) fn inline_function(&self, name: &str) -> Option<&CapturedInline> {
        self.inline_functions.get(name)
    }

    pub(super) fn inline_functions(&self) -> impl Iterator<Item = &CapturedInline> {
        self.inline_functions.values()
    }

    pub(super) fn mark_inline(&mut self, name: &str, function: CapturedInline) {
        self.inline_functions.insert(name.into(), function);
    }

    pub(super) fn refine_capture(&self, caller: &Self) -> Self {
        let mut scope = self.clone();
        for (name, binding) in &mut scope.bindings {
            let current = caller
                .bindings
                .get(name)
                .filter(|current| current.id == binding.id)
                .map(|current| &current.value)
                .or_else(|| caller.retained_owners.get(&binding.id));
            if let Some(value) = current {
                binding.value = value.clone();
            }
        }
        for (id, value) in &mut scope.retained_owners {
            if let Some(current) = caller.retained_owners.get(id) {
                *value = current.clone();
            }
        }
        scope
    }

    pub(super) fn insert(&mut self, name: String, value: Value) {
        self.insert_selected(name, value, None);
    }

    pub(super) fn insert_selected(&mut self, name: String, value: Value, source: Option<Selector>) {
        self.inline_functions.remove(&name);
        self.non_nil_results.remove(&name);
        self.java_hints.remove(&name);
        self.presence
            .retain(|fact| fact.collection != name && fact.key != name);
        let old_selection = match source {
            Some(source) => self.selectors.insert(name.clone(), source),
            None => self.selectors.remove(&name),
        };
        let id = self.next_binding_id();
        // An initializer can capture an old owner even when this binding
        // uses its name again. Only keep values with surviving selectors.
        let binding = Binding {
            id,
            value,
            jvm_class: JvmValueClass::Unknown,
        };
        if let Some(old) = self.bindings.insert(name, binding)
            && self.owner_in_use(old.id)
        {
            self.retained_owners.insert(old.id, old.value);
        }
        if let Some(old) = old_selection {
            self.release_unused_owner(old.root.id);
        }
    }

    fn next_binding_id(&mut self) -> BindingId {
        let id = BindingId(self.next_binding);
        self.next_binding += 1;
        id
    }

    fn owner_in_use(&self, owner: BindingId) -> bool {
        self.pinned_owners.contains(&owner)
            || self
                .selectors
                .values()
                .any(|selector| selector.root.id == owner)
    }

    fn release_unused_owner(&mut self, id: BindingId) {
        if self.retained_owners.contains_key(&id) && !self.owner_in_use(id) {
            self.retained_owners.remove(&id);
        }
    }

    pub(super) fn java_hint<'a>(&'a self, form: &'a Form) -> Option<JavaTypeHint<&'a str>> {
        if let Kind::List(items) = &form.without_metadata().kind
            && let Some((head, body)) = items.split_first()
        {
            match (head.symbol(), body) {
                // The JVM compiler derives the class from the final expression.
                (Some("do"), _) => return self.java_hint(body.last()?),
                (Some("if"), [_, yes] | [_, yes, _]) => {
                    let no = body.get(2);
                    let when_true = self.java_hint(yes);
                    let when_false = no.and_then(|no| self.java_hint(no));
                    return match (when_true, when_false) {
                        (a, b) if a == b => a,
                        (a, None) if no.is_none_or(is_nil_literal) => a,
                        (None, b) if is_nil_literal(yes) => b,
                        _ => Some(JavaTypeHint::Unresolved),
                    };
                }
                _ => {}
            }
        }
        java_type_hint(form).or_else(|| {
            form.symbol()
                .and_then(|name| self.java_hints.get(name))
                .map(JavaTypeHint::as_deref)
        })
    }

    pub(super) fn set_java_hint(&mut self, name: &str, hint: JavaTypeHint<String>) {
        self.java_hints.insert(name.into(), hint);
    }

    pub(super) fn jvm_class(&self, name: &str) -> JvmValueClass {
        self.bindings
            .get(name)
            .map_or(JvmValueClass::Unknown, |binding| binding.jvm_class)
    }

    pub(super) fn set_jvm_class(&mut self, name: &str, class: JvmValueClass) {
        self.bindings.get_mut(name).unwrap().jvm_class = class;
    }

    pub(super) fn is_alias(&self, name: &str) -> bool {
        self.selectors.contains_key(name)
    }

    pub(super) fn result_guard(&self, name: &str) -> Option<Arc<ResultGuard>> {
        self.non_nil_results.get(name).cloned()
    }

    pub(super) fn set_result_guard(&mut self, name: &str, guard: Arc<ResultGuard>) {
        self.non_nil_results.insert(name.into(), guard);
    }

    pub(super) fn selector_for(&self, name: &str) -> Option<Selector> {
        self.selectors.get(name).cloned().or_else(|| {
            let binding = self.bindings.get(name)?;
            Some(Selector {
                root: SelectorRoot {
                    name: Some(name.into()),
                    id: binding.id,
                },
                keys: Vec::new(),
            })
        })
    }

    /// A destructuring input without a visible local still has one saved value.
    pub(super) fn captured_selector(&mut self, value: Value) -> Selector {
        let id = self.next_binding_id();
        self.retained_owners.insert(id, value);
        Selector {
            root: SelectorRoot { name: None, id },
            keys: Vec::new(),
        }
    }

    /// Keep the initializer's owner while its pattern introduces new names.
    pub(super) fn with_owner_pinned<T>(
        &mut self,
        source: Option<&Selector>,
        bind: impl FnOnce(&mut Self) -> T,
    ) -> T {
        if let Some(source) = source {
            self.pinned_owners.push(source.root.id);
            let bound = bind(self);
            self.pinned_owners.pop();
            self.release_unused_owner(source.root.id);
            bound
        } else {
            bind(self)
        }
    }

    pub(super) fn owner(&self, root: &SelectorRoot) -> Option<&Value> {
        match root.name.as_ref().and_then(|name| self.bindings.get(name)) {
            Some(binding) if binding.id == root.id => Some(&binding.value),
            _ => self.retained_owners.get(&root.id),
        }
    }

    pub(super) fn owner_mut(&mut self, root: &SelectorRoot) -> Option<&mut Value> {
        match root
            .name
            .as_ref()
            .and_then(|name| self.bindings.get_mut(name))
        {
            Some(binding) if binding.id == root.id => Some(&mut binding.value),
            _ => self.retained_owners.get_mut(&root.id),
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

fn is_nil_literal(form: &Form) -> bool {
    matches!(form.without_metadata().kind, Kind::Nil)
}
