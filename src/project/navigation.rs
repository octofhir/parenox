//! Source queries over the current project snapshot.
use serde::Serialize;

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

enum SourceTarget<'a> {
    Local(Span),
    Var(ResolvedVar<'a>),
}

impl Session {
    /// Resolves a source var or local binding at a UTF-8 byte offset in the latest snapshot.
    /// Ambiguous namespaces and references outside successfully read forms have no target.
    pub fn definition(&self, path: &str, offset: usize) -> Option<Definition> {
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

    fn source_target(&self, path: &str, offset: usize) -> Option<SourceTarget<'_>> {
        let file = self
            .files
            .get(path)
            .filter(|file| file.snapshot == self.snapshot)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        let qualified = declarations
            .referral_at(offset)
            .or_else(|| declarations.qualified_reference(symbol_at(forms, offset)?));
        let (namespace, local) = match qualified {
            Some(target) => target,
            None => match declarations.scoped_reference_at(
                forms,
                offset,
                |namespace, local, actual| {
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
                },
            )? {
                ScopedReference::Referred(namespace, local) => (namespace, local),
                ScopedReference::Local(span) => return Some(SourceTarget::Local(span)),
            },
        };
        let (target_path, candidate, provider, forms) = self.navigation_provider(namespace)?;
        let span = provider.source_var(forms, local, target_path != path)?;
        Some(SourceTarget::Var(ResolvedVar {
            namespace,
            name: local,
            path: target_path,
            source: candidate,
            span,
        }))
    }

    fn navigation_provider(
        &self,
        namespace: &str,
    ) -> Option<(&str, &FileCache, &Declarations, &[Form])> {
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
