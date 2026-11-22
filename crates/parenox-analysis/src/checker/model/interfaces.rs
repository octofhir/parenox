//! Immutable declarations used to validate cached checks.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{
    Contract, DeclaredValue, Function, Globals, LiteralVar, Model, Names, SpecConversion,
    SpecProvider, UncheckedFunction,
    declarations::symbols_match,
    externs::{ExternalDeclaration, ExternalDeclarations},
};
use crate::checker::FunctionCheck;
use crate::types::{Arity, Type};

pub(super) type Contracts = BTreeMap<String, Contract>;

#[derive(Clone, Default)]
pub(super) struct Exports {
    pub functions: Contracts,
    pub ordinary_dotted_functions: BTreeSet<String>,
    pub literals: BTreeMap<String, LiteralVar>,
    unchecked_functions: BTreeMap<String, UncheckedFunction>,
    macros: BTreeMap<String, bool>,
    private_vars: BTreeSet<String>,
}

impl Exports {
    pub(super) fn unchecked_arities(&self, qualified: &str) -> Option<&[Arity]> {
        self.unchecked_functions.get(qualified)?.arities.as_deref()
    }

    pub(super) fn public_var_names(&self) -> impl Iterator<Item = &str> {
        self.functions
            .keys()
            .chain(self.literals.keys())
            .chain(self.unchecked_functions.keys())
            .chain(
                self.macros
                    .iter()
                    .filter_map(|(name, private)| (!private).then_some(name)),
            )
            .map(|qualified| qualified.split_once('/').unwrap().1)
    }

    fn same_function_contracts(&self, previous: &Self) -> bool {
        self.ordinary_dotted_functions == previous.ordinary_dotted_functions
            && self.functions.len() == previous.functions.len()
            && self.functions.iter().zip(&previous.functions).all(
                |((name, contract), (old_name, old))| {
                    name == old_name
                        && contract.signatures == old.signatures
                        && contract.guard == old.guard
                },
            )
    }
}

impl Globals {
    pub(super) fn may_expand_initializer_macro(&self, qualified: &str) -> bool {
        let Some((namespace, _)) = qualified.split_once('/') else {
            return false;
        };
        self.exports.get(namespace).map_or_else(
            || self.project_namespaces.contains(namespace),
            |source| source.macros.contains_key(qualified),
        ) || matches!(
            self.extern_declaration(qualified),
            Some(ExternalDeclaration::Macro(_))
        )
    }

    pub(crate) fn proves_ordinary_dotted_function(&self, namespace: &str, name: &str) -> bool {
        self.exports.get(namespace).is_some_and(|source| {
            !source.ordinary_dotted_functions.is_empty()
                && source
                    .ordinary_dotted_functions
                    .contains(&format!("{namespace}/{name}"))
        })
    }

    pub(crate) fn same_source_function_contracts(&self, previous: &Self) -> bool {
        let current = self
            .exports
            .iter()
            .filter(|(_, ns)| !ns.functions.is_empty());
        let before = previous
            .exports
            .iter()
            .filter(|(_, ns)| !ns.functions.is_empty());
        current.clone().count() == before.clone().count()
            && current
                .zip(before)
                .all(|((name, namespace), (old_name, old))| {
                    name == old_name
                        && (Arc::ptr_eq(namespace, old) || namespace.same_function_contracts(old))
                })
    }
}

pub(super) fn visible_value<'a>(
    source: Option<&'a Exports>,
    externs: impl FnOnce() -> Option<&'a ExternalDeclarations>,
    name: &str,
) -> Option<DeclaredValue<'a>> {
    if let Some(source) = source {
        if let Some(private) = source.macros.get(name) {
            return Some(if *private {
                DeclaredValue::PrivateMacro
            } else {
                source_macro_value(externs().and_then(|table| table.get(name)))
            });
        }
        if let Some(value) = source.literals.get(name) {
            return Some(DeclaredValue::Literal(value));
        }
        if let Some(contract) = source.functions.get(name) {
            return Some(DeclaredValue::Function(contract));
        }
        if source.private_vars.contains(name) {
            return None;
        }
    }
    if let Some(declaration) = externs().and_then(|table| table.get(name)) {
        return Some(match declaration {
            ExternalDeclaration::Function(contract) => DeclaredValue::Function(contract),
            ExternalDeclaration::Macro(scope) => DeclaredValue::Macro(scope),
            ExternalDeclaration::Spec(_) | ExternalDeclaration::PendingSpec(_) => return None,
            ExternalDeclaration::Literal(literal) => DeclaredValue::ExternLiteral(literal),
        });
    }
    source?
        .unchecked_functions
        .get(name)
        .map(DeclaredValue::UncheckedFunction)
}

pub(super) fn source_macro_value(external: Option<&ExternalDeclaration>) -> DeclaredValue<'_> {
    match external {
        Some(ExternalDeclaration::Macro(scope)) => DeclaredValue::Macro(scope),
        _ => DeclaredValue::OpaqueMacro,
    }
}

#[derive(Default, PartialEq, Eq)]
pub(super) struct LocalInterface {
    names: Names,
    dependencies: Vec<String>,
    specs: Arc<BTreeMap<String, Type>>,
    literals: BTreeMap<String, LiteralVar>,
    functions: BTreeMap<String, LocalFunction>,
    unchecked_functions: BTreeMap<String, UncheckedFunction>,
}

#[derive(PartialEq, Eq)]
struct LocalFunction {
    contract: Option<Contract>,
    private: bool,
    ordinary_dotted: bool,
}

pub(crate) struct InterfaceKey {
    local: Arc<LocalInterface>,
    dependencies: Vec<Dependency>,
    core_in_project: bool,
    unavailable_all_refers: bool,
    has_declaration_findings: bool,
    specs: Arc<BTreeMap<String, Type>>,
    spec_conversions: Arc<BTreeMap<String, SpecConversion>>,
    spec_providers: Arc<BTreeMap<String, Arc<SpecProvider>>>,
}

struct Dependency {
    source: Option<Arc<Exports>>,
    externs: Option<Arc<ExternalDeclarations>>,
    in_project: bool,
}

impl Dependency {
    fn proves_ordinary_dotted_function(&self, name: &str) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.ordinary_dotted_functions.contains(name))
    }

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
    compatible_scope: bool,
    same_declaration_findings: bool,
    local: Option<(&'a LocalInterface, &'a LocalInterface)>,
    namespaces: BTreeMap<&'a str, (&'a Dependency, &'a Dependency)>,
}

impl InterfaceKey {
    pub fn same_specs(&self, before: &Self) -> bool {
        Arc::ptr_eq(&self.specs, &before.specs)
            && Arc::ptr_eq(&self.spec_conversions, &before.spec_conversions)
            && Arc::ptr_eq(&self.spec_providers, &before.spec_providers)
    }

    pub fn changes<'a>(&'a self, before: &'a Self) -> InterfaceChanges<'a> {
        let same_local = self.local == before.local;
        let compatible_scope = self.core_in_project == before.core_in_project
            && self.unavailable_all_refers == before.unavailable_all_refers
            && (same_local
                || (self.local.names == before.local.names
                    && self.local.dependencies == before.local.dependencies
                    && self.local.specs == before.local.specs));
        let namespaces = if compatible_scope {
            std::iter::once(self.local.names.namespace.as_str())
                .chain(self.local.dependencies.iter().map(String::as_str))
                .zip(self.dependencies.iter().zip(&before.dependencies))
                .filter(|(_, (current, old))| !current.same_tables(old))
                .collect()
        } else {
            BTreeMap::new()
        };
        InterfaceChanges {
            compatible_scope,
            same_declaration_findings: self.has_declaration_findings
                == before.has_declaration_findings,
            local: (compatible_scope && !same_local).then_some((&self.local, &before.local)),
            namespaces,
        }
    }
}

impl InterfaceChanges<'_> {
    pub fn is_unchanged(&self) -> bool {
        self.compatible_scope
            && self.same_declaration_findings
            && self.local.is_none()
            && self.namespaces.is_empty()
    }

    pub fn allows_reuse(&self, model: &Model, function: &Function, result: &FunctionCheck) -> bool {
        if !self.compatible_scope
            || (!self.same_declaration_findings && result.declaration_findings_sensitive)
            || self.local.is_some_and(|(current, old)| {
                current.functions.get(&function.name) != old.functions.get(&function.name)
            })
        {
            return false;
        }
        if self.is_unchanged() {
            return true;
        }
        // Cached messages can contain the original binder names.
        let allow_binder_renames = result.diagnostics.is_empty();
        let names = &model.names;
        let mut unchanged = |name: &str| {
            let (namespace, local) = names.resolve_parts(name);
            self.identical_var(namespace, local, allow_binder_renames)
                && ((name.contains('/') && name != "/")
                    || names.refers.contains_key(name)
                    || namespace == names.namespace
                    || self.identical_var(&names.namespace, name, allow_binder_renames))
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
                        .chain(&arity.preconditions)
                        .chain(arity.postconditions.iter().flatten())
                        .chain(&arity.body)
                        .all(|form| symbols_match(form, &mut unchanged))
                })
            })
    }

    fn identical_var(&self, namespace: &str, local: &str, allow_binder_renames: bool) -> bool {
        let external = self.namespaces.get(namespace);
        let local_vars = self
            .local
            .filter(|(current, _)| current.names.namespace == namespace);
        if external.is_none() && local_vars.is_none() {
            return true;
        }
        let qualified = format!("{namespace}/{local}");
        local_vars.is_none_or(|(current, old)| {
            same_local_function(
                current.functions.get(&qualified),
                old.functions.get(&qualified),
                allow_binder_renames,
            ) && current.literals.get(&qualified) == old.literals.get(&qualified)
                && current.unchecked_functions.get(&qualified)
                    == old.unchecked_functions.get(&qualified)
        }) && external.is_none_or(|(current, old)| {
            current.in_project == old.in_project
                && current.proves_ordinary_dotted_function(&qualified)
                    == old.proves_ordinary_dotted_function(&qualified)
                && same_visible_value(
                    current.value(&qualified),
                    old.value(&qualified),
                    allow_binder_renames,
                )
        })
    }
}

fn same_local_function(
    a: Option<&LocalFunction>,
    b: Option<&LocalFunction>,
    allow_binder_renames: bool,
) -> bool {
    a == b
        || allow_binder_renames
            && a.zip(b).is_some_and(|(a, b)| {
                a.private == b.private
                    && a.ordinary_dotted == b.ordinary_dotted
                    && a.contract
                        .as_ref()
                        .zip(b.contract.as_ref())
                        .is_some_and(|(a, b)| a.same_interface(b))
            })
}

fn same_visible_value(
    a: Option<DeclaredValue<'_>>,
    b: Option<DeclaredValue<'_>>,
    allow_binder_renames: bool,
) -> bool {
    a == b
        || allow_binder_renames
            && matches!((a, b), (Some(DeclaredValue::Function(a)), Some(DeclaredValue::Function(b))) if a.same_interface(b))
}

impl Model {
    pub(super) fn freeze_interfaces(
        &mut self,
        specs: BTreeMap<String, Type>,
        dependencies: Vec<String>,
    ) {
        let specs = Arc::new(specs);
        let mut exports = Exports::default();
        let mut functions = BTreeMap::new();
        for f in self.functions.values() {
            if f.private {
                exports.private_vars.insert(f.name.clone());
            } else if let Some(contract) = &f.contract {
                exports.functions.insert(f.name.clone(), contract.clone());
                if f.ordinary_dotted
                    && !self.literals.contains_key(&f.name)
                    && !self.unchecked_functions.contains_key(&f.name)
                {
                    exports.ordinary_dotted_functions.insert(f.name.clone());
                }
            }
            functions.insert(
                f.name.clone(),
                LocalFunction {
                    contract: f.contract.clone(),
                    private: f.private,
                    ordinary_dotted: f.ordinary_dotted,
                },
            );
        }
        for (name, value) in &self.literals {
            if value.private {
                exports.private_vars.insert(name.clone());
            } else {
                exports.literals.insert(name.clone(), value.clone());
            }
        }
        for (name, function) in &self.unchecked_functions {
            if function.private {
                exports.private_vars.insert(name.clone());
            } else {
                exports
                    .unchecked_functions
                    .insert(name.clone(), function.clone());
            }
        }
        for (local, declarations) in &self.names.source_macros {
            let Some(declaration) = declarations.last().filter(|entry| entry.replaced.is_none())
            else {
                continue;
            };
            let name = format!("{}/{local}", self.namespace());
            exports.functions.remove(&name);
            exports.ordinary_dotted_functions.remove(&name);
            exports.literals.remove(&name);
            exports.unchecked_functions.remove(&name);
            exports.private_vars.remove(&name);
            exports.macros.insert(name, declaration.private);
        }
        self.exports = Arc::new(exports);
        self.local_interface = Arc::new(LocalInterface {
            names: self.names.clone(),
            dependencies,
            specs,
            literals: self.literals.clone(),
            functions,
            unchecked_functions: self.unchecked_functions.clone(),
        });
    }

    pub fn publish(&self, globals: &mut Globals, unambiguous: bool) {
        let mut exports = self.exports.clone();
        if !unambiguous && !exports.ordinary_dotted_functions.is_empty() {
            Arc::make_mut(&mut exports)
                .ordinary_dotted_functions
                .clear();
        }
        globals
            .exports
            .insert(self.names.namespace.clone(), exports);
    }

    pub fn interface_key(&self, globals: &Globals, has_declaration_findings: bool) -> InterfaceKey {
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
            core_in_project: globals.project_namespaces.contains("clojure.core"),
            unavailable_all_refers: self.unavailable_all_refers,
            has_declaration_findings,
            specs: globals.specs.clone(),
            spec_conversions: globals.spec_conversions.clone(),
            spec_providers: globals.spec_providers.clone(),
        }
    }
}
