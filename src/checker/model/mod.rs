//! Namespace resolution, project interfaces and cache dependencies.
mod contracts;
mod declarations;
mod externs;
mod require;
mod specs;
mod type_forms;
mod vars;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) use declarations::Declarations;
use declarations::Names;
use externs::Externs;
pub(crate) use specs::{SpecCache, SpecFingerprint, SpecSource};
use vars::LiteralVar;

use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

#[derive(Clone, Debug)]
pub(crate) struct Function {
    pub name: String,
    pub private: bool,
    // Private definitions without a signature are analyzed through direct typed callers.
    pub inferred: bool,
    pub params: Vec<Form>,
    pub args: Vec<Type>,
    pub ret: Type,
    pub body: Vec<Form>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Signature {
    pub args: Vec<Type>,
    pub ret: Type,
}

pub(super) enum DeclaredValue<'a> {
    Literal(&'a Type),
    Function(&'a [Type], &'a Type),
}

impl DeclaredValue<'_> {
    pub(super) fn known(self) -> Type {
        match self {
            Self::Literal(ty) => ty.clone(),
            Self::Function(args, ret) => Type::Function(args.to_vec(), Box::new(ret.clone())),
        }
    }
}

#[derive(Default)]
pub(crate) struct Globals {
    specs: Arc<BTreeMap<String, Type>>,
    pub(super) project_namespaces: BTreeSet<String>,
    functions: BTreeMap<String, Signature>,
    literals: BTreeMap<String, Type>,
    interfaces: BTreeMap<String, String>,
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

    pub fn dependency_key(&self, dependencies: &[String]) -> String {
        dependencies
            .iter()
            .map(|name| {
                let source = self.interfaces.get(name);
                let external = self
                    .externs
                    .as_ref()
                    .and_then(|table| table.interfaces.get(name));
                format!("{name}:{source:?}:{external:?}")
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    fn signature(&self, name: &str) -> Option<&Signature> {
        self.externs
            .as_ref()
            .and_then(|table| table.functions.get(name))
            .or_else(|| self.functions.get(name))
    }

    fn declared_value(&self, name: &str) -> Option<DeclaredValue<'_>> {
        if let Some(ty) = self.literals.get(name) {
            return Some(DeclaredValue::Literal(ty));
        }
        self.signature(name)
            .map(|sig| DeclaredValue::Function(&sig.args, &sig.ret))
    }
}

#[derive(Default)]
pub(crate) struct Model {
    names: Names,
    dependencies: Vec<String>,
    externs: Option<Arc<Externs>>,
    literals: BTreeMap<String, LiteralVar>,
    specs: BTreeMap<String, Type>,
    external_specs: BTreeMap<String, Option<Type>>,
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
        globals: &Globals,
    ) -> Self {
        let mut model = Self {
            names: declarations.names.clone(),
            dependencies: declarations.dependencies.clone(),
            diagnostics: declarations.diagnostics.clone(),
            ..Self::default()
        };
        for key in declarations.specs.keys() {
            let ty = globals.specs.get(key).cloned();
            model
                .specs
                .insert(key.clone(), ty.clone().unwrap_or(Type::Unknown));
            model.external_specs.insert(key.clone(), ty);
        }
        for &index in &declarations.literal_vars {
            let form = &forms[index];
            let Kind::List(xs) = &form.without_metadata().kind else {
                unreachable!()
            };
            model.read_def(xs, form.span, source, globals);
        }
        for &index in &declarations.functions {
            let form = &forms[index];
            let Kind::List(xs) = &form.without_metadata().kind else {
                unreachable!()
            };
            model.read_function(xs, form.span, globals);
        }
        model.collect_helpers();
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

    pub fn publish(&self, globals: &mut Globals) {
        let mut exported = BTreeMap::new();
        for function in self.functions.values().filter(|f| !f.private) {
            let signature = Signature {
                args: function.args.clone(),
                ret: function.ret.clone(),
            };
            globals
                .functions
                .insert(function.name.clone(), signature.clone());
            exported.insert(function.name.clone(), signature);
        }
        let mut literals = BTreeMap::new();
        for (name, value) in self.literals.iter().filter(|(_, value)| !value.private) {
            globals.literals.insert(name.clone(), value.ty.clone());
            literals.insert(name, &value.ty);
        }
        globals.interfaces.insert(
            self.names.namespace.clone(),
            format!("{:?}|{exported:?}|{literals:?}", self.specs),
        );
    }

    pub fn external_specs_current(&self, globals: &Globals) -> bool {
        self.external_specs
            .iter()
            .all(|(key, previous)| globals.specs.get(key) == previous.as_ref())
    }

    pub fn interface_key(&self, globals: &Globals) -> String {
        let sigs: Vec<_> = self
            .functions
            .values()
            .map(|f| (&f.name, &f.args, &f.ret))
            .collect();
        format!(
            "{:?}|{:?}|{sigs:?}|{:?}|{}",
            self.names,
            self.specs,
            self.literals,
            globals.dependency_key(&self.dependencies)
        )
    }

    pub(super) fn local_function(&self, name: &str) -> Option<&Function> {
        self.functions.get(&self.function_name(name))
    }

    fn collect_helpers(&mut self) {
        if !self.functions.values().any(|f| f.inferred) {
            return;
        }
        fn visit(forms: &[Form], model: &Model, names: &mut BTreeSet<String>) {
            for form in forms {
                match &form.kind {
                    Kind::Symbol(name) => {
                        if let Some(function) = model.local_function(name).filter(|f| f.inferred) {
                            names.insert(function.name.clone());
                        }
                    }
                    Kind::List(xs) | Kind::Vector(xs) | Kind::Map(xs) | Kind::Set(xs) => {
                        visit(xs, model, names)
                    }
                    Kind::Quote(x) => visit(std::slice::from_ref(x), model, names),
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
            for param in &function.params {
                visit_defaults(param, self, &mut names);
            }

            visit(&function.body, self, &mut names);
            self.helper_dependencies
                .insert(function.name.clone(), names);
        }
        let reachable: BTreeSet<_> = self
            .functions
            .values()
            .filter(|f| !f.inferred)
            .flat_map(|f| self.helpers(f))
            .collect();
        let unused: Vec<_> = self
            .functions
            .values()
            .filter(|f| f.inferred && !reachable.contains(f.name.as_str()))
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
            return (!function.inferred)
                .then_some(DeclaredValue::Function(&function.args, &function.ret));
        }
        globals
            .declared_value(&qualified)
            .or_else(|| globals.declared_value(&self.resolve(name)))
    }
}
