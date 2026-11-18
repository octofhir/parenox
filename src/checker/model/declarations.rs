//! Source declarations and the namespace context used to read them.
use std::collections::{BTreeMap, BTreeSet};

use super::Globals;
use super::contracts::function_parts;
use crate::Diagnostic;
use crate::reader::{Form, Kind, ReadError, Span};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Names {
    pub(super) namespace: String,
    pub(super) refers: BTreeMap<String, String>,
    pub(super) aliases: BTreeMap<String, String>,
    pub(super) imports: BTreeMap<String, String>,
    pub(super) core_only: Option<BTreeSet<String>>,
    pub(super) core_hidden: BTreeSet<String>,
}

impl Names {
    pub(super) fn resolve(&self, name: &str) -> String {
        let (namespace, local) = self.resolve_parts(name);
        format!("{namespace}/{local}")
    }

    pub(super) fn resolve_parts<'a>(&'a self, name: &'a str) -> (&'a str, &'a str) {
        if let Some((prefix, local)) = name
            .split_once('/')
            .filter(|(prefix, _)| !prefix.is_empty())
        {
            (
                self.aliases.get(prefix).map_or(prefix, String::as_str),
                local,
            )
        } else if let Some(referred) = self.refers.get(name) {
            referred.split_once('/').unwrap()
        } else if self.core_visible(name) {
            ("clojure.core", name)
        } else {
            (&self.namespace, name)
        }
    }

    pub(super) fn core_visible(&self, name: &str) -> bool {
        self.core_only
            .as_ref()
            .is_none_or(|only| only.contains(name))
            && !self.core_hidden.contains(name)
    }

    pub(super) fn key(&self, key: &str) -> String {
        if let Some(tail) = key.strip_prefix("::") {
            if let Some((alias, name)) = tail.split_once('/') {
                format!(
                    ":{}/{name}",
                    self.aliases.get(alias).map_or(alias, String::as_str)
                )
            } else {
                format!(":{}/{tail}", self.namespace)
            }
        } else {
            key.to_string()
        }
    }
}

pub(super) struct SpecDeclaration {
    pub name: Span,
    pub body: Form,
}

#[derive(Default)]
pub(crate) struct Declarations {
    pub(super) names: Names,
    namespace_end: Option<usize>,
    pub dependencies: Vec<String>,
    pub(super) all_refers: BTreeSet<String>,
    pub(super) specs: BTreeMap<String, SpecDeclaration>,
    pub(super) fdefs: Vec<usize>,
    pub(super) functions: Vec<usize>,
    pub(super) literal_vars: Vec<usize>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl Declarations {
    pub fn namespace(&self) -> &str {
        &self.names.namespace
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn qualified_reference<'a>(&'a self, symbol: &'a str) -> Option<(&'a str, &'a str)> {
        let (prefix, local) = symbol.split_once('/')?;
        if prefix.is_empty() || local.is_empty() || local.contains('/') {
            return None;
        }
        Some(self.names.resolve_parts(symbol))
    }

    pub(crate) fn source_var(
        &self,
        forms: &[Form],
        local: &str,
        public_only: bool,
    ) -> Option<Span> {
        let mut matches = self
            .functions
            .iter()
            .chain(&self.literal_vars)
            .filter_map(|&index| {
                let Kind::List(items) = &forms[index].without_metadata().kind else {
                    return None;
                };
                let name = items.get(1)?;
                (name.symbol() == Some(local)).then_some(items.as_slice())
            });
        let items = matches.next()?;
        if matches.next().is_some() || (public_only && self.private_var(items)) {
            return None;
        }
        Some(items[1].without_metadata().span)
    }

    fn private_var(&self, items: &[Form]) -> bool {
        let private =
            |form: &Form| !matches!(form.without_metadata().kind, Kind::Nil | Kind::Bool(false));
        let op = items[0].symbol().unwrap();
        if op == "def" {
            return items[1].metadata(":private").is_some_and(private);
        }
        let mut attrs = function_parts(&items[2..]).1;
        if let Some([_, value]) = attrs.find(|[key, _]| key.keyword() == Some(":private")) {
            return private(value);
        }
        self.names.resolve(op) == "clojure.core/defn-"
            || items[1].metadata(":private").is_some_and(private)
    }

    pub(super) fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    pub fn read(forms: &[Form]) -> Self {
        Self::read_with_imports(forms, None)
    }

    pub(super) fn read_with_imports(forms: &[Form], globals: Option<&Globals>) -> Self {
        let mut model = Self {
            names: Names {
                namespace: "user".into(),
                ..Names::default()
            },
            ..Self::default()
        };
        let mut seen_ns = false;
        for form in forms {
            if let Kind::List(xs) = &form.without_metadata().kind
                && matches!(
                    xs.first().and_then(Form::symbol),
                    Some("ns" | "clojure.core/ns")
                )
            {
                if seen_ns {
                    model.error("PNX9001", form.span, "Only one ns declaration is supported");
                }
                seen_ns = true;
                model.namespace_end = Some(form.span.end.offset);
                model.read_ns(xs, form.span, globals);
            }
        }
        let mut dependencies: BTreeSet<_> = model.dependencies.iter().map(String::as_str).collect();
        for form in forms {
            symbols_match(form, &mut |name| {
                dependencies.insert(model.names.resolve_parts(name).0);
                true
            });
        }
        dependencies.remove(model.names.namespace.as_str());
        model.dependencies = dependencies.into_iter().map(str::to_owned).collect();
        for (index, form) in forms.iter().enumerate() {
            let Kind::List(xs) = &form.without_metadata().kind else {
                model.error("PNX9001", form.span, "Unsupported top-level form");
                continue;
            };
            let Some(op) = xs.first().and_then(Form::symbol) else {
                model.error("PNX9001", form.span, "Expected a top-level declaration");
                continue;
            };
            match op {
                "ns" | "clojure.core/ns" => continue,
                "def" => {
                    model.literal_vars.push(index);
                    continue;
                }
                _ => {}
            }
            let resolved = model.names.resolve(op);
            match resolved.as_str() {
                "clojure.core/ns" => {}
                "clojure.spec.alpha/def" if xs.len() == 3 => {
                    if let Some(key) = xs[1].keyword() {
                        let key = model.names.key(key);
                        let declaration = SpecDeclaration {
                            name: xs[1].span,
                            body: xs[2].clone(),
                        };
                        if model.specs.insert(key.clone(), declaration).is_some() {
                            model.error("PNX2790", xs[1].span, format!("Duplicate spec {key}"));
                        }
                    } else {
                        model.error("PNX9001", xs[1].span, "Spec name must be a keyword");
                    }
                }
                "clojure.core/defn" | "clojure.core/defn-" => model.functions.push(index),
                "clojure.spec.alpha/fdef" => model.fdefs.push(index),
                qualified if known_unsupported_declaration(qualified, xs) => {
                    model.diagnostics.push(Diagnostic::analysis_gap(
                        "PNX9001",
                        xs[0].span,
                        format!("Analysis for {op} declarations is unavailable"),
                    ));
                }
                _ => model.error(
                    "PNX9001",
                    form.span,
                    format!("Unsupported top-level declaration {op}"),
                ),
            }
        }
        model
    }

    pub fn validate_keyword_aliases(&self, source: &str, spans: &[Span]) -> Result<(), ReadError> {
        for &span in spans {
            let token = &source[span.start.offset..span.end.offset];
            if let Some((alias, _)) = token[2..].split_once('/')
                && (self.namespace_end.is_none_or(|end| span.start.offset < end)
                    || !self.names.aliases.contains_key(alias))
            {
                return Err(ReadError {
                    span,
                    message: format!(
                        "No namespace alias {alias} for auto-resolved keyword {token}"
                    ),
                });
            }
        }
        Ok(())
    }

    fn read_ns(&mut self, xs: &[Form], span: Span, globals: Option<&Globals>) {
        let Some(name) = xs.get(1).and_then(Form::symbol) else {
            self.error("PNX9001", span, "ns requires a namespace name");
            return;
        };
        self.names.namespace = name.to_string();
        let mut has_core_clause = false;
        for clause in xs.iter().skip(2) {
            // ns docstrings and attr-maps are declarations, not executable forms.
            if matches!(clause.kind, Kind::String(_) | Kind::Map(_)) {
                continue;
            }
            let Kind::List(items) = &clause.without_metadata().kind else {
                self.error("PNX9001", clause.span, "Unsupported ns clause");
                continue;
            };
            match items.first().and_then(Form::keyword) {
                Some(":require") => {
                    for lib in &items[1..] {
                        self.read_libspec(lib, globals);
                    }
                }
                Some(":import") => {
                    for form in &items[1..] {
                        self.read_import(form);
                    }
                }
                Some(":refer-clojure") if !has_core_clause => {
                    has_core_clause = true;
                    self.read_core_refers(&items[1..], clause.span);
                }
                Some(":refer-clojure") => self.error(
                    "PNX9001",
                    clause.span,
                    "Only one :refer-clojure clause is supported",
                ),
                _ => self.error("PNX9001", clause.span, "Unsupported ns clause"),
            }
        }
    }
}

fn known_unsupported_declaration(qualified: &str, items: &[Form]) -> bool {
    if items.get(1).and_then(Form::symbol).is_none() {
        return false;
    }
    match qualified {
        "clojure.core/defrecord" => items.get(2).and_then(Form::vector).is_some(),
        "clojure.core/defmethod" => items.len() >= 4,
        "clojure.test/deftest" => true,
        "clojure.test.check.clojure-test/defspec" => items.len() >= 3,
        _ => false,
    }
}

pub(super) fn symbols_match<'a>(form: &'a Form, test: &mut impl FnMut(&'a str) -> bool) -> bool {
    match &form.kind {
        Kind::Symbol(name) => test(name),
        Kind::List(forms) if forms.first().and_then(Form::symbol) == Some("quote") => true,
        Kind::List(forms) | Kind::Vector(forms) | Kind::Map(forms) | Kind::Set(forms) => {
            forms.iter().all(|form| symbols_match(form, test))
        }
        Kind::AnonymousFunction(function) => {
            function.body.iter().all(|form| symbols_match(form, test))
        }
        Kind::Metadata(metadata, target) => {
            symbols_match(metadata, test) && symbols_match(target, test)
        }
        _ => true,
    }
}
