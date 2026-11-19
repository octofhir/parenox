//! Source queries over the current project snapshot.
mod call_hierarchy;
mod completion_docs;
mod declaration;
mod implementations;
mod rename;
mod semantic;
mod signatures;
mod specs;
mod test_tree;
mod usages;

pub use call_hierarchy::{CallHierarchyCalls, CallHierarchyItem};
pub use semantic::{SemanticSymbol, SemanticSymbolKind};
use serde::Serialize;
pub use signatures::{CallSignature, CallSignatures};
pub use test_tree::{SourceTestKind, SourceTestNode};

use crate::ContractOrigin;
use crate::checker::{Declarations, ProjectCall, ScopedReference};
use crate::reader::{Form, Kind, Span};

use super::{FileCache, Session};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Definition {
    pub path: String,
    pub span: Span,
}

#[derive(Serialize)]
pub struct VarInfo {
    pub name: String,
    pub signatures: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<ContractOrigin>,
}

struct ResolvedVar<'a> {
    namespace: &'a str,
    name: &'a str,
    path: &'a str,
    source: &'a FileCache,
    span: Span,
}

type Provider<'a> = (&'a str, &'a FileCache, &'a Declarations, &'a [Form]);

enum SourceTarget<'a> {
    Local(Span),
    Var(ResolvedVar<'a>),
}

impl Session {
    /// Resolves a source var, local binding or named spec at a UTF-8 byte offset.
    /// Ambiguous declarations and references outside successfully read forms have no target.
    pub fn definition(&self, path: &str, offset: usize) -> Option<Definition> {
        if let Some(spec) = self.spec_definition(path, offset) {
            return Some(spec);
        }
        Some(match self.source_target(path, offset)? {
            SourceTarget::Var(target) => Definition {
                path: target.path.into(),
                span: target.span,
            },
            SourceTarget::Local(span) => Definition {
                path: path.into(),
                span,
            },
        })
    }

    /// Describes a qualified source var and any signature from the checked declaration model.
    pub fn var_at(&self, path: &str, offset: usize) -> Option<VarInfo> {
        let SourceTarget::Var(target) = self.source_target(path, offset)? else {
            return None;
        };
        let name = format!("{}/{}", target.namespace, target.name);
        let contract = target
            .source
            .model
            .as_ref()
            .and_then(|model| model.functions.get(&name))
            .and_then(|function| function.contract.as_ref());
        Some(VarInfo {
            name,
            signatures: contract.map_or_else(Vec::new, |contract| {
                contract
                    .signatures
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            }),
            contract: contract.map(|contract| contract.origin.as_ref().clone()),
        })
    }

    /// Complete a qualified var from the current source snapshot, using its
    /// namespace or an alias in the caller's (possibly incomplete) buffer.
    pub fn complete_qualified_var(&self, path: &str, prefix: &str, limit: usize) -> Vec<String> {
        let Some((qualifier, prefix)) = prefix.split_once('/').filter(|(ns, _)| !ns.is_empty())
        else {
            return Vec::new();
        };
        let Some((caller, _)) = self
            .current_file(path)
            .and_then(FileCache::navigation_source)
        else {
            return Vec::new();
        };
        let Some((provider_path, _, provider, forms)) =
            self.navigation_provider(caller.qualifier(qualifier))
        else {
            return Vec::new();
        };
        provider
            .top_level_names(forms)
            .into_iter()
            .filter(|name| {
                name.starts_with(prefix)
                    && provider
                        .source_var(forms, name, provider_path != path)
                        .is_some()
            })
            .take(limit)
            .map(|name| format!("{qualifier}/{name}"))
            .collect()
    }

    /// Complete named specs registered in this project, preserving the alias
    /// or auto-resolution prefix used by the caller.
    pub fn complete_spec_key(&self, path: &str, prefix: &str, limit: usize) -> Vec<String> {
        let Some((caller, _)) = self
            .current_file(path)
            .and_then(FileCache::navigation_source)
        else {
            return Vec::new();
        };
        let (search, qualified) = if let Some(auto) = prefix.strip_prefix("::") {
            let (namespace, local) = auto
                .split_once('/')
                .map_or((caller.namespace(), auto), |(alias, local)| {
                    (caller.qualifier(alias), local)
                });
            (format!(":{namespace}/{local}"), true)
        } else {
            (prefix.to_owned(), false)
        };
        self.specs
            .types
            .range(search.clone()..)
            .map_while(|(key, _)| key.strip_prefix(&search).map(|suffix| (key, suffix)))
            .take(limit)
            .map(|(key, suffix)| {
                if qualified {
                    format!("{prefix}{suffix}")
                } else {
                    key.clone()
                }
            })
            .collect()
    }

    pub fn core_completion_visible(&self, path: &str, name: &str, offset: usize) -> bool {
        self.current_file(path)
            .and_then(FileCache::navigation_source)
            .is_none_or(|(decl, forms)| decl.core_completion_visible(forms, name, offset))
    }

    fn current_file(&self, path: &str) -> Option<&FileCache> {
        self.files
            .get(path)
            .filter(|file| file.snapshot == self.snapshot)
    }

    fn source_target(&self, path: &str, offset: usize) -> Option<SourceTarget<'_>> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        self.source_target_in(path, offset, declarations, forms)
    }

    fn source_target_in<'a>(
        &'a self,
        path: &str,
        offset: usize,
        declarations: &'a Declarations,
        forms: &'a [Form],
    ) -> Option<SourceTarget<'a>> {
        let qualified = declarations
            .referral_at(offset)
            .or_else(|| declarations.qualified_reference(symbol_at(forms, offset)?));
        let (namespace, local, before) = match qualified {
            Some((namespace, local)) => (namespace, local, None),
            None => match declarations.scoped_reference_at(
                forms,
                offset,
                self.has_standard_test(),
                |namespace, local, actual| self.navigation_call(namespace, local, actual),
            )? {
                ScopedReference::Referred(namespace, local) => (namespace, local, None),
                ScopedReference::InNamespace(local) => {
                    (declarations.namespace(), local, Some(offset))
                }
                ScopedReference::Local(span) => return Some(SourceTarget::Local(span)),
            },
        };
        self.resolve_source_var(path, namespace, local, before)
            .map(SourceTarget::Var)
    }

    fn resolve_source_var<'a>(
        &'a self,
        path: &str,
        namespace: &'a str,
        local: &'a str,
        before: Option<usize>,
    ) -> Option<ResolvedVar<'a>> {
        let (target_path, candidate, provider, forms) = self.navigation_provider(namespace)?;
        let span = if let Some(offset) = before {
            provider.source_var_before(forms, local, offset)
        } else {
            provider.source_var(forms, local, target_path != path)
        }?;
        Some(ResolvedVar {
            namespace,
            name: local,
            path: target_path,
            source: candidate,
            span,
        })
    }

    fn has_standard_test(&self) -> bool {
        !self.has_source_namespace("clojure.test")
    }

    fn has_source_namespace(&self, namespace: &str) -> bool {
        self.files.values().any(|file| {
            file.snapshot == self.snapshot
                && file
                    .navigation_source()
                    .is_some_and(|(declarations, _)| declarations.namespace() == namespace)
        })
    }

    fn navigation_call(&self, namespace: &str, local: &str, actual: usize) -> ProjectCall {
        self.navigation_provider(namespace)
            .filter(|(_, file, declarations, forms)| {
                file.parse_error.is_none()
                    && *file
                        .known_var_surface
                        .get_or_init(|| declarations.has_known_var_surface(forms))
            })
            .map_or(ProjectCall::Unknown, |(_, _, declarations, forms)| {
                declarations.project_call(forms, local, actual)
            })
    }

    fn navigation_provider(&self, namespace: &str) -> Option<Provider<'_>> {
        let mut providers = self.files.iter().filter_map(|(path, candidate)| {
            if candidate.snapshot != self.snapshot {
                return None;
            }
            let (provider, forms) = candidate.navigation_source()?;
            (provider.namespace() == namespace).then_some((
                path.as_str(),
                candidate,
                provider,
                forms,
            ))
        });
        let found = providers.next()?;
        providers.next().is_none().then_some(found)
    }
}

fn symbol_at(forms: &[Form], offset: usize) -> Option<&str> {
    let index = forms.partition_point(|form| form.span.end.offset <= offset);
    let form = forms.get(index)?.without_metadata();
    if offset < form.span.start.offset || offset >= form.span.end.offset {
        return None;
    }
    match &form.kind {
        Kind::Symbol(name) => Some(name),
        Kind::List(items)
            if !matches!(
                items.first().and_then(Form::symbol),
                Some("ns" | "clojure.core/ns" | "quote")
            ) =>
        {
            symbol_at(items, offset)
        }
        Kind::Vector(items)
        | Kind::Map(items)
        | Kind::Set(items)
        | Kind::AutoNamespacedMap { entries: items, .. } => symbol_at(items, offset),
        Kind::AnonymousFunction(function) => symbol_at(&function.body, offset),
        _ => None,
    }
}
