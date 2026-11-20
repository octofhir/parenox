//! Documentation for source functions suggested by completion.
use std::fmt::Write;

use super::{FileCache, Session, archive::namespace_declarations};
use crate::SourceSymbolKind;

#[derive(Clone)]
pub struct SourceExport {
    pub name: String,
    pub callable: bool,
}

/// Read public source declarations from a namespace supplied by the caller.
/// Classpath consumers use this query without bringing the source into a project check.
pub fn namespace_exports(
    source: &str,
    namespace: &str,
    portable: bool,
) -> Option<Vec<SourceExport>> {
    let (declarations, forms) = namespace_declarations(source, namespace, portable)?;
    let mut exports: Vec<_> = declarations
        .outline(&forms)
        .into_iter()
        .filter(|symbol| {
            !matches!(
                symbol.kind,
                SourceSymbolKind::Namespace | SourceSymbolKind::Spec
            )
        })
        .filter(|symbol| declarations.source_var(&forms, symbol.name, true).is_some())
        .map(|symbol| SourceExport {
            name: symbol.name.into(),
            callable: matches!(
                symbol.kind,
                SourceSymbolKind::Function
                    | SourceSymbolKind::Macro
                    | SourceSymbolKind::Method
                    | SourceSymbolKind::Test
            ),
        })
        .collect();
    exports.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    exports.dedup_by(|a, b| a.name == b.name);
    Some(exports)
}

impl Session {
    /// Resolve a completion qualifier using the caller's namespace declaration.
    pub fn completion_namespace<'a>(&'a self, path: &str, qualifier: &'a str) -> Option<&'a str> {
        self.completion_namespace_at(path, qualifier, 0)
    }

    pub fn completion_namespace_at<'a>(
        &'a self,
        path: &str,
        qualifier: &'a str,
        offset: usize,
    ) -> Option<&'a str> {
        let (decl, _) = self
            .current_file(path)
            .and_then(FileCache::navigation_source)?;
        Some(
            self.completion_require_alias(path, qualifier, offset)
                .unwrap_or_else(|| decl.qualifier(qualifier)),
        )
    }

    pub fn completion_archive_namespace<'a>(
        &'a self,
        path: &str,
        qualifier: &'a str,
        offset: usize,
    ) -> Option<&'a str> {
        let namespace = self.completion_namespace_at(path, qualifier, offset)?;
        // An indexed namespace takes precedence, even if it is ambiguous
        // or has no candidates for the current prefix.
        (!self.has_source_namespace(namespace)).then_some(namespace)
    }

    pub fn completion_documentation(
        &self,
        path: &str,
        label: &str,
        expected_namespace: &str,
    ) -> Option<String> {
        self.completion_documentation_at(path, label, expected_namespace, 0)
    }

    /// Recheck the original completion target against the current source snapshot.
    pub fn completion_documentation_at(
        &self,
        path: &str,
        label: &str,
        expected_namespace: &str,
        offset: usize,
    ) -> Option<String> {
        let (qualifier, local) = label.split_once('/')?;
        let namespace = self.completion_namespace_at(path, qualifier, offset)?;
        if namespace != expected_namespace {
            return None;
        }
        let target = self.resolve_source_var(path, namespace, local, None)?;
        let info = target.info();
        let docs = target.documentation();
        if docs.arglists.is_empty() && docs.documentation.is_none() && info.contract.is_none() {
            return None;
        }
        let mut result = info.name;
        for arglist in docs.arglists {
            write!(result, "\n({} {arglist})", target.name).ok()?;
        }
        if let Some(documentation) = docs.documentation {
            result.push_str("\n\n");
            result.push_str(&documentation);
        }
        if info.contract.is_some() {
            result.push_str("\n\nChecked contracts");
            for signature in info.signatures {
                write!(result, "\n{signature}").ok()?;
            }
        }
        Some(result)
    }
}
