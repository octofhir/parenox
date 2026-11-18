//! Project-wide spec declarations, resolution and cache validity.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::declarations::{Declarations, Names, SpecDeclaration};
use super::type_forms::read_type;
use crate::Diagnostic;
use crate::reader::Span;
use crate::types::Type;

#[derive(PartialEq, Eq)]
pub(crate) struct SpecFingerprint {
    names: Names,
    bodies: Vec<(String, Span, Span, String)>,
}

impl Declarations {
    pub fn spec_fingerprint(&self, source: &str) -> Option<SpecFingerprint> {
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
        })
    }
}

pub(crate) struct SpecSource<'a> {
    pub path: &'a str,
    pub declarations: &'a Declarations,
    pub fingerprint: &'a Arc<SpecFingerprint>,
}

#[derive(Default)]
pub(crate) struct SpecCache {
    previous: Vec<(String, Arc<SpecFingerprint>)>,
    pub types: Arc<BTreeMap<String, Type>>,
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
                };
                if let Some(previous) = resolver.declarations.get(key.as_str()).copied() {
                    if duplicate.insert(key.as_str()) {
                        resolver.duplicate(key, previous);
                    }
                    resolver.duplicate(key, owner);
                } else {
                    resolver.declarations.insert(key, owner);
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
}

#[derive(Default)]
struct Resolver<'a> {
    declarations: BTreeMap<&'a str, Owner<'a>>,
    active: Vec<&'a str>,
    types: BTreeMap<String, Type>,
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
        let ty = read_type(
            owner.names,
            &owner.declaration.body,
            &mut diagnostics,
            |key, span| self.resolve(key, owner.path, span),
        );
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
