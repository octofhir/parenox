//! Namespace resolution, project interfaces and cache dependencies.
mod contracts;
mod declarations;
mod externs;
mod fdefs;
mod guards;
mod imports;
mod interfaces;
mod references;
mod require;
mod specs;
mod type_forms;
mod vars;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) use declarations::Declarations;
use declarations::Names;
pub(super) use externs::MacroScope;
pub(crate) use externs::{EncodeError, ExternCache, SnapshotWriter};
use externs::{ExternFile, Externs};
pub(super) use guards::VerifiedGuard;
pub(crate) use interfaces::InterfaceKey;
use interfaces::{Exports, LocalInterface, visible_value};
pub(crate) use references::{ProjectCall, ScopedReference};
pub(crate) use specs::{SpecCache, SpecFingerprint, SpecSource};
use vars::LiteralVar;

use crate::reader::{Form, Kind, Span};
use crate::types::{Arity, Signature, Type};
use crate::{ContractKind, ContractOrigin, Diagnostic};

#[derive(Clone, Debug)]
pub(crate) struct Function {
    pub name: String,
    pub private: bool,
    pub arities: BTreeMap<Arity, FunctionArity>,
    pub contract: Option<Contract>,
    pub span: Span,
}

impl Function {
    // Private definitions without a signature are analyzed through direct typed callers.
    pub fn inferred(&self) -> bool {
        self.contract.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Contract {
    pub signatures: Arc<[Signature]>,
    pub origin: Arc<ContractOrigin>,
    pub guard: Option<Arc<VerifiedGuard>>,
}

impl Contract {
    fn new(signatures: Vec<Signature>, name: &str, kind: ContractKind, path: &str) -> Self {
        Self {
            signatures: signatures.into(),
            origin: Arc::new(ContractOrigin {
                name: name.into(),
                kind,
                path: Some(path.into()),
            }),
            guard: None,
        }
    }

    fn same_interface(&self, other: &Self) -> bool {
        self.origin == other.origin
            && self.guard == other.guard
            && self.signatures.len() == other.signatures.len()
            && self
                .signatures
                .iter()
                .zip(other.signatures.iter())
                .all(|(a, b)| a.same_type(b))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FunctionArity {
    pub params: Vec<Form>,
    pub body: Vec<Form>,
    pub span: Span,
}

#[derive(PartialEq, Eq)]
pub(super) enum DeclaredValue<'a> {
    Literal(&'a Type),
    ExternLiteral(&'a Type, &'a ContractOrigin),
    Function(&'a Contract),
    Macro(&'a MacroScope),
}

#[derive(Default)]
pub(crate) struct Globals {
    specs: Arc<BTreeMap<String, Type>>,
    pub(super) project_namespaces: BTreeSet<String>,
    exports: BTreeMap<String, Arc<Exports>>,
    externs: Option<Arc<Externs>>,
}

impl Globals {
    pub fn with_specs(specs: Arc<BTreeMap<String, Type>>) -> Self {
        Self {
            specs,
            ..Self::default()
        }
    }

    pub fn register_namespace(&mut self, namespace: &str) {
        self.project_namespaces.insert(namespace.into());
    }

    fn declared_value(&self, name: &str) -> Option<DeclaredValue<'_>> {
        let (namespace, _) = name.split_once('/')?;
        visible_value(
            self.exports.get(namespace).map(Arc::as_ref),
            || {
                self.externs
                    .as_ref()
                    .and_then(|table| table.namespaces.get(namespace))
                    .map(Arc::as_ref)
            },
            name,
        )
    }
}

#[derive(Default)]
pub(crate) struct Model {
    names: Names,
    path: String,
    externs: Option<Arc<ExternFile>>,
    literals: BTreeMap<String, LiteralVar>,
    exports: Arc<Exports>,
    local_interface: Arc<LocalInterface>,
    external_specs: BTreeMap<String, Option<Type>>,
    external_refers: BTreeMap<String, Option<Arc<Exports>>>,
    guard_core_in_project: Option<bool>,
    pub functions: BTreeMap<String, Function>,
    // Conservative references to unannotated helpers, including transitive cache dependencies.
    helper_dependencies: BTreeMap<String, BTreeSet<String>>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Model {
    pub fn build(
        declarations: &Declarations,
        forms: &[Form],
        source: &str,
        path: &str,
        globals: &Globals,
    ) -> Self {
        let resolved;
        let declarations = if declarations.all_refers.is_empty() {
            declarations
        } else {
            let mut imports = Declarations::read_with_imports(forms, Some(globals));
            if imports.specs.keys().ne(declarations.specs.keys()) {
                let span = forms.first().map_or(Span::default(), |form| form.span);
                imports.error(
                    "PNX9001",
                    span,
                    "Use qualified spec declarations when :refer :all changes their resolution",
                );
            }
            resolved = imports;
            &resolved
        };
        let mut model = Self {
            names: declarations.names.clone(),
            path: path.into(),
            diagnostics: declarations.diagnostics.clone(),
            external_refers: declarations
                .all_refers
                .iter()
                .map(|namespace| (namespace.clone(), globals.exports.get(namespace).cloned()))
                .collect(),
            ..Self::default()
        };
        let mut specs = BTreeMap::new();
        for key in declarations.specs.keys() {
            let ty = globals.specs.get(key).cloned();
            specs.insert(key.clone(), ty.clone().unwrap_or(Type::Unknown));
            model.external_specs.insert(key.clone(), ty);
        }
        for &index in &declarations.literal_vars {
            let form = &forms[index];
            let Kind::List(xs) = &form.without_metadata().kind else {
                unreachable!()
            };
            model.read_def(xs, form.span, source, globals);
        }
        for &index in &declarations.unchecked_vars {
            let Kind::List(xs) = &forms[index].without_metadata().kind else {
                unreachable!()
            };
            model.read_unchecked_var(xs, declarations.private_var(xs));
        }
        let mut fdefs = model.collect_fdefs(&declarations.fdefs, forms);
        let mut guards = BTreeMap::new();
        for &index in &declarations.functions {
            let form = &forms[index];
            let Kind::List(xs) = &form.without_metadata().kind else {
                unreachable!()
            };
            model.read_function(xs, form.span, globals, &mut fdefs, &mut guards);
        }
        for (local, var) in &declarations.forward_vars {
            let qualified = format!("{}/{local}", model.namespace());
            if !model.literals.contains_key(&qualified) && !model.functions.contains_key(&qualified)
            {
                model.names.refers.remove(local);
                model.literals.insert(
                    qualified.clone(),
                    LiteralVar {
                        ty: Type::Unknown,
                        private: var.private,
                    },
                );
                model.diagnostics.push(Diagnostic::analysis_gap(
                    "PNX9001",
                    var.span,
                    format!("Declared var {qualified} has no source definition"),
                ));
            }
        }
        for (name, parts) in fdefs {
            model.error(
                "PNX9001",
                parts[1].span,
                format!("s/fdef has no supported function definition for {name}"),
            );
        }
        model.verify_guards(guards, declarations, forms, globals);
        model.collect_helpers();
        model.freeze_interfaces(specs, declarations.dependencies.clone());
        model
    }

    fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    pub(super) fn namespace(&self) -> &str {
        &self.names.namespace
    }

    pub(super) fn resolve(&self, name: &str) -> String {
        self.names.resolve(name)
    }

    pub(super) fn class_name<'a>(&'a self, symbol: &'a str) -> &'a str {
        self.names
            .imports
            .get(symbol)
            .map_or(symbol, String::as_str)
    }

    pub(super) fn key(&self, key: &str) -> String {
        self.names.key(key)
    }

    pub(super) fn function_name(&self, name: &str) -> String {
        if name.contains('/') && name != "/" {
            self.resolve(name)
        } else {
            self.names
                .refers
                .get(name)
                .cloned()
                .unwrap_or_else(|| format!("{}/{name}", self.names.namespace))
        }
    }

    pub fn refresh_external_inputs(&mut self, globals: &Globals) -> bool {
        self.guard_core_in_project
            .is_none_or(|present| present == globals.project_namespaces.contains("clojure.core"))
            && self
                .external_specs
                .iter()
                .all(|(key, previous)| globals.specs.get(key) == previous.as_ref())
            && self
                .external_refers
                .iter_mut()
                .all(|(namespace, previous)| {
                    match (previous.as_ref(), globals.exports.get(namespace)) {
                        (Some(old), Some(current)) if Arc::ptr_eq(old, current) => true,
                        (Some(old), Some(current))
                            if old.public_var_names().eq(current.public_var_names()) =>
                        {
                            *previous = Some(current.clone());
                            true
                        }
                        (None, None) => true,
                        _ => false,
                    }
                })
    }

    pub(super) fn local_function(&self, name: &str) -> Option<&Function> {
        self.functions.get(&self.function_name(name))
    }

    fn collect_helpers(&mut self) {
        if !self.functions.values().any(Function::inferred) {
            return;
        }
        fn visit(forms: &[Form], model: &Model, names: &mut BTreeSet<String>) {
            for form in forms {
                match &form.kind {
                    Kind::Symbol(name) => {
                        if let Some(function) = model.local_function(name).filter(|f| f.inferred())
                        {
                            names.insert(function.name.clone());
                        }
                    }
                    Kind::List(xs) if xs.first().and_then(Form::symbol) == Some("quote") => {}
                    Kind::List(xs) | Kind::Vector(xs) | Kind::Map(xs) | Kind::Set(xs) => {
                        visit(xs, model, names)
                    }
                    Kind::AnonymousFunction(function) => visit(&function.body, model, names),
                    Kind::Metadata(_, target) => visit(std::slice::from_ref(target), model, names),
                    _ => {}
                }
            }
        }
        fn visit_defaults(form: &Form, model: &Model, names: &mut BTreeSet<String>) {
            match &form.without_metadata().kind {
                Kind::Map(entries) => {
                    for pair in entries.as_chunks::<2>().0 {
                        match pair[0].keyword() {
                            Some(":or") => {
                                if let Some(defaults) = pair[1].map() {
                                    for default in defaults.as_chunks::<2>().0 {
                                        visit(std::slice::from_ref(&default[1]), model, names);
                                    }
                                }
                            }
                            Some(":req!") => visit_defaults(&pair[1], model, names),
                            _ => visit_defaults(&pair[0], model, names),
                        }
                    }
                }
                Kind::Vector(forms) => {
                    for form in forms {
                        visit_defaults(form, model, names);
                    }
                }
                _ => {}
            }
        }
        for function in self.functions.values() {
            let mut names = BTreeSet::new();
            for arity in function.arities.values() {
                for param in &arity.params {
                    visit_defaults(param, self, &mut names);
                }
                visit(&arity.body, self, &mut names);
            }
            self.helper_dependencies
                .insert(function.name.clone(), names);
        }
        let reachable: BTreeSet<_> = self
            .functions
            .values()
            .filter(|f| !f.inferred())
            .flat_map(|f| self.helpers(f))
            .collect();
        let unused: Vec<_> = self
            .functions
            .values()
            .filter(|f| f.inferred() && !reachable.contains(f.name.as_str()))
            .map(|f| (f.span, f.name.clone()))
            .collect();
        for (span, name) in unused {
            self.error("PNX9002", span, format!(
                "Cannot infer unused private function {name}; add a :t/sig annotation or a direct typed caller"
            ));
        }
    }

    fn helpers(&self, function: &Function) -> BTreeSet<&str> {
        let mut pending: Vec<_> = self
            .helper_dependencies
            .get(&function.name)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        let mut names = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if names.insert(name) {
                pending.extend(
                    self.helper_dependencies
                        .get(name)
                        .into_iter()
                        .flatten()
                        .map(String::as_str),
                );
            }
        }
        names
    }

    pub fn function_ranges(&self, function: &Function) -> Vec<Span> {
        std::iter::once(function.span)
            .chain(
                self.helpers(function)
                    .into_iter()
                    .map(|name| self.functions[name].span),
            )
            .collect()
    }

    pub(super) fn declared_value<'a>(
        &'a self,
        name: &str,
        globals: &'a Globals,
    ) -> Option<DeclaredValue<'a>> {
        let qualified = self.function_name(name);
        if let Some(value) = self.literals.get(&qualified) {
            return Some(DeclaredValue::Literal(&value.ty));
        }
        if let Some(function) = self.functions.get(&qualified) {
            return function.contract.as_ref().map(DeclaredValue::Function);
        }
        globals
            .declared_value(&qualified)
            .or_else(|| globals.declared_value(&self.resolve(name)))
    }
}
