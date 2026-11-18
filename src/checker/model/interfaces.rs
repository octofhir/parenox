//! Immutable declarations used to validate cached checks.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{Globals, LiteralVar, Model, Names, Signature};
use crate::types::Type;

pub(super) type Signatures = BTreeMap<String, Arc<[Signature]>>;

#[derive(Default, PartialEq, Eq)]
pub(super) struct Exports {
    pub functions: Signatures,
    pub literals: BTreeMap<String, Type>,
    specs: Arc<BTreeMap<String, Type>>,
}

impl Exports {
    pub(super) fn public_var_names(&self) -> impl Iterator<Item = &str> {
        self.functions
            .keys()
            .chain(self.literals.keys())
            .map(|qualified| qualified.split_once('/').unwrap().1)
    }
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

#[derive(PartialEq, Eq)]
pub(crate) struct InterfaceKey {
    local: Arc<LocalInterface>,
    dependencies: Vec<Dependency>,
}

#[derive(PartialEq, Eq)]
struct Dependency {
    source: Option<Arc<Exports>>,
    externs: Option<Arc<Signatures>>,
}

impl Model {
    pub(super) fn freeze_interfaces(
        &mut self,
        specs: BTreeMap<String, Type>,
        dependencies: Vec<String>,
    ) {
        let specs = Arc::new(specs);
        let mut exports = Exports {
            specs: specs.clone(),
            ..Exports::default()
        };
        let mut functions = Vec::with_capacity(self.functions.len());
        for f in self.functions.values() {
            if !f.private {
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
        for (name, value) in self.literals.iter().filter(|(_, value)| !value.private) {
            exports.literals.insert(name.clone(), value.ty.clone());
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
        let dependencies = self
            .local_interface
            .dependencies
            .iter()
            .map(|name| Dependency {
                source: globals.exports.get(name).cloned(),
                externs: globals
                    .externs
                    .as_ref()
                    .and_then(|table| table.namespaces.get(name))
                    .cloned(),
            })
            .collect();
        InterfaceKey {
            local: self.local_interface.clone(),
            dependencies,
        }
    }
}
