//! Source queries over the current project snapshot.
use serde::Serialize;

use crate::reader::{Form, Kind, Span};

use super::Session;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Definition {
    pub path: String,
    pub span: Span,
}

impl Session {
    /// Resolves a qualified source var at a UTF-8 byte offset in the latest snapshot.
    /// Ambiguous namespaces and references outside successfully read forms have no target.
    pub fn definition(&self, path: &str, offset: usize) -> Option<Definition> {
        let file = self
            .files
            .get(path)
            .filter(|file| file.snapshot == self.snapshot)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let declarations = file.declarations.as_ref()?;
        let name = symbol_at(&file.forms, offset)?;
        let (namespace, local) = declarations.qualified_reference(name)?;
        let mut providers = self.files.iter().filter_map(|(path, candidate)| {
            if candidate.snapshot != self.snapshot {
                return None;
            }
            let provider = candidate.declarations.as_ref()?;
            (provider.namespace() == namespace).then_some((path, candidate, provider))
        });
        let (target_path, candidate, provider) = providers.next()?;
        if providers.next().is_some() {
            return None;
        }
        let span = provider.source_var(&candidate.forms, local, target_path != path)?;
        Some(Definition {
            path: target_path.clone(),
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
