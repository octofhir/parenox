//! Source queries over the current project snapshot.
use serde::Serialize;

use crate::ContractOrigin;
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

impl Session {
    /// Resolves a qualified source var at a UTF-8 byte offset in the latest snapshot.
    /// Ambiguous namespaces and references outside successfully read forms have no target.
    pub fn definition(&self, path: &str, offset: usize) -> Option<Definition> {
        let target = self.resolve_var(path, offset)?;
        Some(Definition {
            path: target.path.into(),
            span: target.span,
        })
    }

    /// Describes a qualified source var and any signature from the checked declaration model.
    pub fn var_at(&self, path: &str, offset: usize) -> Option<VarInfo> {
        let target = self.resolve_var(path, offset)?;
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

    fn resolve_var(&self, path: &str, offset: usize) -> Option<ResolvedVar<'_>> {
        let file = self
            .files
            .get(path)
            .filter(|file| file.snapshot == self.snapshot)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        let (namespace, local) = declarations
            .referral_at(offset)
            .or_else(|| declarations.qualified_reference(symbol_at(forms, offset)?))?;
        let mut providers = self.files.iter().filter_map(|(path, candidate)| {
            if candidate.snapshot != self.snapshot {
                return None;
            }
            let (provider, forms) = candidate.navigation_source()?;
            (provider.namespace() == namespace).then_some((path, candidate, provider, forms))
        });
        let (target_path, candidate, provider, forms) = providers.next()?;
        if providers.next().is_some() {
            return None;
        }
        let span = provider.source_var(forms, local, target_path != path)?;
        Some(ResolvedVar {
            namespace,
            name: local,
            path: target_path,
            source: candidate,
            span,
        })
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
        Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => symbol_at(items, offset),
        Kind::AnonymousFunction(function) => symbol_at(&function.body, offset),
        _ => None,
    }
}
