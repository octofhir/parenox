//! Static usages resolved by the same lexical traversal as source navigation.
use std::collections::BTreeMap;

use crate::checker::{DocumentReferences, ProjectCall, ScopedReference};
use crate::reader::Span;

use super::{Definition, FileCache, Provider, Session, SourceTarget, symbol_at};

impl Session {
    /// Find statically resolved references in the latest project snapshot.
    /// Quoted data and opaque macro arguments are not treated as references.
    pub fn references(
        &self,
        path: &str,
        offset: usize,
        include_declaration: bool,
    ) -> Vec<Definition> {
        self.find_references(path, offset, include_declaration, false)
    }

    pub fn document_highlights(&self, path: &str, offset: usize) -> Vec<Definition> {
        self.find_references(path, offset, true, true)
    }

    fn find_references(
        &self,
        path: &str,
        offset: usize,
        include_declaration: bool,
        document_only: bool,
    ) -> Vec<Definition> {
        let Some(file) = self
            .current_file(path)
            .filter(|file| file.source.is_char_boundary(offset))
        else {
            return Vec::new();
        };
        let Some((_, _)) = file.navigation_source() else {
            return Vec::new();
        };
        let mut providers = BTreeMap::new();
        let standard_test = self.has_standard_test();
        let origin = self.collect_file_references(path, file, standard_test, false, &mut providers);
        let target = self.reference_target(path, offset, file, &origin);
        let mut result = Vec::new();
        let mut push = |path: &str, span: Span| {
            result.push(Definition {
                path: path.into(),
                span,
            })
        };
        match target {
            Some(SourceTarget::Local(binding)) => {
                if include_declaration {
                    push(path, binding);
                }
                for (span, reference) in origin.uses {
                    if matches!(reference, ScopedReference::Local(candidate) if candidate == binding)
                    {
                        push(path, span);
                    }
                }
            }
            Some(SourceTarget::Var(target)) => {
                if include_declaration && (!document_only || path == target.path) {
                    push(target.path, target.span);
                }
                let Some((provider, provider_forms)) = target.source.navigation_source() else {
                    return result;
                };
                let public = provider
                    .source_var(provider_forms, target.name, true)
                    .is_some();
                let qualified = format!("{}/{}", target.namespace, target.name);
                for (candidate_path, candidate) in &self.files {
                    if candidate.snapshot != self.snapshot
                        || (document_only && candidate_path != path)
                        || (!public && candidate_path != target.path)
                        || !candidate.source.contains(target.name)
                    {
                        continue;
                    }
                    let Some((caller, _)) = candidate.navigation_source() else {
                        continue;
                    };
                    let other;
                    let occurrences = if candidate_path == path {
                        &origin
                    } else {
                        other = self.collect_file_references(
                            candidate_path,
                            candidate,
                            standard_test,
                            false,
                            &mut providers,
                        );
                        &other
                    };
                    for &(span, reference) in &occurrences.uses {
                        let matches = match reference {
                            ScopedReference::Local(_) => false,
                            ScopedReference::Referred(namespace, name) => {
                                namespace == target.namespace && name == target.name
                            }
                            ScopedReference::InNamespace(name) => {
                                candidate_path == target.path
                                    && name == target.name
                                    && provider
                                        .source_var_before(provider_forms, name, span.start.offset)
                                        .is_some()
                            }
                        };
                        if matches {
                            push(candidate_path, span);
                        }
                    }
                    for span in caller.referrals_to(&qualified) {
                        push(candidate_path, span);
                    }
                }
            }
            None => {}
        }
        result.sort_unstable_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then(a.span.start.offset.cmp(&b.span.start.offset))
        });
        result.dedup();
        result
    }

    pub(super) fn reference_target<'a>(
        &'a self,
        path: &str,
        offset: usize,
        file: &'a FileCache,
        origin: &DocumentReferences<'a>,
    ) -> Option<SourceTarget<'a>> {
        let (decl, forms) = file.navigation_source()?;
        if let Some((namespace, name)) = decl.referral_at(offset) {
            self.resolve_source_var(path, namespace, name, None)
                .map(SourceTarget::Var)
        } else if let Some((span, reference)) = origin
            .uses
            .iter()
            .copied()
            .find(|(span, _)| contains(*span, offset))
        {
            match reference {
                ScopedReference::Local(binding) => Some(SourceTarget::Local(binding)),
                ScopedReference::Referred(namespace, name) => self
                    .resolve_source_var(path, namespace, name, None)
                    .map(SourceTarget::Var),
                ScopedReference::InNamespace(name) => self
                    .resolve_source_var(path, decl.namespace(), name, Some(span.start.offset))
                    .map(SourceTarget::Var),
            }
        } else if let Some(binding) = origin.bindings.iter().find(|&&span| contains(span, offset)) {
            Some(SourceTarget::Local(*binding))
        } else {
            symbol_at(forms, offset)
                .and_then(|name| self.resolve_source_var(path, decl.namespace(), name, None))
                .filter(|target| target.path == path && contains(target.span, offset))
                .map(SourceTarget::Var)
        }
    }

    pub(super) fn collect_file_references<'a>(
        &'a self,
        path: &str,
        file: &'a FileCache,
        standard_test: bool,
        collect_calls: bool,
        providers: &mut BTreeMap<String, Option<Provider<'a>>>,
    ) -> DocumentReferences<'a> {
        let Some((declarations, forms)) = file.navigation_source() else {
            return DocumentReferences::default();
        };
        declarations.collect_references(
            forms,
            standard_test,
            collect_calls,
            |namespace, name, actual| self.reference_call(path, namespace, name, actual, providers),
        )
    }

    pub(super) fn reference_call<'a>(
        &'a self,
        path: &str,
        namespace: &str,
        name: &str,
        actual: usize,
        providers: &mut BTreeMap<String, Option<Provider<'a>>>,
    ) -> ProjectCall {
        let provider = providers.get(namespace).copied().unwrap_or_else(|| {
            let provider = self.navigation_provider(namespace);
            providers.insert(namespace.into(), provider);
            provider
        });
        provider.map_or(
            ProjectCall::Unknown,
            |(provider_path, _, declarations, forms)| {
                declarations
                    .source_function_call(forms, name, actual, provider_path != path)
                    .with_source_proof(self.query_globals.as_ref(), namespace, name)
            },
        )
    }
}

fn contains(span: Span, offset: usize) -> bool {
    (span.start.offset..span.end.offset).contains(&offset)
}
