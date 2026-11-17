//! Source declarations and the namespace context used to read them.
use std::collections::{BTreeMap, BTreeSet};

use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Names {
    pub(super) namespace: String,
    pub(super) refers: BTreeMap<String, String>,
    pub(super) aliases: BTreeMap<String, String>,
}

impl Names {
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
    pub dependencies: Vec<String>,
    pub(super) specs: BTreeMap<String, SpecDeclaration>,
    pub(super) functions: Vec<usize>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl Declarations {
    pub fn namespace(&self) -> &str {
        &self.names.namespace
    }

    pub(super) fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    pub fn read(forms: &[Form]) -> Self {
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
                model.read_ns(xs, form.span);
            }
        }
        let mut dependencies: BTreeSet<_> = model.dependencies.iter().cloned().collect();
        fn visit(form: &Form, names: &Names, dependencies: &mut BTreeSet<String>) {
            match &form.kind {
                Kind::Symbol(name) => {
                    let resolved = names.resolve(name);
                    if let Some((namespace, _)) = resolved.rsplit_once('/') {
                        dependencies.insert(namespace.to_string());
                    }
                }
                Kind::List(xs) | Kind::Vector(xs) | Kind::Map(xs) | Kind::Set(xs) => {
                    for x in xs {
                        visit(x, names, dependencies);
                    }
                }
                Kind::Quote(x) => visit(x, names, dependencies),
                Kind::Metadata(metadata, target) => {
                    visit(metadata, names, dependencies);
                    visit(target, names, dependencies);
                }
                _ => {}
            }
        }
        for form in forms {
            visit(form, &model.names, &mut dependencies);
        }
        dependencies.remove(&model.names.namespace);
        model.dependencies = dependencies.into_iter().collect();
        for (index, form) in forms.iter().enumerate() {
            let Kind::List(xs) = &form.without_metadata().kind else {
                model.error("PNX9001", form.span, "Unsupported top-level form");
                continue;
            };
            let Some(op) = xs.first().and_then(Form::symbol) else {
                model.error("PNX9001", form.span, "Expected a top-level declaration");
                continue;
            };
            match model.names.resolve(op).as_str() {
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
                _ => model.error(
                    "PNX9001",
                    form.span,
                    format!("Unsupported top-level declaration {op}"),
                ),
            }
        }
        model
    }

    fn read_ns(&mut self, xs: &[Form], span: Span) {
        let Some(name) = xs.get(1).and_then(Form::symbol) else {
            self.error("PNX9001", span, "ns requires a namespace name");
            return;
        };
        self.names.namespace = name.to_string();
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
}
