//! Source queries over the current project snapshot.
mod archive;
mod call_hierarchy;
mod completion_docs;
mod completion_require;
mod declaration;
mod hover;
mod implementations;
mod java_completion;
mod namespace_move;
mod recovered_types;
mod rename;
mod semantic;
mod signatures;
mod specs;
mod test_tree;
mod usages;

pub use archive::ArchiveDeclarations;
pub use call_hierarchy::{CallHierarchyCalls, CallHierarchyItem};
pub use completion_docs::{SourceExport, namespace_exports};
use completion_require::LiteralRequireSite;
pub use hover::{
    DocumentHover, LocalHover, SourceHover, VarDocumentation, namespace_documentation,
};
pub(super) use recovered_types::CheckedPrefix;
pub use semantic::{InspectedReference, SemanticSymbol, SemanticSymbolKind};
use serde::Serialize;
pub use signatures::{CallSignature, CallSignatures};
pub use test_tree::{SourceTestKind, SourceTestNode};

use crate::ContractOrigin;
use crate::checker::{Declarations, ProjectCall, ScopedReference, java_type_tag_at};
use crate::reader::{Form, Kind, Span};

use super::{FileCache, Session, Source, TypeQueries};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Definition {
    pub path: String,
    pub span: Span,
}

pub struct PrivacyDeclaration<'a> {
    pub operator: &'a Form,
    pub name: &'a Form,
    pub attribute: Option<&'a Form>,
    pub core_name: &'static str,
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

impl ResolvedVar<'_> {
    fn definition(&self) -> Definition {
        Definition {
            path: self.path.into(),
            span: self.span,
        }
    }

    fn info(&self) -> VarInfo {
        let name = format!("{}/{}", self.namespace, self.name);
        let contract = self
            .source
            .model
            .as_ref()
            .and_then(|model| model.functions.get(&name))
            .and_then(|function| function.contract.as_ref());
        VarInfo {
            name,
            signatures: contract.map_or_else(Vec::new, |contract| {
                contract
                    .signatures
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            }),
            contract: contract.map(|contract| contract.origin.as_ref().clone()),
        }
    }
}

enum SourceTarget<'a> {
    Local(Span),
    Var(ResolvedVar<'a>),
}

impl Session {
    /// Find an active source contract in the provider's current snapshot.
    pub fn source_contract_declaration(
        &self,
        origin: &ContractOrigin,
    ) -> Option<(Source<'_>, Span)> {
        let (path, file) = self.files.get_key_value(origin.path.as_deref()?)?;
        if file.snapshot != self.snapshot {
            return None;
        }
        let span = file.model.as_ref()?.source_contract_span(origin)?;
        Some((
            Source {
                path,
                source: &file.source,
            },
            span,
        ))
    }

    /// Resolves a source var, local binding, namespace or named spec at a UTF-8 byte offset.
    /// Ambiguous declarations and references outside successfully read forms have no target.
    pub fn definition(&self, path: &str, offset: usize) -> Option<Definition> {
        if let Some(spec) = self.spec_definition(path, offset) {
            return Some(spec);
        }
        if let Some((namespace, name)) = self.literal_require_var_at(path, offset) {
            return Some(
                self.resolve_source_var(path, namespace, name, None)?
                    .definition(),
            );
        }
        Some(match self.source_target(path, offset) {
            Some(SourceTarget::Var(target)) => target.definition(),
            Some(SourceTarget::Local(span)) => Definition {
                path: path.into(),
                span,
            },
            None => {
                let namespace = self.namespace_reference_at(path, offset)?;
                let (path, _, provider, forms) = self.navigation_provider(namespace)?;
                Definition {
                    path: path.into(),
                    span: provider.namespace_selection(forms)?,
                }
            }
        })
    }

    /// Describes a qualified source var and any signature from the checked declaration model.
    pub fn var_at(&self, path: &str, offset: usize) -> Option<VarInfo> {
        let SourceTarget::Var(target) = self.source_target(path, offset)? else {
            return None;
        };
        Some(target.info())
    }

    pub fn complete_qualified_var(&self, path: &str, prefix: &str, limit: usize) -> Vec<String> {
        self.complete_qualified_var_at(path, prefix, 0, limit)
    }

    /// Complete a qualified var in the caller's (possibly incomplete) buffer.
    pub fn complete_qualified_var_at(
        &self,
        path: &str,
        prefix: &str,
        offset: usize,
        limit: usize,
    ) -> Vec<String> {
        let Some((qualifier, prefix)) = prefix.split_once('/').filter(|(ns, _)| !ns.is_empty())
        else {
            return Vec::new();
        };
        let Some(namespace) = self.completion_namespace_at(path, qualifier, offset) else {
            return Vec::new();
        };
        let Some((provider_path, _, provider, forms)) = self.navigation_provider(namespace) else {
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

    /// Identify a reference to the standard get or get-in functions in evaluated source.
    pub fn core_lookup_at(&self, path: &str, offset: usize) -> Option<&str> {
        if self.has_source_namespace("clojure.core") {
            return None;
        }
        let file = self.current_file(path)?;
        let (declarations, forms) = file.navigation_source()?;
        let (spelling, reference) = declarations.evaluated_reference_at(
            forms,
            offset,
            self.has_standard_test(),
            |namespace, name, arity| self.navigation_call(namespace, name, arity),
        )?;
        let (namespace, name) = if let Some((qualifier, name)) = spelling.split_once('/') {
            (self.completion_namespace_at(path, qualifier, offset)?, name)
        } else if let Some(target) = self.literal_require_referred_var(path, offset) {
            target
        } else {
            match reference {
                ScopedReference::Referred(namespace, name) => (namespace, name),
                ScopedReference::InNamespace(name)
                    if declarations.hover_core_visible(forms, name, offset) =>
                {
                    ("clojure.core", name)
                }
                _ => return None,
            }
        };
        (namespace == "clojure.core" && matches!(name, "get" | "get-in")).then_some(name)
    }

    /// Check whether a refactor can introduce an unqualified core name in this file.
    /// The caller also checks lexical binders in the enclosing syntax.
    pub fn refactor_core_visible(&self, path: &str, name: &str, offset: usize) -> bool {
        !self.has_source_namespace("clojure.core")
            && self
                .current_file(path)
                .and_then(FileCache::navigation_source)
                .is_some_and(|(declarations, forms)| {
                    !declarations.has_top_level_require
                        && declarations.hover_core_visible(forms, name, offset)
                })
    }

    /// Look up a local name at the start of an expression whose evaluation is known.
    pub fn refactor_local_binding_at(&self, path: &str, name: &str, offset: usize) -> Option<bool> {
        let (declarations, forms) = self.current_file(path)?.navigation_source()?;
        declarations.lexical_binding_at(
            forms,
            offset,
            name,
            self.has_standard_test(),
            |namespace, name, arity| self.navigation_call(namespace, name, arity),
        )
    }

    pub fn privacy_declaration_at(
        &self,
        path: &str,
        offset: usize,
    ) -> Option<PrivacyDeclaration<'_>> {
        let (declarations, forms) = self.current_file(path)?.navigation_source()?;
        let form = forms
            .iter()
            .find(|form| form.span.start.offset <= offset && offset < form.span.end.offset)?;
        let Kind::List(items) = &form.without_metadata().kind else {
            return None;
        };
        let [operator, name, tail @ ..] = items.as_slice() else {
            return None;
        };
        if name.symbol()?.contains('/') {
            return None;
        }
        let core_name = declarations.privacy_operator(forms, operator)?;
        let attribute = if matches!(core_name, "defn" | "defn-") {
            Declarations::function_privacy_attribute(tail)
        } else {
            None
        };
        Some(PrivacyDeclaration {
            operator,
            name,
            attribute,
            core_name,
        })
    }

    fn current_file(&self, path: &str) -> Option<&FileCache> {
        self.files
            .get(path)
            .filter(|file| file.snapshot == self.snapshot)
    }

    fn namespace_reference_at(&self, path: &str, offset: usize) -> Option<&str> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, _) = file.navigation_source()?;
        declarations.namespace_reference_at(offset).or_else(|| {
            let LiteralRequireSite::Namespace(namespace) =
                self.literal_require_site(path, offset)?
            else {
                return None;
            };
            Some(namespace)
        })
    }

    fn literal_require_var_at(&self, path: &str, offset: usize) -> Option<(&str, &str)> {
        let (declarations, _) = self.current_file(path)?.navigation_source()?;
        if !declarations.has_top_level_require {
            return None;
        }
        if let Some(LiteralRequireSite::Var(namespace, name)) =
            self.literal_require_site(path, offset)
        {
            return Some((namespace, name));
        }
        self.literal_require_qualified_var(path, offset)
            .or_else(|| self.literal_require_referred_var(path, offset))
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

    pub(super) fn has_source_namespace(&self, namespace: &str) -> bool {
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
    symbol_form_at(forms, offset)?.symbol()
}

fn symbol_form_at(forms: &[Form], offset: usize) -> Option<&Form> {
    source_symbol_form_at(forms, offset, false)
}

fn source_symbol_form_at(forms: &[Form], offset: usize, type_hint: bool) -> Option<&Form> {
    let index = forms.partition_point(|form| form.span.end.offset <= offset);
    let form = forms.get(index)?;
    if type_hint && let Some(tag) = java_type_tag_at(form, offset) {
        return Some(tag);
    }
    let form = form.without_metadata();
    if offset < form.span.start.offset || offset >= form.span.end.offset {
        return None;
    }
    match &form.kind {
        Kind::Symbol(_) | Kind::AnonymousArgument(_) if !type_hint => Some(form),
        Kind::List(items)
            if !matches!(
                items.first().and_then(Form::symbol),
                Some("ns" | "clojure.core/ns" | "quote")
            ) =>
        {
            source_symbol_form_at(items, offset, type_hint)
        }
        Kind::Vector(items)
        | Kind::Map(items)
        | Kind::Set(items)
        | Kind::AutoNamespacedMap { entries: items, .. } => {
            source_symbol_form_at(items, offset, type_hint)
        }
        Kind::AnonymousFunction(function) => {
            source_symbol_form_at(&function.body, offset, type_hint)
        }
        _ => None,
    }
}

const MAX_CACHED_TYPE_QUERIES: usize = 128;

fn cached_type_query<T: Clone>(
    cache: &TypeQueries<T>,
    start: usize,
    infer: impl FnOnce() -> Option<T>,
) -> Option<T> {
    let entries = cache.get_or_init(Default::default);
    if let Some(value) = entries.lock().unwrap().get(&start).cloned() {
        return value;
    }
    let value = infer();
    let mut entries = entries.lock().unwrap();
    if let Some(current) = entries.get(&start) {
        return current.clone();
    }
    if entries.len() == MAX_CACHED_TYPE_QUERIES {
        entries.pop_first();
    }
    entries.insert(start, value.clone());
    value
}

#[cfg(test)]
mod cache_tests {
    use super::{MAX_CACHED_TYPE_QUERIES, TypeQueries, cached_type_query};

    #[test]
    fn type_query_cache_reuses_results_and_bounds_distinct_sites() {
        let mut cache = TypeQueries::default();
        assert_eq!(
            cached_type_query(&cache, 0, || Some("found")),
            Some("found")
        );
        assert_eq!(cached_type_query(&cache, 1, || None), None);
        assert_eq!(
            cached_type_query(&cache, 0, || panic!("cached result")),
            Some("found")
        );
        assert_eq!(cached_type_query(&cache, 1, || panic!("cached miss")), None);
        for offset in 2..MAX_CACHED_TYPE_QUERIES + 10 {
            cached_type_query(&cache, offset, || Some("other result"));
        }
        assert_eq!(
            cache.get().unwrap().lock().unwrap().len(),
            MAX_CACHED_TYPE_QUERIES
        );
        cache.take();
        assert_eq!(
            cached_type_query(&cache, 0, || Some("updated")),
            Some("updated")
        );
    }
}
