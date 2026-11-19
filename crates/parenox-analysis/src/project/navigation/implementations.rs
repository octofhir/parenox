//! Static defmethod registrations for source multimethods.
use std::collections::BTreeMap;

use crate::checker::ScopedReference;

use super::{Definition, FileCache, ResolvedVar, Session, SourceTarget};

impl Session {
    /// Find source methods of a multimethod declaration or resolved usage.
    pub fn implementations(&self, path: &str, offset: usize) -> Vec<Definition> {
        let Some(target) = self.multimethod_at(path, offset) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for (candidate_path, file) in &self.files {
            if file.snapshot != self.snapshot || !file.source.contains(target.name) {
                continue;
            }
            let Some((declarations, forms)) = file.navigation_source() else {
                continue;
            };
            for (spelling, span) in declarations.method_targets(forms) {
                if self
                    .multimethod_reference(candidate_path, file, spelling, span.start.offset)
                    .is_some_and(|candidate| {
                        candidate.path == target.path && candidate.span == target.span
                    })
                {
                    result.push(Definition {
                        path: candidate_path.to_owned(),
                        span,
                    });
                }
            }
        }
        result.sort_unstable_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then(a.span.start.offset.cmp(&b.span.start.offset))
        });
        result
    }

    fn multimethod_at(&self, path: &str, offset: usize) -> Option<ResolvedVar<'_>> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let references = self.collect_file_references(
            path,
            file,
            self.has_standard_test(),
            false,
            &mut BTreeMap::new(),
        );
        let target = match self.reference_target(path, offset, file, &references) {
            Some(SourceTarget::Var(target)) => target,
            Some(SourceTarget::Local(_)) => return None,
            None => {
                let (span, name) = references.uses.iter().find_map(|(span, reference)| {
                    let ScopedReference::InNamespace(name) = reference else {
                        return None;
                    };
                    (span.start.offset..span.end.offset)
                        .contains(&offset)
                        .then_some((span, *name))
                })?;
                self.multimethod_reference(path, file, name, span.start.offset)?
            }
        };
        let (declarations, forms) = target.source.navigation_source()?;
        declarations
            .is_source_multimethod(forms, target.name, target.span)
            .then_some(target)
    }

    fn multimethod_reference<'a>(
        &'a self,
        path: &str,
        file: &'a FileCache,
        spelling: &'a str,
        offset: usize,
    ) -> Option<ResolvedVar<'a>> {
        let (declarations, _) = file.navigation_source()?;
        if let Some((namespace, name)) = declarations.qualified_reference(spelling) {
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
