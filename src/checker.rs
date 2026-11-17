use std::collections::{BTreeMap, BTreeSet};

use crate::expand::{self, Threading};
use crate::reader::{Form, Kind, Span};
use crate::types::{Field, MapType, Type};
use crate::{Diagnostic, Suggestion};

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
    project_namespaces: BTreeSet<String>,
    functions: BTreeMap<String, Signature>,
    interfaces: BTreeMap<String, String>,
}

impl Globals {
    pub fn register_namespace(&mut self, namespace: &str) {
        self.project_namespaces.insert(namespace.into());
    }

    pub fn dependency_key(&self, dependencies: &[String]) -> String {
        dependencies
            .iter()
            .map(|name| format!("{name}:{:?}", self.interfaces.get(name)))
            .collect::<Vec<_>>()
            .join("|")
    }
}

#[derive(Default)]
pub(crate) struct Model {
    pub ns: String,
    pub dependencies: Vec<String>,
    refers: BTreeMap<String, String>,
    externs: BTreeMap<String, Signature>,
    aliases: BTreeMap<String, String>,
    raw_specs: BTreeMap<String, Form>,
    specs: BTreeMap<String, Type>,
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

    fn resolve(&self, name: &str) -> String {
        if let Some((prefix, tail)) = name.split_once('/') {
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

    fn function_name(&self, name: &str) -> String {
        if name.contains('/') {
            self.resolve(name)
        } else {
            self.refers
                .get(name)
                .cloned()
                .unwrap_or_else(|| format!("{}/{name}", self.ns))
        }
    }

    fn key(&self, key: &str) -> String {
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
                let Some(parts) = lib.vector() else {
                    self.error("PNX9001", lib.span, "Expected [namespace :as alias]");
                    continue;
                };
                let Some(namespace) = parts.first().and_then(Form::symbol) else {
                    self.error("PNX9001", lib.span, "Expected a required namespace");
                    continue;
                };
                self.dependencies.push(namespace.into());
                if !(parts.len() - 1).is_multiple_of(2) {
                    self.error("PNX9001", lib.span, "Expected require option/value pairs");
                    continue;
                }
                for [option, value] in parts[1..].as_chunks::<2>().0 {
                    match option.keyword() {
                        Some(":as" | ":as-alias") if value.symbol().is_some() => {
                            self.aliases
                                .insert(value.symbol().unwrap().into(), namespace.into());
                        }
                        Some(":refer") if value.vector().is_some() => {
                            for name in value.vector().unwrap() {
                                if let Some(name) = name.symbol() {
                                    self.refers
                                        .insert(name.into(), format!("{namespace}/{name}"));
                                } else {
                                    self.error("PNX9001", name.span, "Expected a referred symbol");
                                }
                            }
                        }
                        _ => self.error("PNX9001", option.span, "Unsupported require option"),
                    }
                }
            }
        }
    }

    fn spec(
        &mut self,
        key: &str,
        visiting: &mut Vec<String>,
        span: Span,
        globals: &Globals,
    ) -> Type {
        if let Some(ty) = self.specs.get(key) {
            return ty.clone();
        }
        if !self.raw_specs.contains_key(key)
            && let Some(ty) = globals.specs.get(key)
        {
            return ty.clone();
        }
        if visiting.iter().any(|k| k == key) {
            self.error(
                "PNX9001",
                span,
                format!("Recursive spec {key} is not supported yet"),
            );
            return Type::Unknown;
        }
        if visiting.len() >= 128 {
            self.error("PNX9001", span, "Spec expansion limit (128) exceeded");
            return Type::Unknown;
        }
        let Some(form) = self.raw_specs.get(key).cloned() else {
            self.error("PNX9001", span, format!("No declaration for spec {key}"));
            return Type::Unknown;
        };
        visiting.push(key.to_string());
        let ty = self.parse_type(&form, visiting, globals);
        visiting.pop();
        self.specs.insert(key.to_string(), ty.clone());
        ty
    }

    fn parse_type(&mut self, form: &Form, visiting: &mut Vec<String>, globals: &Globals) -> Type {
        match &form.without_metadata().kind {
            Kind::Symbol(s) => match self.resolve(s).as_str() {
                "clojure.core/string?" => Type::String,
                "clojure.core/char?" => Type::Character,
                "clojure.core/number?" => Type::Number,
                "clojure.core/boolean?" => Type::Boolean,
                "clojure.core/keyword?" => Type::Keyword,
                "clojure.core/nil?" => Type::Nil,
                "clojure.core/map?" => Type::Map(MapType {
                    fields: BTreeMap::new(),
                    open: true,
                }),
                _ => {
                    self.error("PNX9001", form.span, format!("Unsupported type {s}"));
                    Type::Unknown
                }
            },
            Kind::Keyword(key) => self.spec(&self.key(key), visiting, form.span, globals),
            Kind::Set(xs) => {
                let mut types = Vec::new();
                for x in xs {
                    types.push(match &x.kind {
                        Kind::String(s) => Type::Str(s.clone()),
                        Kind::Character(c) => Type::Char(*c),
                        Kind::Keyword(k) => Type::Key(self.key(k)),
                        Kind::Bool(b) => Type::Bool(*b),
                        Kind::Nil => Type::Nil,
                        _ => {
                            self.error("PNX9001", x.span, "Unsupported literal union member");
                            Type::Unknown
                        }
                    });
                }
                Type::union(types)
            }
            Kind::List(xs) => {
                let op = xs.first().and_then(Form::symbol).map(|s| self.resolve(s));
                match op.as_deref() {
                    Some("clojure.spec.alpha/nilable") if xs.len() == 2 => {
                        self.parse_type(&xs[1], visiting, globals).nilable()
                    }
                    Some("clojure.spec.alpha/or") if xs.len() >= 3 && xs.len() % 2 == 1 => {
                        let mut types = Vec::new();
                        for pair in xs[1..].as_chunks::<2>().0 {
                            if pair[0].keyword().is_none() {
                                self.error("PNX9001", pair[0].span, "s/or requires keyword tags");
                            }
                            types.push(self.parse_type(&pair[1], visiting, globals));
                        }
                        Type::union(types)
                    }
                    Some("clojure.spec.alpha/keys") if xs.len() % 2 == 1 => {
                        let mut fields = BTreeMap::new();
                        for pair in xs[1..].as_chunks::<2>().0 {
                            let directive = pair[0].keyword().unwrap_or("");
                            if !matches!(directive, ":req" | ":opt" | ":req-un" | ":opt-un") {
                                self.error("PNX9001", pair[0].span, "Unsupported s/keys directive");
                                continue;
                            }
                            let Some(keys) = pair[1].vector() else {
                                self.error("PNX9001", pair[1].span, "s/keys requires key vectors");
                                continue;
                            };
                            for key in keys {
                                let Some(k) = key.keyword() else {
                                    self.error("PNX9001", key.span, "Expected a spec keyword");
                                    continue;
                                };
                                let qualified = self.key(k);
                                if !qualified.contains('/') {
                                    self.error(
                                        "PNX9001",
                                        key.span,
                                        "s/keys needs qualified spec keys",
                                    );
                                }
                                let ty = self.spec(&qualified, visiting, key.span, globals);
                                let field_key = if directive.ends_with("-un") {
                                    format!(":{}", qualified.rsplit('/').next().unwrap())
                                } else {
                                    qualified
                                };
                                let required = directive.starts_with(":req");
                                if fields
                                    .insert(field_key.clone(), Field { ty, required })
                                    .is_some()
                                {
                                    self.error(
                                        "PNX9001",
                                        key.span,
                                        format!("Repeated field {field_key}"),
                                    );
                                }
                            }
                        }
                        Type::Map(MapType { fields, open: true })
                    }
                    Some("clojure.spec.alpha/coll-of")
                        if xs.len() == 4
                            && xs[2].keyword() == Some(":kind")
                            && matches!(
                                xs[3].symbol().map(|s| self.resolve(s)).as_deref(),
                                Some("clojure.core/vector?" | "clojure.core/set?")
                            ) =>
                    {
                        let element = Box::new(self.parse_type(&xs[1], visiting, globals));
                        if xs[3].symbol().map(|s| self.resolve(s)).as_deref()
                            == Some("clojure.core/set?")
                        {
                            Type::Set(element)
                        } else {
                            Type::Vector(element)
                        }
                    }
                    _ => {
                        self.error("PNX9001", form.span, "Unsupported type form");
                        Type::Unknown
                    }
                }
            }
            _ => {
                self.error("PNX9001", form.span, "Expected a type declaration");
                Type::Unknown
            }
        }
    }

    fn read_function(&mut self, xs: &[Form], span: Span, globals: &Globals) {
        let Some(name) = xs.get(1).and_then(Form::symbol) else {
            self.error("PNX9001", span, "defn requires a function name");
            return;
        };
        let name = self.function_name(name);
        let mut index = 2;
        if xs
            .get(index)
            .is_some_and(|x| matches!(x.kind, Kind::String(_)))
        {
            index += 1;
        }
        let attrs = if let Some(attrs) = xs.get(index).and_then(Form::map) {
            index += 1;
            attrs
        } else {
            &[]
        };
        let default_private = xs[0]
            .symbol()
            .is_some_and(|s| self.resolve(s) == "clojure.core/defn-");
        let private_form = attrs
            .as_chunks::<2>()
            .0
            .iter()
            .find(|p| p[0].keyword() == Some(":private"))
            .map(|p| &p[1])
            .or_else(|| {
                if default_private {
                    None
                } else {
                    xs[1].metadata(":private")
                }
            });
        let private = if let Some(form) = private_form {
            match form.without_metadata().kind {
                Kind::Bool(value) => value,
                _ => {
                    self.error(
                        "PNX9001",
                        form.span,
                        ":private currently requires a literal boolean",
                    );
                    default_private
                }
            }
        } else {
            default_private
        };
        let signature = attrs
            .as_chunks::<2>()
            .0
            .iter()
            .find(|p| p[0].keyword() == Some(":t/sig"))
            .map(|p| &p[1])
            .or_else(|| xs[1].metadata(":t/sig"));
        let inferred = signature.is_none();
        if inferred && !private {
            self.error(
                "PNX9002",
                span,
                format!("Public function {name} needs a :t/sig annotation"),
            );
            return;
        }
        let signature = if let Some(signature) = signature {
            if !matches!(signature.without_metadata().kind, Kind::Quote(_)) {
                self.error("PNX9001", signature.span, ":t/sig must be quoted data");
            }
            let Some(signature) = self.parse_signature(signature, globals) else {
                return;
            };
            Some(signature)
        } else {
            None
        };
        let Some(params) = xs.get(index).and_then(Form::vector) else {
            self.error("PNX9001", span, "Only single-arity defn is supported");
            return;
        };
        if params.iter().any(|p| p.symbol() == Some("&")) {
            self.error(
                "PNX9001",
                xs[index].span,
                "Variadic definitions are not supported yet",
            );
            return;
        }
        let signature = signature.unwrap_or_else(|| Signature {
            args: vec![Type::Unknown; params.len()],
            ret: Type::Unknown,
        });
        if params.len() != signature.args.len() || xs.len() <= index + 1 {
            self.error(
                "PNX2554",
                span,
                "Signature arity must match parameters; body is required",
            );
            return;
        }
        let function = Function {
            name: name.clone(),
            private,
            inferred,
            params: params.to_vec(),
            args: signature.args,
            ret: signature.ret,
            body: xs[index + 1..].to_vec(),
            span,
        };
        if self.functions.insert(name.clone(), function).is_some() {
            self.error("PNX2790", span, format!("Duplicate function {name}"));
        }
    }

    fn parse_signature(&mut self, form: &Form, globals: &Globals) -> Option<Signature> {
        let Some(types) = form.unquoted().vector() else {
            self.error("PNX9001", form.span, "Expected [Arg ... :-> Result]");
            return None;
        };
        if types.len() < 2
            || types[types.len() - 2].keyword() != Some(":->")
            || types[..types.len() - 2]
                .iter()
                .any(|t| t.keyword() == Some(":->"))
        {
            self.error("PNX9001", form.span, "Expected [Arg ... :-> Result]");
            return None;
        }
        let args = types[..types.len() - 2]
            .iter()
            .map(|t| self.parse_type(t, &mut Vec::new(), globals))
            .collect();
        let ret = self.parse_type(types.last().unwrap(), &mut Vec::new(), globals);
        Some(Signature { args, ret })
    }

    pub fn externs(forms: &[Form], globals: &Globals) -> Self {
        let mut model = Self {
            ns: "externs".into(),
            ..Self::default()
        };
        let [
            Form {
                kind: Kind::Map(entries),
                ..
            },
        ] = forms
        else {
            model.error(
                "PNX9001",
                forms.first().map_or(Span::default(), |f| f.span),
                "Externs must be one EDN map of qualified function names to signatures",
            );
            return model;
        };
        for [name, signature] in entries.as_chunks::<2>().0 {
            let Some(name_string) = name.symbol().filter(|s| s.contains('/')) else {
                model.error(
                    "PNX9001",
                    name.span,
                    "Extern function names must be qualified symbols",
                );
                continue;
            };
            if let Some(signature) = model.parse_signature(signature, globals)
                && model
                    .externs
                    .insert(name_string.into(), signature)
                    .is_some()
            {
                model.error(
                    "PNX2790",
                    name.span,
                    format!("Duplicate extern {name_string}"),
                );
            }
        }
        model
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
        let mut external_namespaces: BTreeMap<&str, Vec<_>> = BTreeMap::new();
        for (name, signature) in &self.externs {
            globals.functions.insert(name.clone(), signature.clone());
            external_namespaces
                .entry(name.rsplit_once('/').unwrap().0)
                .or_default()
                .push((name, signature));
        }
        for (namespace, signatures) in external_namespaces {
            globals
                .interfaces
                .entry(namespace.into())
                .or_default()
                .push_str(&format!("|{signatures:?}"));
        }
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

    fn local_function(&self, name: &str) -> Option<&Function> {
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

    fn signature<'a>(&'a self, name: &str, globals: &'a Globals) -> Option<(&'a [Type], &'a Type)> {
        let qualified = self.function_name(name);
        if let Some(function) = self.functions.get(&qualified) {
            if function.inferred {
                None
            } else {
                Some((&function.args, &function.ret))
            }
        } else {
            globals
                .functions
                .get(&qualified)
                .or_else(|| globals.functions.get(&self.resolve(name)))
                .map(|s| (s.args.as_slice(), &s.ret))
        }
    }

    pub fn check_function(
        &self,
        function: &Function,
        source: &str,
        globals: &Globals,
    ) -> Vec<Diagnostic> {
        let mut checker = Checker {
            model: self,
            globals,
            source,
            diagnostics: Vec::new(),
            active_helpers: Vec::new(),
            thread_depth: 0,
            specializations: BTreeMap::new(),
        };
        let env = checker.bind_parameters(&function.params, |index| {
            Value::new(function.args[index].clone())
        });
        let result = checker.body(&function.body, &env);
        checker.expect(
            &result,
            &function.ret,
            function.body.last().unwrap().span,
            &format!("Return value of {}", function.name),
        );
        checker.diagnostics
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Value {
    // Inferred language type for ordinary use and diagnostics, e.g. String or Set<String>.
    ty: Type,
    // More precise static knowledge, e.g. a string literal or finite union of values.
    known: Type,
    notes: Vec<String>,
}

impl Value {
    fn new(known: Type) -> Self {
        Self {
            ty: known.inferred(),
            known,
            notes: Vec::new(),
        }
    }

    fn with_notes(known: Type, notes: Vec<String>) -> Self {
        Self {
            notes,
            ..Self::new(known)
        }
    }

    fn join(mut self, other: Value) -> Value {
        if self.known == Type::Never {
            return other;
        }
        if other.known == Type::Never {
            return self;
        }
        for note in other.notes {
            if !self.notes.contains(&note) {
                self.notes.push(note);
            }
        }
        Value::with_notes(self.known.join_values(other.known), self.notes)
    }

    fn narrow(&mut self, known: Type) {
        self.ty = known.inferred();
        self.known = known;
    }
}

type Env = BTreeMap<String, Value>;
type Defaults = BTreeMap<String, (Value, Span)>;

#[derive(Clone, Copy)]
enum Lookup<'a> {
    Optional(Option<&'a Value>),
    Required,
    Presence,
}

struct Checker<'a> {
    model: &'a Model,
    globals: &'a Globals,
    source: &'a str,
    diagnostics: Vec<Diagnostic>,
    active_helpers: Vec<String>,
    thread_depth: usize,
    specializations: BTreeMap<(String, Vec<Value>), (Value, Vec<Diagnostic>)>,
}

impl Checker<'_> {
    fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    fn unsupported(&mut self, span: Span, message: impl Into<String>) -> Value {
        self.error("PNX9001", span, message);
        Value::new(Type::Unknown)
    }

    fn expect(&mut self, actual: &Value, expected: &Type, span: Span, context: &str) {
        if actual.ty.contains_unknown() || expected.contains_unknown() {
            return; // The originating unsupported/unresolved form has its own diagnostic.
        }
        if !actual.known.subtype_of(expected) {
            let reported_type = if expected.constrains_literal_values() {
                &actual.known
            } else {
                &actual.ty
            };
            let mut diagnostic = Diagnostic::new(
                "PNX2322",
                span,
                format!("{context}: expected {expected}, got {reported_type}"),
            );
            diagnostic.expected = Some(expected.to_string());
            diagnostic.actual = Some(reported_type.to_string());
            diagnostic.notes = actual.notes.clone();
            let expression = &self.source[span.start.offset..span.end.offset];
            if *expected == Type::String
                && actual.ty.without_nil() != actual.ty
                && actual.ty.without_nil() != Type::Never
                && actual.ty.without_nil().subtype_of(expected)
            {
                diagnostic.suggestions.push(Suggestion {
                    message: "handle Nil explicitly; choose a fallback for your application".into(),
                    span,
                    replacement: format!("(or {expression} \"\")"),
                    applicability: "requires-review",
                });
            } else if *expected == Type::Number
                && let Type::Str(value) = &actual.known
                && value
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '+' || c == '-')
                && value.parse::<f64>().is_ok()
                && crate::reader::read(value)
                    .is_ok_and(|forms| forms.len() == 1 && matches!(forms[0].kind, Kind::Number))
            {
                diagnostic.suggestions.push(Suggestion {
                    message: "use a number literal if this value is intended to be numeric".into(),
                    span,
                    replacement: value.clone(),
                    applicability: "requires-review",
                });
            } else {
                diagnostic.help = Some(if context.starts_with("Return value of ") {
                    format!("Return a value of type {expected} to satisfy the declared signature.")
                } else {
                    format!(
                        "Provide a value of type {expected}; validate or convert the input before using it here."
                    )
                });
            }
            self.diagnostics.push(diagnostic);
        }
    }

    fn body(&mut self, body: &[Form], env: &Env) -> Value {
        let mut value = Value::new(Type::Nil);
        for form in body {
            value = self.infer(form, env);
        }
        value
    }

    fn bind_parameters(&mut self, params: &[Form], value: impl Fn(usize) -> Value) -> Env {
        let mut env = Env::new();
        // All ordinary arguments exist before fn destructuring, including later ones.
        // Destructuring then proceeds in parameter order and can shadow those names.
        for (index, param) in params
            .iter()
            .enumerate()
            .filter(|(_, p)| p.symbol().is_some())
        {
            self.bind(param, value(index), &mut env);
        }
        for (index, param) in params
            .iter()
            .enumerate()
            .filter(|(_, p)| p.symbol().is_none())
        {
            self.bind(param, value(index), &mut env);
        }
        env
    }

    fn bind(&mut self, form: &Form, value: Value, env: &mut Env) {
        match &form.without_metadata().kind {
            Kind::Symbol(name) => {
                env.insert(name.clone(), value);
            }
            Kind::Map(xs) => {
                // :missing changes checked lookups into collection of absent keys.
                // Do not apply throwing semantics before this mode is implemented.
                if xs
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|p| p[0].keyword() == Some(":missing"))
                {
                    self.error(
                        "PNX9001",
                        form.span,
                        "Destructuring with :missing is not supported yet",
                    );
                    return;
                }
                // Clojure evaluates every default once in the outer lexical scope,
                // before introducing this map's locals and :as alias.
                let defaults = xs
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .find(|p| p[0].keyword() == Some(":or"))
                    .map(|p| self.binding_defaults(&p[1], env))
                    .unwrap_or_default();
                for pair in xs.as_chunks::<2>().0 {
                    let directive = pair[0].keyword().map(|key| self.model.key(key));
                    match directive.as_deref() {
                        Some(":or") => {}
                        Some(":as") if pair[1].symbol().is_some() => {
                            env.insert(pair[1].symbol().unwrap().into(), value.clone());
                        }
                        Some(":req!") => {
                            let Some(entries) = pair[1].map() else {
                                self.error(
                                    "PNX9001",
                                    pair[1].span,
                                    ":req! requires a map of bindings to keys",
                                );
                                continue;
                            };
                            for entry in entries.as_chunks::<2>().0 {
                                self.bind_entry(entry, &value, true, &defaults, env);
                            }
                        }
                        Some(directive)
                            if matches!(
                                directive.rsplit('/').next(),
                                Some(":keys" | ":keys!" | "keys" | "keys!")
                            ) =>
                        {
                            self.bind_keys(directive, &pair[1], &value, &defaults, env);
                        }
                        _ => self.bind_entry(pair, &value, false, &defaults, env),
                    }
                }
            }
            _ => self.error("PNX9001", form.span, "Unsupported binding pattern"),
        }
    }

    fn bind_entry(
        &mut self,
        pair: &[Form; 2],
        value: &Value,
        required: bool,
        defaults: &Defaults,
        env: &mut Env,
    ) {
        if !matches!(
            pair[0].without_metadata().kind,
            Kind::Symbol(_) | Kind::Map(_) | Kind::Vector(_)
        ) {
            self.error(
                "PNX9001",
                pair[0].span,
                "Unsupported destructuring directive",
            );
            return;
        }
        let Some(key) = pair[1].keyword() else {
            self.error(
                "PNX9001",
                pair[1].span,
                "Destructuring currently requires literal keyword keys",
            );
            return;
        };
        let key = self.model.key(key);
        let mode = if required {
            Lookup::Required
        } else {
            Lookup::Optional(None)
        };
        let field = self.binding_value(value, &key, pair[0].symbol(), mode, defaults, pair[1].span);
        self.bind(&pair[0], field, env);
    }

    fn bind_keys(
        &mut self,
        directive: &str,
        form: &Form,
        value: &Value,
        defaults: &Defaults,
        env: &mut Env,
    ) {
        let Some(keys) = form.vector() else {
            self.error(
                "PNX9001",
                form.span,
                format!("{directive} requires a vector"),
            );
            return;
        };
        let namespace = directive.rsplit_once('/').map(|(ns, _)| ns);
        let required = directive.ends_with("keys!");
        let mut presence_only = false;
        for key in keys {
            if key.symbol() == Some("&") {
                if presence_only {
                    self.error("PNX9001", key.span, "A keys binding can contain only one &");
                }
                presence_only = true;
                continue;
            }
            let (local, mut lookup_key) = if presence_only {
                let Some(key_name) = key.keyword() else {
                    self.error(
                        "PNX9001",
                        key.span,
                        "Keys after & must be literal keywords; they do not bind locals",
                    );
                    continue;
                };
                (None, self.model.key(key_name))
            } else {
                let Some(name) = key.symbol() else {
                    self.error(
                        "PNX9001",
                        key.span,
                        "A keys binding requires symbols before &",
                    );
                    continue;
                };
                if namespace.is_some() && name.contains('/') {
                    self.error(
                        "PNX9001",
                        key.span,
                        "Namespaced keys directives require unqualified binding symbols",
                    );
                    continue;
                }
                if name.starts_with('/') || name.ends_with('/') || name.matches('/').count() > 1 {
                    self.error("PNX9001", key.span, "Invalid symbol in keys binding");
                    continue;
                }
                let local = name.rsplit('/').next().unwrap();

                (Some(local), format!(":{name}"))
            };
            if let Some(ns) = namespace
                && !lookup_key.contains('/')
            {
                lookup_key = format!("{ns}/{}", lookup_key.trim_start_matches(':'));
            }
            if let Some(local) = local {
                let mode = if required {
                    Lookup::Required
                } else {
                    Lookup::Optional(None)
                };
                let field =
                    self.binding_value(value, &lookup_key, Some(local), mode, defaults, key.span);
                env.insert(local.into(), field);
            } else if required {
                self.binding_value(
                    value,
                    &lookup_key,
                    None,
                    Lookup::Presence,
                    defaults,
                    key.span,
                );
            }
        }
    }

    fn binding_defaults(&mut self, form: &Form, env: &Env) -> Defaults {
        let mut defaults = Defaults::new();
        let Some(entries) = form.map() else {
            self.error(
                "PNX1001",
                form.span,
                ":or requires a map of locals or keyword keys to defaults",
            );
            return defaults;
        };
        for pair in entries.as_chunks::<2>().0 {
            let value = self.infer(&pair[1], env);
            let key = if let Some(symbol) = pair[0].symbol() {
                symbol.to_owned()
            } else if let Some(keyword) = pair[0].keyword() {
                self.model.key(keyword)
            } else {
                self.error(
                    "PNX9001",
                    pair[0].span,
                    "Defaults currently require local symbols or literal keyword keys",
                );
                continue;
            };
            if defaults
                .insert(key.clone(), (value, pair[0].span))
                .is_some()
            {
                self.error(
                    "PNX2790",
                    pair[0].span,
                    format!("Duplicate :or default {key}"),
                );
            }
        }
        defaults
    }

    fn binding_value(
        &mut self,
        value: &Value,
        key: &str,
        local: Option<&str>,
        mode: Lookup<'_>,
        defaults: &Defaults,
        span: Span,
    ) -> Value {
        let by_local = local.and_then(|name| defaults.get(name));
        let by_key = defaults.get(key);
        if let (Some(_), Some((_, span))) = (by_local, by_key) {
            self.error(
                "PNX1001",
                *span,
                format!(
                    "Both local {} and key {key} have :or defaults; choose one",
                    local.unwrap()
                ),
            );
            return Value::new(Type::Unknown);
        }
        let default = by_local.or(by_key);
        match mode {
            Lookup::Optional(_) => self.lookup(value, key, default.map(|(value, _)| value), span),
            Lookup::Required | Lookup::Presence => {
                if let Some((_, span)) = default {
                    self.error(
                        "PNX1001",
                        *span,
                        format!("Required key {key} cannot have an :or default"),
                    );
                    Value::new(Type::Unknown)
                } else {
                    self.checked_lookup(value, key, mode, span)
                }
            }
        }
    }

    fn infer(&mut self, form: &Form, env: &Env) -> Value {
        match &form.kind {
            Kind::Nil => Value::new(Type::Nil),
            Kind::Bool(b) => Value::new(Type::Bool(*b)),
            Kind::Number => Value::new(Type::Number),
            Kind::String(s) => Value::new(Type::Str(s.clone())),
            Kind::Character(c) => Value::new(Type::Char(*c)),
            Kind::Metadata(_, target) => self.infer(target, env),
            Kind::Conditional { .. } => self.unsupported(form.span, "Reader conditional must be selected before type analysis"),
            Kind::Tagged(tag, _) => self.unsupported(form.span, format!(
                "Tagged literal #{} has no declared reader semantics; its reader function was not executed",
                tag.symbol().unwrap())),
            Kind::Keyword(k) => Value::new(Type::Key(self.model.key(k))),
            Kind::Symbol(s) => {
                if let Some(value) = env.get(s) {
                    value.clone()
                } else if self.model.local_function(s).is_some_and(|f| f.inferred) {
                    self.error(
                        "PNX9002",
                        form.span,
                        format!(
                            "Private function {s} needs a :t/sig annotation when used as a value"
                        ),
                    );
                    Value::new(Type::Unknown)
                } else if let Some((args, ret)) = self.model.signature(s, self.globals) {
                    Value::new(Type::Function(args.to_vec(), Box::new(ret.clone())))
                } else {
                    let qualified = self.model.function_name(s);
                    let external = qualified.rsplit_once('/').is_some_and(|(namespace, _)| {
                        namespace != self.model.ns
                            && !self.globals.project_namespaces.contains(namespace)
                    });
                    self.error(
                        if external { "PNX9003" } else { "PNX2304" },
                        form.span,
                        if external {
                            format!("No declared signature for external value {s}")
                        } else {
                            format!("Unresolved or private value {s}")
                        },
                    );
                    Value::new(Type::Unknown)
                }
            }
            Kind::Vector(xs) | Kind::Set(xs) => {
                let types = xs
                    .iter()
                    .map(|x| self.infer(x, env).known)
                    .collect::<Vec<_>>();
                let element = Box::new(Type::union(types));
                Value::new(if matches!(form.kind, Kind::Set(_)) {
                    Type::Set(element)
                } else {
                    Type::Vector(element)
                })
            }
            Kind::Map(xs) => {
                let mut fields = BTreeMap::new();
                for pair in xs.as_chunks::<2>().0 {
                    let value = self.infer(&pair[1], env);
                    if let Some(key) = pair[0].keyword() {
                        let key = self.model.key(key);
                        if fields
                            .insert(
                                key.clone(),
                                Field {
                                    ty: value.known,
                                    required: true,
                                },
                            )
                            .is_some()
                        {
                            self.error("PNX2790", pair[0].span, format!("Duplicate map key {key}"));
                        }
                    } else {
                        self.unsupported(pair[0].span, "Only keyword map keys are supported");
                    }
                }
                Value::new(Type::Map(MapType {
                    fields,
                    open: false,
                }))
            }
            Kind::List(xs) if xs.is_empty() => {
                self.unsupported(form.span, "Empty list values are not supported yet")
            }
            Kind::List(xs) => self.list(xs, form.span, env),
            Kind::Quote(_) => self.unsupported(form.span, "Quoted data is not supported yet"),
        }
    }

    fn inferred_call(&mut self, function: &Function, args: Vec<Value>, span: Span) -> Value {
        if args.len() != function.params.len() {
            self.error(
                "PNX2554",
                span,
                format!(
                    "{} expects {} arguments, got {}",
                    function.name,
                    function.params.len(),
                    args.len()
                ),
            );
            return Value::new(Type::Unknown);
        }
        if self.active_helpers.contains(&function.name) {
            self.error(
                "PNX9002",
                span,
                format!(
                    "Recursive private function {} needs a :t/sig annotation",
                    function.name
                ),
            );
            return Value::new(Type::Unknown);
        }
        let key = (function.name.clone(), args);
        let start = self.diagnostics.len();
        let result = if let Some((value, diagnostics)) = self.specializations.get(&key) {
            self.diagnostics.extend(diagnostics.iter().cloned());
            value.clone()
        } else {
            if self.active_helpers.len() >= 32
                || self.specializations.len() + self.active_helpers.len() >= 256
            {
                self.error("PNX9002", span,
                    "Private inference limit reached (32 nested calls or 256 specializations); add :t/sig annotations");
                return Value::new(Type::Unknown);
            }
            self.active_helpers.push(function.name.clone());
            let env = self.bind_parameters(&function.params, |index| key.1[index].clone());
            let result = self.body(&function.body, &env);
            self.active_helpers.pop();
            self.specializations
                .insert(key, (result.clone(), self.diagnostics[start..].to_vec()));
            result
        };
        for diagnostic in &mut self.diagnostics[start..] {
            diagnostic.notes.push(format!(
                "While checking call to {} at {}:{}",
                function.name, span.start.line, span.start.column
            ));
        }
        result
    }

    fn list(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if let Some(name) = xs[0].symbol()
            && !env.contains_key(name)
            && let Some(function) = self.model.local_function(name).filter(|f| f.inferred)
        {
            let args = xs[1..].iter().map(|x| self.infer(x, env)).collect();
            return self.inferred_call(function, args, span);
        }
        if let Some(key) = xs[0].keyword() {
            if !(2..=3).contains(&xs.len()) {
                self.error(
                    "PNX2554",
                    span,
                    "Keyword lookup requires a map or set and optional default",
                );
                return Value::new(Type::Unknown);
            }
            let target = self.infer(&xs[1], env);
            let default = xs.get(2).map(|x| self.infer(x, env));
            return self.lookup(&target, &self.model.key(key), default.as_ref(), xs[0].span);
        }
        if let Some(name) = xs[0].symbol()
            && !env.contains_key(name)
            && self.model.signature(name, self.globals).is_none()
        {
            let resolved = self.model.resolve(name);
            if let Some(threading) = Threading::from_core_symbol(&resolved) {
                return self.thread_form(xs, span, env, threading, &resolved);
            }
            match resolved.as_str() {
                "clojure.core/as->" => return self.named_thread(xs, span, env),
                "clojure.core/let" => return self.let_form(xs, span, env),
                "clojure.core/if" | "clojure.core/when" => {
                    return self.if_form(xs, span, env, resolved.ends_with("/when"));
                }
                "clojure.core/if-let"
                | "clojure.core/when-let"
                | "clojure.core/if-some"
                | "clojure.core/when-some" => {
                    return self.binding_conditional(xs, span, env, &resolved);
                }
                "clojure.core/do" => return self.body(&xs[1..], env),

                "clojure.core/or" | "clojure.core/and" => {
                    return self.short_circuit(xs, env, resolved.ends_with("/or"));
                }
                "clojure.core/get" | "clojure.core/req!" => {
                    let required = resolved == "clojure.core/req!";
                    let max_arity = if required { 3 } else { 4 };
                    if !(3..=max_arity).contains(&xs.len()) {
                        self.error(
                            "PNX2554",
                            span,
                            format!(
                                "Invalid arity for {resolved}: expected a collection and key{}",
                                if required {
                                    ""
                                } else {
                                    " and optional default"
                                }
                            ),
                        );
                        return Value::new(Type::Unknown);
                    }
                    let target = self.infer(&xs[1], env);
                    let key = self.infer(&xs[2], env);
                    let default = xs.get(3).map(|x| self.infer(x, env));
                    return if let Type::Key(key) = key.ty {
                        if required {
                            self.checked_lookup(&target, &key, Lookup::Required, xs[2].span)
                        } else {
                            self.lookup(&target, &key, default.as_ref(), xs[2].span)
                        }
                    } else {
                        self.unsupported(
                            xs[2].span,
                            format!("{resolved} currently requires a literal keyword"),
                        )
                    };
                }
                "clojure.core/assoc" => return self.assoc(xs, span, env),
                _ => {
                    if let Some(result) = self.builtin(&resolved, &xs[1..], span, env) {
                        return result;
                    }
                }
            }
        }
        let function = self.infer(&xs[0], env);
        let args = xs[1..]
            .iter()
            .map(|x| self.infer(x, env))
            .collect::<Vec<_>>();
        match function.known {
            Type::Function(expected, ret) => {
                if args.len() != expected.len() {
                    self.error(
                        "PNX2554",
                        span,
                        format!("Expected {} arguments, got {}", expected.len(), args.len()),
                    );
                }
                for (index, (actual, expected)) in args.iter().zip(&expected).enumerate() {
                    self.expect(
                        actual,
                        expected,
                        xs[index + 1].span,
                        &format!("Argument {}", index + 1),
                    );
                }
                Value::new(*ret)
            }
            Type::Set(_) => {
                if args.len() != 1 {
                    self.error(
                        "PNX2554",
                        span,
                        format!(
                            "A set used as a function expects 1 argument, got {}",
                            args.len()
                        ),
                    );
                    Value::new(Type::Unknown)
                } else {
                    Value::with_notes(args[0].known.clone().nilable(), vec![
                        "A set used as a function returns the matching value or Nil, not a Boolean.".into(),
                    ])
                }
            }
            Type::Unknown | Type::Never => Value::new(function.known),
            _ => self.unsupported(
                xs[0].span,
                format!("Calling {} is not supported", function.ty),
            ),
        }
    }

    fn thread_form(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        threading: Threading,
        name: &str,
    ) -> Value {
        if xs.len() < 2 {
            self.error(
                "PNX2554",
                span,
                format!("{name} requires an initial expression"),
            );
            return Value::new(Type::Unknown);
        }
        if threading.is_conditional() && !(xs.len() - 2).is_multiple_of(2) {
            self.error(
                "PNX2554",
                span,
                format!("{name} requires condition/step pairs"),
            );
            return Value::new(Type::Unknown);
        }
        self.with_thread_expansion(span, |checker| {
            if threading.is_conditional() {
                checker.conditional_steps(xs, env, threading, name, span)
            } else {
                checker.thread_steps(xs, env, threading, name, span)
            }
        })
    }

    fn with_thread_expansion(
        &mut self,
        span: Span,
        expand: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        if self.thread_depth >= 32 {
            return self.unsupported(span, "Threading expansion nesting limit (32) exceeded");
        }
        self.thread_depth += 1;
        let result = expand(self);
        self.thread_depth -= 1;
        result
    }

    fn infer_path(&mut self, form: &Form, env: &Env, reachable: bool) -> Value {
        self.check_path(reachable, |checker| checker.infer(form, env))
    }

    fn check_path(&mut self, reachable: bool, check: impl FnOnce(&mut Self) -> Value) -> Value {
        let start = self.diagnostics.len();
        let mut value = check(self);
        if !reachable {
            // Unexecuted forms must still resolve and pass compiler-level checks.
            let errors = self.diagnostics.split_off(start);
            self.diagnostics.extend(
                errors
                    .into_iter()
                    .filter(|d| !matches!(d.code.as_str(), "PNX2322" | "PNX2741")),
            );
            value.narrow(Type::Never);
        }
        value
    }

    fn expansion_origin(&mut self, start: usize, name: &str, span: Span, transformed: bool) {
        for diagnostic in &mut self.diagnostics[start..] {
            // Synthetic argument spans can point at bare steps. Replacing such a
            // fragment may change threading semantics; original as-> forms are editable.
            if transformed && !diagnostic.suggestions.is_empty() {
                diagnostic.suggestions.clear();
                diagnostic.help = Some("Handle the expected type explicitly in the threading expression before this call.".into());
            }
            diagnostic.notes.push(format!(
                "While expanding {name} at {}:{}",
                span.start.line, span.start.column
            ));
        }
    }

    fn thread_steps(
        &mut self,
        xs: &[Form],
        env: &Env,
        threading: Threading,
        name: &str,
        span: Span,
    ) -> Value {
        let mut current = self.infer(&xs[1], env);
        if xs.len() == 2 {
            return current;
        }
        let temporary = expand::temporary(&xs[1..], |name| env.contains_key(name));
        let mut local = env.clone();
        local.insert(temporary.clone(), Value::new(Type::Never));
        let mut origin = xs[1].span;
        let mut skipped_nil = false;
        for step in &xs[2..] {
            if threading.guards_nil() {
                skipped_nil |= nil_part(&current.known) == Type::Nil;
                current.narrow(current.known.without_nil());
            }
            let reachable = current.known != Type::Never;
            *local.get_mut(&temporary).unwrap() = current;
            let call = threading.apply(
                step,
                Form {
                    kind: Kind::Symbol(temporary.clone()),
                    span: origin,
                },
            );
            let start = self.diagnostics.len();
            current = self.infer_path(&call, &local, reachable);
            self.expansion_origin(start, name, span, true);
            origin = step.span;
        }
        if skipped_nil {
            let known = std::mem::replace(&mut current.known, Type::Never);
            current.narrow(known.nilable());
            current.notes.push(format!("{name} returns Nil when its input or an intermediate result is Nil; False continues through the chain."));
        }
        current
    }

    fn conditional_steps(
        &mut self,
        xs: &[Form],
        env: &Env,
        threading: Threading,
        name: &str,
        span: Span,
    ) -> Value {
        let mut current = self.infer(&xs[1], env);
        if xs.len() == 2 {
            return current;
        }
        let temporary = expand::temporary(&xs[1..], |name| env.contains_key(name));
        let mut origin = xs[1].span;
        let mut aliases_seed = true;
        let mut joined_branches = false;
        for [test, step] in xs[2..].as_chunks::<2>().0 {
            let start = self.diagnostics.len();
            let condition = self.infer_path(test, env, current.known != Type::Never);
            self.expansion_origin(start, name, span, false);
            let yes_reachable = condition.known.truthy() != Type::Never;
            let no_reachable = condition.known.falsey() != Type::Never;
            joined_branches |= yes_reachable && no_reachable;
            let mut yes_env = env.clone();
            self.narrow(test, true, &mut yes_env);
            let mut skipped = if no_reachable {
                current.clone()
            } else {
                Value::new(Type::Never)
            };
            // Before a potentially executed transformation, the temporary still
            // denotes the original seed local, so its guard also narrows the input.
            if aliases_seed && let Some(seed) = xs[1].symbol() {
                if let Some(value) = yes_env.get(seed) {
                    current = value.clone();
                }
                if no_reachable {
                    let mut no_env = env.clone();
                    self.narrow(test, false, &mut no_env);
                    if let Some(value) = no_env.get(seed) {
                        skipped = value.clone();
                    }
                }
            }
            yes_env.insert(temporary.clone(), current);
            let call = threading.apply(
                step,
                Form {
                    kind: Kind::Symbol(temporary.clone()),
                    span: origin,
                },
            );
            let start = self.diagnostics.len();
            let applied = self.infer_path(&call, &yes_env, yes_reachable);
            self.expansion_origin(start, name, span, true);
            current = applied.join(skipped);
            if yes_reachable {
                aliases_seed = false;
                origin = step.span;
            }
        }
        if joined_branches {
            current.notes.push(format!("{name} keeps the previous value when a condition is false; its result includes both paths."));
        }
        current
    }

    fn named_thread(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if xs.len() < 3 {
            self.error(
                "PNX2554",
                span,
                "clojure.core/as-> requires an expression and binding name",
            );
            return Value::new(Type::Unknown);
        }
        let Some(binding) = xs[2].symbol() else {
            if matches!(
                xs[2].without_metadata().kind,
                Kind::Map(_) | Kind::Vector(_)
            ) {
                return self.unsupported(
                    xs[2].span,
                    "as-> destructuring bindings are not supported yet",
                );
            }
            self.error("PNX1001", xs[2].span, "as-> requires a local binding name");
            return Value::new(Type::Unknown);
        };
        if binding == "&" || binding.contains('/') {
            self.error(
                "PNX1001",
                xs[2].span,
                "as-> requires an unqualified local binding name",
            );
            return Value::new(Type::Unknown);
        }
        self.with_thread_expansion(span, |checker| {
            let mut current = checker.infer(&xs[1], env);
            if xs.len() == 3 {
                return current;
            }
            let mut local = env.clone();
            local.insert(binding.to_owned(), Value::new(Type::Never));
            for step in &xs[3..] {
                let reachable = current.known != Type::Never;
                *local.get_mut(binding).unwrap() = current;
                let start = checker.diagnostics.len();
                current = checker.infer_path(step, &local, reachable);
                checker.expansion_origin(start, "clojure.core/as->", span, false);
            }
            current
        })
    }

    fn let_form(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        let Some(bindings) = xs.get(1).and_then(Form::vector) else {
            return self.unsupported(span, "let requires a binding vector");
        };
        if bindings.len() % 2 != 0 {
            return self.unsupported(xs[1].span, "let requires binding/value pairs");
        }
        let mut local = env.clone();
        for pair in bindings.as_chunks::<2>().0 {
            let value = self.infer(&pair[1], &local);
            self.bind(&pair[0], value, &mut local);
        }
        self.body(&xs[2..], &local)
    }

    fn if_form(&mut self, xs: &[Form], span: Span, env: &Env, when: bool) -> Value {
        if xs.len() < 3 || (!when && xs.len() > 4) {
            return self.unsupported(span, "Invalid conditional arity");
        }
        let condition = self.infer(&xs[1], env);
        let mut yes = {
            let mut local = env.clone();
            self.narrow(&xs[1], true, &mut local);
            if when {
                self.body(&xs[2..], &local)
            } else {
                self.infer(&xs[2], &local)
            }
        };
        let mut no = {
            let mut local = env.clone();
            self.narrow(&xs[1], false, &mut local);
            if when {
                Value::new(Type::Nil)
            } else {
                xs.get(3)
                    .map_or_else(|| Value::new(Type::Nil), |x| self.infer(x, &local))
            }
        };
        // Even unreachable forms must be resolved: Clojure compiles both branches.
        if condition.ty.truthy() == Type::Never {
            yes.narrow(Type::Never);
        }
        if condition.ty.falsey() == Type::Never {
            no.narrow(Type::Never);
        }
        Value::with_notes(
            Type::union([yes.known, no.known]),
            [yes.notes, no.notes].concat(),
        )
    }

    fn binding_conditional(&mut self, xs: &[Form], span: Span, env: &Env, name: &str) -> Value {
        let when = name.starts_with("clojure.core/when-");
        if xs.len() < 2 || (!when && !(3..=4).contains(&xs.len())) {
            self.error("PNX2554", span, format!("Invalid arity for {name}"));
            return Value::new(Type::Unknown);
        }
        let Some([pattern, init]) = xs[1].vector() else {
            self.error(
                "PNX1001",
                xs[1].span,
                format!("{name} requires a binding vector with exactly two forms"),
            );
            return Value::new(Type::Unknown);
        };
        let valid_pattern = match &pattern.without_metadata().kind {
            Kind::Symbol(symbol) => symbol != "&" && !symbol.contains('/'),
            Kind::Map(_) | Kind::Vector(_) => true,
            _ => false,
        };
        if !valid_pattern {
            self.error(
                "PNX1001",
                pattern.span,
                format!("Invalid binding form for {name}"),
            );
            return Value::new(Type::Unknown);
        }

        let mut input = self.infer(init, env);
        let non_nil = name.ends_with("-some");
        let (matched, skipped) = if non_nil {
            (input.known.without_nil(), nil_part(&input.known))
        } else {
            (input.known.truthy(), input.known.falsey())
        };
        let yes_reachable = matched != Type::Never;
        let no_reachable = skipped != Type::Never;
        let narrow = |truth, ty, scope: &mut Env| {
            if non_nil {
                if let Some(value) = init.symbol().and_then(|symbol| scope.get_mut(symbol)) {
                    value.narrow(ty);
                }
            } else {
                self.narrow(init, truth, scope);
            }
        };
        let mut yes_env = env.clone();
        narrow(true, matched.clone(), &mut yes_env);
        let no_env = if !when && xs.len() == 4 {
            let mut scope = env.clone();
            narrow(false, skipped, &mut scope);
            Some(scope)
        } else {
            None
        };

        input.narrow(matched);
        let yes = self.check_path(yes_reachable, |checker| {
            checker.bind(pattern, input, &mut yes_env);
            checker.body(if when { &xs[2..] } else { &xs[2..3] }, &yes_env)
        });
        let no = if let Some(scope) = no_env {
            self.infer_path(&xs[3], &scope, no_reachable)
        } else if no_reachable {
            Value::new(Type::Nil)
        } else {
            Value::new(Type::Never)
        };
        Value::with_notes(
            Type::union([yes.known, no.known]),
            [yes.notes, no.notes].concat(),
        )
    }

    fn short_circuit(&mut self, xs: &[Form], env: &Env, or: bool) -> Value {
        if xs.len() == 1 {
            return Value::new(if or { Type::Nil } else { Type::Bool(true) });
        }
        let mut local = env.clone();
        let mut types = Vec::new();
        let mut notes = Vec::new();
        let mut reachable = true;
        for (index, x) in xs[1..].iter().enumerate() {
            let value = self.infer(x, &local);
            if !reachable {
                continue;
            }
            notes.extend(value.notes);
            if index == xs.len() - 2 {
                types.push(value.known);
                break;
            }
            types.push(if or {
                value.known.truthy()
            } else {
                value.known.falsey()
            });
            if (if or {
                value.ty.falsey()
            } else {
                value.ty.truthy()
            }) == Type::Never
            {
                reachable = false;
            }
            self.narrow(x, !or, &mut local);
        }
        Value::with_notes(Type::union(types), notes)
    }

    fn narrow(&self, test: &Form, truth: bool, env: &mut Env) {
        if let Some(name) = test.symbol() {
            if let Some(value) = env.get_mut(name) {
                value.narrow(if truth {
                    value.known.truthy()
                } else {
                    value.known.falsey()
                });
            }
            return;
        }
        let Kind::List(xs) = &test.without_metadata().kind else {
            return;
        };
        if xs.len() != 2 {
            return;
        }
        let Some(op) = xs[0].symbol() else { return };
        // Locals and project functions may shadow core predicates.
        if env.contains_key(op)
            || self.model.local_function(op).is_some()
            || self.model.signature(op, self.globals).is_some()
        {
            return;
        }
        let op = self.model.resolve(op);
        if op == "clojure.core/not" {
            self.narrow(&xs[1], !truth, env);
            return;
        }
        let Some(name) = xs[1].symbol() else { return };
        let Some(value) = env.get_mut(name) else {
            return;
        };
        let positive = match op.as_str() {
            "clojure.core/nil?" => Some(Type::Nil),
            "clojure.core/string?" => Some(Type::String),
            "clojure.core/char?" => Some(Type::Character),
            "clojure.core/number?" => Some(Type::Number),
            "clojure.core/boolean?" => Some(Type::Boolean),
            "clojure.core/keyword?" => Some(Type::Keyword),
            "clojure.core/some?" => {
                value.narrow(if truth {
                    value.known.without_nil()
                } else {
                    nil_part(&value.known)
                });
                return;
            }
            _ => None,
        };
        if let Some(wanted) = positive {
            let parts = match &value.known {
                Type::Union(xs) => xs.clone(),
                t => vec![t.clone()],
            };
            value.narrow(Type::union(parts.into_iter().filter_map(|t| {
                if t.subtype_of(&wanted) {
                    truth.then_some(t)
                } else if wanted.subtype_of(&t) {
                    Some(if truth { wanted.clone() } else { t })
                } else {
                    (!truth).then_some(t)
                }
            })));
        }
    }

    fn lookup(&mut self, value: &Value, key: &str, default: Option<&Value>, span: Span) -> Value {
        self.lookup_value(value, key, Lookup::Optional(default), span)
    }

    fn checked_lookup(&mut self, value: &Value, key: &str, mode: Lookup<'_>, span: Span) -> Value {
        let mut result = self.lookup_value(value, key, mode, span);
        if !result.known.contains_unknown() && nil_part(&result.known) == Type::Nil {
            result.notes.push("A checked lookup requires the key to exist; an explicitly present Nil remains Nil.".into());
        }
        if result.known == Type::Never && value.known != Type::Never {
            let mut diagnostic = Diagnostic::new(
                "PNX2741",
                span,
                format!("Required key {key} is absent; checked lookup would throw"),
            );
            diagnostic.notes = value.notes.clone();
            diagnostic.help = Some(format!(
                "Provide {key} in the input collection. An explicitly present Nil still satisfies the presence check."
            ));
            self.diagnostics.push(diagnostic);
        }
        result
    }

    fn lookup_value(&mut self, value: &Value, key: &str, mode: Lookup<'_>, span: Span) -> Value {
        let default = match mode {
            Lookup::Optional(default) => default,
            _ => None,
        };

        let default_type = match mode {
            Lookup::Optional(default) => default.map_or(Type::Nil, |v| v.known.clone()),
            Lookup::Required | Lookup::Presence => Type::Never,
        };
        let mut notes = value.notes.clone();
        let ty = match &value.known {
            Type::Nil => default_type,
            Type::Never => Type::Never,
            Type::Unknown => Type::Unknown,
            Type::Union(xs) => {
                let values = xs
                    .iter()
                    .map(|t| self.lookup_value(&Value::new(t.clone()), key, mode, span))
                    .collect::<Vec<_>>();
                notes.extend(values.iter().flat_map(|v| v.notes.clone()));
                Type::union(values.into_iter().map(|v| v.known))
            }
            Type::Set(element) => {
                let key_type = Type::Key(key.into());
                if matches!(mode, Lookup::Optional(_)) {
                    Type::union([key_type, default_type])
                } else if key_type.subtype_of(element) || element.contains_unknown() {
                    key_type
                } else {
                    Type::Never
                }
            }
            Type::Map(map) => match map.fields.get(key) {
                Some(field) if field.required || !matches!(mode, Lookup::Optional(_)) => {
                    field.ty.clone()
                }
                Some(field) => {
                    notes.push(format!(
                        "Field {key} is optional; a missing key returns {}",
                        default_type
                    ));
                    Type::union([field.ty.clone(), default_type])
                }
                None if !map.open => default_type,
                None if matches!(mode, Lookup::Presence) => Type::Unknown,
                None => {
                    self.error(
                        "PNX9003",
                        span,
                        format!("No declared type for {key} in this open map"),
                    );
                    Type::Unknown
                }
            },
            _ => {
                self.error(
                    "PNX2322",
                    span,
                    format!("Keyword lookup expects a map, set or Nil, got {}", value.ty),
                );
                Type::Unknown
            }
        };
        if let Some(default) = default {
            notes.extend(default.notes.clone());
        }
        Value::with_notes(ty, notes)
    }

    fn assoc(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if xs.len() < 4 || !xs.len().is_multiple_of(2) {
            self.error("PNX2554", span, "assoc requires a map and key/value pairs");
            return Value::new(Type::Unknown);
        }
        let target = self.infer(&xs[1], env);
        let mut updates = Vec::new();
        for pair in xs[2..].as_chunks::<2>().0 {
            let key = self.infer(&pair[0], env);
            let value = self.infer(&pair[1], env);
            let Type::Key(key) = key.ty else {
                return self.unsupported(
                    pair[0].span,
                    "assoc currently requires literal keyword keys",
                );
            };
            updates.push((key, value.known));
        }
        let parts = match target.known {
            Type::Union(xs) => xs,
            t => vec![t],
        };
        let mut results = Vec::new();
        for part in parts {
            let mut map = match part {
                Type::Map(map) => map,
                Type::Nil => MapType {
                    fields: BTreeMap::new(),
                    open: false,
                },
                Type::Unknown | Type::Never => {
                    results.push(part);
                    continue;
                }
                _ => {
                    self.error(
                        "PNX2322",
                        xs[1].span,
                        format!("assoc expects a map or Nil, got {part}"),
                    );
                    results.push(Type::Unknown);
                    continue;
                }
            };
            for (key, ty) in &updates {
                map.fields.insert(
                    key.clone(),
                    Field {
                        ty: ty.clone(),
                        required: true,
                    },
                );
            }
            results.push(Type::Map(map));
        }
        Value::new(Type::union(results))
    }

    fn builtin(&mut self, name: &str, args: &[Form], span: Span, env: &Env) -> Option<Value> {
        let recognized = matches!(
            name,
            "clojure.core/+"
                | "clojure.core/-"
                | "clojure.core/*"
                | "clojure.core/inc"
                | "clojure.core/dec"
                | "clojure.core/str"
                | "clojure.core/identity"
                | "clojure.core/nil?"
                | "clojure.core/some?"
                | "clojure.core/string?"
                | "clojure.core/char?"
                | "clojure.core/number?"
                | "clojure.core/boolean?"
                | "clojure.core/keyword?"
                | "clojure.core/not"
                | "clojure.core/="
                | "clojure.core/count"
                | "clojure.string/upper-case"
                | "clojure.string/lower-case"
                | "clojure.string/trim"
        );
        if !recognized {
            return None;
        }
        let values = args.iter().map(|a| self.infer(a, env)).collect::<Vec<_>>();
        let arity_ok = match name {
            "clojure.core/+" | "clojure.core/*" | "clojure.core/str" => true,
            "clojure.core/-" | "clojure.core/=" => !args.is_empty(),
            _ => args.len() == 1,
        };
        if !arity_ok {
            self.error(
                "PNX2554",
                span,
                format!("Invalid arity for {name}: {}", args.len()),
            );
            return Some(Value::new(Type::Unknown));
        }
        let ty = match name {
            "clojure.core/+" | "clojure.core/-" | "clojure.core/*" | "clojure.core/inc"
            | "clojure.core/dec" => {
                for (arg, value) in args.iter().zip(&values) {
                    self.expect(value, &Type::Number, arg.span, name);
                }
                Type::Number
            }
            "clojure.string/upper-case" | "clojure.string/lower-case" | "clojure.string/trim" => {
                self.expect(&values[0], &Type::String, args[0].span, name);
                Type::String
            }
            "clojure.core/str" => Type::String,
            "clojure.core/identity" => return Some(values[0].clone()),
            "clojure.core/count" => {
                fn countable(t: &Type) -> bool {
                    match t {
                        Type::Nil
                        | Type::String
                        | Type::Str(_)
                        | Type::Map(_)
                        | Type::Vector(_)
                        | Type::Set(_)
                        | Type::Never
                        | Type::Unknown => true,
                        Type::Union(xs) => xs.iter().all(countable),
                        _ => false,
                    }
                }
                if !countable(&values[0].ty) {
                    self.error(
                        "PNX2322",
                        args[0].span,
                        format!(
                            "count expects a collection, string or Nil, got {}",
                            values[0].ty
                        ),
                    );
                }
                Type::Number
            }
            "clojure.core/string?" => predicate_result(&values[0].known, &Type::String),
            "clojure.core/char?" => predicate_result(&values[0].known, &Type::Character),
            "clojure.core/number?" => predicate_result(&values[0].known, &Type::Number),
            "clojure.core/boolean?" => predicate_result(&values[0].known, &Type::Boolean),
            "clojure.core/keyword?" => predicate_result(&values[0].known, &Type::Keyword),
            "clojure.core/nil?" => {
                if values[0].ty == Type::Nil {
                    Type::Bool(true)
                } else if nil_part(&values[0].ty) == Type::Never {
                    Type::Bool(false)
                } else {
                    Type::Boolean
                }
            }
            "clojure.core/some?" => {
                if values[0].ty == Type::Nil {
                    Type::Bool(false)
                } else if nil_part(&values[0].ty) == Type::Never {
                    Type::Bool(true)
                } else {
                    Type::Boolean
                }
            }
            "clojure.core/not" => {
                if values[0].ty.truthy() == Type::Never {
                    Type::Bool(true)
                } else if values[0].ty.falsey() == Type::Never {
                    Type::Bool(false)
                } else {
                    Type::Boolean
                }
            }
            _ => Type::Boolean,
        };
        Some(Value::new(ty))
    }
}

fn nil_part(ty: &Type) -> Type {
    match ty {
        Type::Nil | Type::Unknown => Type::Nil,
        Type::Union(xs) if xs.iter().any(|t| nil_part(t) == Type::Nil) => Type::Nil,
        _ => Type::Never,
    }
}

fn predicate_result(actual: &Type, expected: &Type) -> Type {
    match actual {
        Type::Unknown => Type::Boolean,
        Type::Never => Type::Never,
        Type::Union(parts) => {
            Type::union(parts.iter().map(|part| predicate_result(part, expected)))
        }
        actual => Type::Bool(actual.subtype_of(expected)),
    }
}
