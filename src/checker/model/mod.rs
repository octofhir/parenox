//! Namespace resolution, project interfaces and cache dependencies.
mod contracts;
mod externs;
mod require;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use externs::Externs;

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

#[derive(Default)]
pub(crate) struct Globals {
    specs: BTreeMap<String, Type>,
    pub(super) project_namespaces: BTreeSet<String>,
    functions: BTreeMap<String, Signature>,
    interfaces: BTreeMap<String, String>,
    externs: Option<Arc<Externs>>,
}

impl Globals {
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
}

#[derive(Default)]
pub(crate) struct Model {
    pub ns: String,
    pub dependencies: Vec<String>,
    refers: BTreeMap<String, String>,
    externs: Option<Arc<Externs>>,
    aliases: BTreeMap<String, String>,
    raw_specs: BTreeMap<String, Form>,
    specs: BTreeMap<String, Type>,
    external_specs: BTreeMap<String, Option<Type>>,
    pub functions: BTreeMap<String, Function>,
    // Conservative references to unannotated helpers, including transitive cache dependencies.
    helper_dependencies: BTreeMap<String, BTreeSet<String>>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Model {
    pub fn context(forms: &[Form]) -> Self {
        let mut model = Self {
            ns: "user".into(),
            ..Self::default()
        };
        let mut seen_ns = false;
        for form in forms {
            if let Kind::List(xs) = &form.without_metadata().kind
                && xs.first().and_then(Form::symbol) == Some("ns")
            {
                if seen_ns {
                    model.error("PNX9001", form.span, "Only one ns declaration is supported");
                }
                seen_ns = true;
                model.read_ns(xs, form.span);
            }
        }
        let mut dependencies: BTreeSet<_> = model.dependencies.iter().cloned().collect();
        fn visit(form: &Form, model: &Model, dependencies: &mut BTreeSet<String>) {
            match &form.kind {
                Kind::Symbol(name) => {
                    let resolved = model.resolve(name);
                    if let Some((namespace, _)) = resolved.rsplit_once('/') {
                        dependencies.insert(namespace.to_string());
                    }
                }
                Kind::List(xs) | Kind::Vector(xs) | Kind::Map(xs) | Kind::Set(xs) => {
                    for x in xs {
                        visit(x, model, dependencies);
                    }
                }
                Kind::Quote(x) => visit(x, model, dependencies),
                Kind::Metadata(metadata, target) => {
                    visit(metadata, model, dependencies);
                    visit(target, model, dependencies);
                }
                _ => {}
            }
        }
        for form in forms {
            visit(form, &model, &mut dependencies);
        }
        dependencies.remove(&model.ns);
        model.dependencies = dependencies.into_iter().collect();
        model
    }

    pub fn build(forms: &[Form], globals: &Globals) -> Self {
        let mut model = Self::context(forms);
        let mut definitions = Vec::new();
        for form in forms {
            let Kind::List(xs) = &form.without_metadata().kind else {
                model.error("PNX9001", form.span, "Unsupported top-level form");
                continue;
            };
            let Some(op) = xs.first().and_then(Form::symbol) else {
                model.error("PNX9001", form.span, "Expected a top-level declaration");
                continue;
            };
            match model.resolve(op).as_str() {
                "clojure.core/ns" => {}
                "clojure.spec.alpha/def" if xs.len() == 3 => {
                    if let Some(key) = xs[1].keyword() {
                        let key = model.key(key);
                        if model.raw_specs.insert(key.clone(), xs[2].clone()).is_some() {
                            model.error("PNX2790", xs[1].span, format!("Duplicate spec {key}"));
                        }
                    } else {
                        model.error("PNX9001", xs[1].span, "Spec name must be a keyword");
                    }
                }
                "clojure.core/defn" | "clojure.core/defn-" => {
                    definitions.push((xs, form.span));
                }
                _ => model.error(
                    "PNX9001",
                    form.span,
                    format!("Unsupported top-level declaration {op}"),
                ),
            }
        }
        for key in model.raw_specs.keys().cloned().collect::<Vec<_>>() {
            model.spec(&key, &mut Vec::new(), model.raw_specs[&key].span, globals);
        }
        for (xs, span) in definitions {
            model.read_function(xs, span, globals);
        }
        model.collect_helpers();
        model
    }

    fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    pub(super) fn resolve(&self, name: &str) -> String {
        if let Some((prefix, tail)) = name
            .split_once('/')
            .filter(|(prefix, _)| !prefix.is_empty())
        {
            format!(
                "{}/{tail}",
                self.aliases.get(prefix).map_or(prefix, String::as_str)
            )
        } else {
            self.refers
                .get(name)
                .cloned()
                .unwrap_or_else(|| format!("clojure.core/{name}"))
        }
    }

    pub(super) fn function_name(&self, name: &str) -> String {
        if name.contains('/') && name != "/" {
            self.resolve(name)
        } else {
            self.refers
                .get(name)
                .cloned()
                .unwrap_or_else(|| format!("{}/{name}", self.ns))
        }
    }

    pub(super) fn key(&self, key: &str) -> String {
        if let Some(tail) = key.strip_prefix("::") {
            if let Some((alias, name)) = tail.split_once('/') {
                format!(
                    ":{}/{name}",
                    self.aliases.get(alias).map_or(alias, String::as_str)
                )
            } else {
                format!(":{}/{tail}", self.ns)
            }
        } else {
            key.to_string()
        }
    }

    fn read_ns(&mut self, xs: &[Form], span: Span) {
        let Some(name) = xs.get(1).and_then(Form::symbol) else {
            self.error("PNX9001", span, "ns requires a namespace name");
            return;
        };
        self.ns = name.to_string();
        for clause in xs.iter().skip(2) {
            // ns docstrings and attr-maps are declarations, not executable forms.
            if matches!(clause.kind, Kind::String(_) | Kind::Map(_)) {
                continue;
            }
            let Kind::List(items) = &clause.without_metadata().kind else {
                self.error("PNX9001", clause.span, "Unsupported ns clause");
                continue;
            };
            if items.first().and_then(Form::keyword) != Some(":require") {
                self.error(
                    "PNX9001",
                    clause.span,
                    "Only :require ns clauses are supported",
                );
                continue;
            }
            for lib in items.iter().skip(1) {
                self.read_libspec(lib);
            }
        }
    }

    pub fn publish(&self, globals: &mut Globals) {
        globals.specs.extend(self.specs.clone());
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
        globals
            .interfaces
            .insert(self.ns.clone(), format!("{:?}|{exported:?}", self.specs));
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
            "{:?}|{:?}|{:?}|{:?}|{sigs:?}|{}",
            self.ns,
            self.aliases,
            self.refers,
            self.specs,
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

    pub fn function_key(&self, function: &Function, source: &str) -> String {
        let mut key = format!(
            "{:?}|{}",
            function.span,
            &source[function.span.start.offset..function.span.end.offset]
        );
        for name in self.helpers(function) {
            let helper = &self.functions[name];
            key.push_str(&format!(
                "|{name}:{:?}|{}",
                helper.span,
                &source[helper.span.start.offset..helper.span.end.offset]
            ));
        }
        key
    }

    pub(super) fn signature<'a>(
        &'a self,
        name: &str,
        globals: &'a Globals,
    ) -> Option<(&'a [Type], &'a Type)> {
        let qualified = self.function_name(name);
        if let Some(function) = self.functions.get(&qualified) {
            if function.inferred {
                None
            } else {
                Some((&function.args, &function.ret))
            }
        } else {
            globals
                .signature(&qualified)
                .or_else(|| globals.signature(&self.resolve(name)))
                .map(|s| (s.args.as_slice(), &s.ret))
        }
    }
}
