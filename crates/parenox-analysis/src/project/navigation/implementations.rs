//! Explicit source registrations for multimethods and protocol methods.
use std::collections::BTreeMap;

use crate::checker::ScopedReference;

use super::{Definition, FileCache, ResolvedVar, Session, SourceTarget};

impl Session {
    /// Find implementations of a source multimethod or protocol method.
    pub fn implementations(&self, path: &str, offset: usize) -> Vec<Definition> {
        let Some(method) = self.implemented_var_at(path, offset) else {
            return Vec::new();
        };
        let Some((decl, forms)) = method.source.navigation_source() else {
            return Vec::new();
        };
        let method_name = method.name;
        let owner = decl.protocol_method_owner(forms, method.name, method.span);
        let target = if let Some(protocol) = owner {
            let Some(target) =
                self.resolve_source_var(method.path, method.namespace, protocol, None)
            else {
                return Vec::new();
            };
            target
        } else if decl.is_source_multimethod(forms, method.name, method.span) {
            method
        } else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let standard_test = owner.is_some() && self.has_standard_test();
        for (candidate_path, file) in &self.files {
            if file.snapshot != self.snapshot || !file.source.contains(method_name) {
                continue;
            }
            let Some((declarations, forms)) = file.navigation_source() else {
                continue;
            };
            let mut add_target = |(spelling, span): (&str, crate::reader::Span)| {
                if self
                    .registration_reference(candidate_path, file, spelling, span.start.offset)
                    .is_some_and(|candidate| {
                        candidate.path == target.path && candidate.span == target.span
                    })
                {
                    result.push(Definition {
                        path: candidate_path.to_owned(),
                        span,
                    });
                }
            };
            if owner.is_some() {
                declarations
                    .protocol_method_targets(forms, method_name)
                    .into_iter()
                    .for_each(&mut add_target);
                if file.source.contains("reify") {
                    declarations
                        .reify_method_targets(
                            forms,
                            method_name,
                            standard_test,
                            |namespace, local, arity| self.navigation_call(namespace, local, arity),
                        )
                        .into_iter()
                        .for_each(add_target);
                }
            } else {
                declarations.method_targets(forms).for_each(add_target);
            }
        }
        result.sort_unstable_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then(a.span.start.offset.cmp(&b.span.start.offset))
        });
        result
    }

    fn implemented_var_at(&self, path: &str, offset: usize) -> Option<ResolvedVar<'_>> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        if let Some((namespace, name)) = self.literal_require_var_at(path, offset) {
            return self.resolve_source_var(path, namespace, name, None);
        }
        let references = self.collect_file_references(
            path,
            file,
            self.has_standard_test(),
            false,
            &mut BTreeMap::new(),
        );
        match self
            .reference_target(path, offset, file, &references)
            .or_else(|| self.source_target(path, offset))
        {
            Some(SourceTarget::Var(target)) => Some(target),
            Some(SourceTarget::Local(_)) => None,
            None => {
                let (span, name) = references.uses.iter().find_map(|(span, reference)| {
                    let ScopedReference::InNamespace(name) = reference else {
                        return None;
                    };
                    (span.start.offset..span.end.offset)
                        .contains(&offset)
                        .then_some((span, *name))
                })?;
                self.registration_reference(path, file, name, span.start.offset)
            }
        }
    }

    fn registration_reference<'a>(
        &'a self,
        path: &str,
        file: &'a FileCache,
        spelling: &'a str,
        offset: usize,
    ) -> Option<ResolvedVar<'a>> {
        let (declarations, _) = file.navigation_source()?;
        if let Some((namespace, name)) = declarations.qualified_reference(spelling) {
            let namespace = spelling
                .split_once('/')
                .and_then(|(qualifier, _)| self.completion_require_alias(path, qualifier, offset))
                .unwrap_or(namespace);
            let before = (namespace == declarations.namespace()).then_some(offset);
            return self.resolve_source_var(path, namespace, name, before);
        }
        if let Some(local) =
            self.resolve_source_var(path, declarations.namespace(), spelling, Some(offset))
        {
            return Some(local);
        }
        let (namespace, name) = file
            .model
            .as_ref()
            .and_then(|model| model.resolved_referral(spelling))
            .unwrap_or_else(|| declarations.top_level_call(spelling));
        if namespace == declarations.namespace() {
            return None;
        }
        self.resolve_source_var(path, namespace, name, None)
    }
}
