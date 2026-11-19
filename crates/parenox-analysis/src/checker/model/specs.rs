//! Project-wide spec declarations, resolution and cache validity.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::contracts::function_parts;
use super::declarations::{Declarations, Names, SpecDeclaration, symbols_match};
use super::type_forms::{core_predicate_type, enum_conversion, read_type, source_conversion};
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::{IntegerType, Type};

#[derive(PartialEq, Eq)]
pub(crate) struct SpecFingerprint {
    names: Names,
    bodies: Vec<(String, Span, Span, String)>,
    duplicates: BTreeSet<String>,
    enums: BTreeMap<String, SpecConversion>,
}

impl Declarations {
    pub(super) fn has_identity_spec(&self, forms: &[Form]) -> bool {
        let uses_identity = |form: &Form| {
            !symbols_match(form, &mut |name| {
                self.names.resolve_parts(name) != ("clojure.core", "identity")
            })
        };
        self.specs.values().any(|spec| uses_identity(&spec.body))
            || self.fdefs.iter().any(|&index| uses_identity(&forms[index]))
    }

    pub(super) fn has_inline_spec_composition(&self, forms: &[Form]) -> bool {
        self.functions.iter().any(|&index| {
            let Kind::List(items) = &forms[index].without_metadata().kind else {
                return false;
            };
            let [_, name, tail @ ..] = items.as_slice() else {
                return false;
            };
            // Dependency scanning skips quoted data; :t/sig uses quoted types.
            let signature = function_parts(tail)
                .1
                .find_map(|[key, value]| (key.keyword() == Some(":t/sig")).then_some(value))
                .or_else(|| name.metadata(":t/sig"));
            signature.is_some_and(|form| {
                !symbols_match(form.unquoted(), &mut |symbol| {
                    !matches!(
                        self.names.resolve_parts(symbol),
                        ("clojure.spec.alpha", "and" | "conformer" | "coll-of")
                            | ("clojure.core", "integer?" | "int?" | "identity")
                    )
                })
            })
        })
    }

    pub(super) fn read_spec_core_scope(&mut self, forms: &[Form]) {
        self.names.spec_core_shadows = self
            .top_level_names(forms)
            .into_iter()
            .chain(self.forward_vars.keys().map(String::as_str))
            .filter(|name| {
                let (namespace, local) = self.names.resolve_parts(name);
                ((namespace == "clojure.core"
                    && (matches!(local, "second" | "int") || core_predicate_type(local).is_some()))
                    || (namespace == "clojure.spec.alpha" && matches!(local, "and" | "conformer")))
                    && (self.forward_vars.contains_key(*name)
                        || self.source_declarations(forms, name).next().is_some())
            })
            .map(str::to_owned)
            .collect();
        self.names.spec_core_scope_known =
            self.all_refers.is_empty() && self.has_known_var_surface(forms);
    }

    pub fn spec_fingerprint(&self, source: &str, forms: &[Form]) -> Option<SpecFingerprint> {
        if self.specs.is_empty() {
            return None;
        }
        let bodies = self
            .specs
            .iter()
            .map(|(key, declaration)| {
                let span = declaration.body.span;
                let text = &source[span.start.offset..span.end.offset];
                (key.clone(), declaration.name, span, text.to_owned())
            })
            .collect();
        Some(SpecFingerprint {
            names: self.names.clone(),
            bodies,
            duplicates: self.duplicate_specs.clone(),
            enums: self
                .specs
                .iter()
                .filter_map(|(key, declaration)| {
                    enum_conversion(self, forms, source, &declaration.body)
                        .map(|conversion| (key.clone(), conversion))
                })
                .collect(),
        })
    }
}

pub(crate) struct SpecSource<'a> {
    pub path: &'a str,
    pub declarations: &'a Declarations,
    pub fingerprint: &'a Arc<SpecFingerprint>,
}

/// An exact built-in conversion, separate from a spec's static input shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SpecConversion {
    CheckedInt32,
    Identity,
    IntEnum(Arc<[(i32, String)]>),
}

impl SpecConversion {
    /// The original input domain of a checked contract (successful validation).
    pub fn success_input(&self) -> Type {
        match self {
            Self::CheckedInt32 => Type::Integral(IntegerType::INT32),
            Self::Identity => Type::SpecTruthy,
            Self::IntEnum(entries) => {
                Type::union(entries.iter().map(|(key, _)| Type::Integer(*key)))
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct SpecCache {
    previous: Vec<(String, Arc<SpecFingerprint>)>,
    pub types: Arc<BTreeMap<String, Type>>,
    pub conversions: Arc<BTreeMap<String, SpecConversion>>,
    pub diagnostics: BTreeMap<String, Vec<Diagnostic>>,
}

impl SpecCache {
    pub fn update(&mut self, mut sources: Vec<SpecSource<'_>>) {
        sources.sort_by_key(|source| source.path);
        if sources.len() == self.previous.len()
            && sources
                .iter()
                .zip(&self.previous)
                .all(|(source, (path, fingerprint))| {
                    source.path == path && source.fingerprint == fingerprint
                })
        {
            return;
        }

        let mut resolver = Resolver::default();
        let mut duplicate = BTreeSet::new();
        for source in &sources {
            for (key, declaration) in &source.declarations.specs {
                let owner = Owner {
                    path: source.path,
                    names: &source.declarations.names,
                    declaration,
                    enum_conversion: source.fingerprint.enums.get(key),
                };
                if let Some(previous) = resolver.declarations.get(key.as_str()).copied() {
                    if duplicate.insert(key.as_str()) {
                        resolver.duplicate(key, previous);
                    }
                    resolver.duplicate(key, owner);
                } else {
                    resolver.declarations.insert(key, owner);
                }
                if source.declarations.duplicate_specs.contains(key) {
                    resolver.types.insert(key.to_owned(), Type::Unknown);
                }
            }
        }
        for key in duplicate {
            resolver.types.insert(key.to_owned(), Type::Unknown);
        }

        let keys: Vec<_> = resolver.declarations.keys().copied().collect();
        for key in keys {
            let owner = resolver.declarations[key];
            resolver.resolve(key, owner.path, owner.declaration.name);
        }
        self.types = Arc::new(resolver.types);
        self.conversions = Arc::new(resolver.conversions);
        self.diagnostics = resolver.diagnostics;
        self.previous = sources
            .into_iter()
            .map(|source| (source.path.to_owned(), source.fingerprint.clone()))
            .collect();
    }
}

#[derive(Clone, Copy)]
struct Owner<'a> {
    path: &'a str,
    names: &'a Names,
    declaration: &'a SpecDeclaration,
    enum_conversion: Option<&'a SpecConversion>,
}

#[derive(Default)]
struct Resolver<'a> {
    declarations: BTreeMap<&'a str, Owner<'a>>,
    active: Vec<&'a str>,
    types: BTreeMap<String, Type>,
    conversions: BTreeMap<String, SpecConversion>,
    diagnostics: BTreeMap<String, Vec<Diagnostic>>,
}

impl Resolver<'_> {
    fn resolve(&mut self, key: &str, path: &str, span: Span) -> Option<Type> {
        if let Some(ty) = self.types.get(key) {
            return Some(ty.clone());
        }
        let (&key, &owner) = self.declarations.get_key_value(key)?;
        if self.active.contains(&key) {
            self.diagnostics
                .entry(path.into())
                .or_default()
                .push(Diagnostic::analysis_gap(
                    "PNX9001",
                    span,
                    format!("Recursive spec {key} is not supported yet"),
                ));
            return Some(Type::Unknown);
        }
        if self.active.len() >= 128 {
            self.error(path, span, "Spec expansion limit (128) exceeded");
            return Some(Type::Unknown);
        }

        self.active.push(key);
        let mut diagnostics = Vec::new();
        let ty = if let Some(conversion) = owner.enum_conversion {
            conversion.success_input()
        } else {
            read_type(
                owner.names,
                &owner.declaration.body,
                &mut diagnostics,
                |key, span| self.resolve(key, owner.path, span),
            )
        };
        let body = &owner.declaration.body;
        let conversion = owner
            .enum_conversion
            .cloned()
            .or_else(|| source_conversion(owner.names, body))
            .or_else(|| {
                let alias = owner.names.key(body.keyword()?);
                self.conversions.get(&alias).cloned()
            });
        if let Some(conversion) = conversion {
            self.conversions.insert(key.to_owned(), conversion);
        }
        self.active.pop();
        if !diagnostics.is_empty() {
            self.diagnostics
                .entry(owner.path.into())
                .or_default()
                .extend(diagnostics);
        }
        self.types.insert(key.to_owned(), ty.clone());
        Some(ty)
    }

    fn duplicate(&mut self, key: &str, owner: Owner<'_>) {
        self.error(
            owner.path,
            owner.declaration.name,
            format!("Spec {key} is declared in more than one source file"),
        );
    }

    fn error(&mut self, path: &str, span: Span, message: impl Into<String>) {
        self.diagnostics
            .entry(path.into())
            .or_default()
            .push(Diagnostic::new("PNX9001", span, message));
    }
}
