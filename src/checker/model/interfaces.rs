//! Immutable declarations used to validate cached checks.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{
    DeclaredValue, Function, Globals, LiteralVar, Model, Names, Signature,
    declarations::symbols_match,
};
use crate::types::Type;

pub(super) type Signatures = BTreeMap<String, Arc<[Signature]>>;

#[derive(Default)]
pub(super) struct Exports {
    pub functions: Signatures,
    pub literals: BTreeMap<String, Type>,
    private_vars: BTreeSet<String>,
}

impl Exports {
    pub(super) fn public_var_names(&self) -> impl Iterator<Item = &str> {
        self.functions
            .keys()
            .chain(self.literals.keys())
            .map(|qualified| qualified.split_once('/').unwrap().1)
    }
}

pub(super) fn visible_value<'a>(
    source: Option<&'a Exports>,
    externs: impl FnOnce() -> Option<&'a Signatures>,
    name: &str,
) -> Option<DeclaredValue<'a>> {
    if let Some(source) = source {
        if let Some(ty) = source.literals.get(name) {
            return Some(DeclaredValue::Literal(ty));
        }
        if let Some(signatures) = source.functions.get(name) {
            return Some(DeclaredValue::Function(signatures));
        }
        if source.private_vars.contains(name) {
            return None;
        }
    }
    externs()?.get(name).map(DeclaredValue::Function)
}

#[derive(Default, PartialEq, Eq)]
pub(super) struct LocalInterface {
    names: Names,
    dependencies: Vec<String>,
    specs: Arc<BTreeMap<String, Type>>,
    literals: BTreeMap<String, LiteralVar>,
    functions: Vec<LocalFunction>,
}

#[derive(PartialEq, Eq)]
struct LocalFunction {
    name: String,
    signatures: Arc<[Signature]>,
    private: bool,
    inferred: bool,
}

pub(crate) struct InterfaceKey {
    local: Arc<LocalInterface>,
    dependencies: Vec<Dependency>,
}

struct Dependency {
    source: Option<Arc<Exports>>,
    externs: Option<Arc<Signatures>>,
    in_project: bool,
}

impl Dependency {
    fn same_tables(&self, before: &Self) -> bool {
        fn same_arc<T>(this: &Option<Arc<T>>, before: &Option<Arc<T>>) -> bool {
            match (this, before) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }
        self.in_project == before.in_project
            && same_arc(&self.source, &before.source)
            && same_arc(&self.externs, &before.externs)
    }

    fn value(&self, name: &str) -> Option<DeclaredValue<'_>> {
        visible_value(self.source.as_deref(), || self.externs.as_deref(), name)
    }
}

pub(crate) struct InterfaceChanges<'a> {
    same_local: bool,
    namespaces: BTreeMap<&'a str, (&'a Dependency, &'a Dependency)>,
}

impl InterfaceKey {
    pub fn changes<'a>(&'a self, before: &'a Self) -> InterfaceChanges<'a> {
        let same_local = self.local == before.local;
        let namespaces = if same_local {
            std::iter::once(self.local.names.namespace.as_str())
                .chain(self.local.dependencies.iter().map(String::as_str))
                .zip(self.dependencies.iter().zip(&before.dependencies))
                .filter(|(_, (current, old))| !current.same_tables(old))
                .collect()
        } else {
            BTreeMap::new()
        };
        InterfaceChanges {
            same_local,
            namespaces,
        }
    }
}

impl InterfaceChanges<'_> {
    pub fn is_unchanged(&self) -> bool {
        self.same_local && self.namespaces.is_empty()
    }

    pub fn allows_reuse(&self, model: &Model, function: &Function) -> bool {
        if !self.same_local {
            return false;
        }
        if self.namespaces.is_empty() {
            return true;
        }
        let names = &model.names;
        let mut unchanged = |name: &str| {
            let (namespace, local) = names.resolve_parts(name);
            self.identical_var(namespace, local)
                && ((name.contains('/') && name != "/")
                    || names.refers.contains_key(name)
                    || namespace == names.namespace
                    || self.identical_var(&names.namespace, name))
        };
        std::iter::once(function)
            .chain(
                model
                    .helpers(function)
                    .into_iter()
                    .map(|name| &model.functions[name]),
            )
            .all(|function| {
                function.arities.values().all(|arity| {
                    arity
                        .params
                        .iter()
                        .chain(&arity.body)
                        .all(|form| symbols_match(form, &mut unchanged))
                })
            })
    }

    fn identical_var(&self, namespace: &str, local: &str) -> bool {
        self.namespaces.get(namespace).is_none_or(|(current, old)| {
            let qualified = format!("{namespace}/{local}");
            current.in_project == old.in_project
                && current.value(&qualified) == old.value(&qualified)
        })
    }
}

impl Model {
    pub(super) fn freeze_interfaces(
        &mut self,
        specs: BTreeMap<String, Type>,
        dependencies: Vec<String>,
    ) {
        let specs = Arc::new(specs);
        let mut exports = Exports::default();
        let mut functions = Vec::with_capacity(self.functions.len());
        for f in self.functions.values() {
            if f.private {
                exports.private_vars.insert(f.name.clone());
            } else {
                exports
                    .functions
                    .insert(f.name.clone(), f.signatures.clone());
            }
            functions.push(LocalFunction {
                name: f.name.clone(),
                signatures: f.signatures.clone(),
                private: f.private,
                inferred: f.inferred,
            });
        }
        for (name, value) in &self.literals {
            if value.private {
                exports.private_vars.insert(name.clone());
            } else {
                exports.literals.insert(name.clone(), value.ty.clone());
            }
        }
        self.exports = Arc::new(exports);
        self.local_interface = Arc::new(LocalInterface {
            names: self.names.clone(),
            dependencies,
            specs,
            literals: self.literals.clone(),
            functions,
        });
    }

    pub fn publish(&self, globals: &mut Globals) {
        globals
            .exports
            .insert(self.names.namespace.clone(), self.exports.clone());
    }

    pub fn interface_key(&self, globals: &Globals) -> InterfaceKey {
        // Extern-only vars can exist in the source's own namespace without
        // adding an edge to the project's namespace load order.
        let own = std::iter::once((self.names.namespace.as_str(), false));
        let dependencies = own
            .chain(
                self.local_interface
                    .dependencies
                    .iter()
                    .map(|name| (name.as_str(), true)),
            )
            .map(|(name, external)| Dependency {
                source: external
                    .then(|| globals.exports.get(name).cloned())
                    .flatten(),
                externs: globals
                    .externs
                    .as_ref()
                    .and_then(|table| table.namespaces.get(name))
                    .cloned(),
                in_project: globals.project_namespaces.contains(name),
            })
            .collect();
        InterfaceKey {
            local: self.local_interface.clone(),
            dependencies,
        }
    }
}
