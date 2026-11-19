//! Symbols from the same source snapshot used by checks and navigation.
use crate::reader::Span;

use super::{FileCache, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceSymbolKind {
    Namespace,
    Function,
    Macro,
    Test,
    Method,
    Protocol,
    Type,
    Variable,
    Spec,
}

#[derive(Clone, Copy, Debug)]
pub struct SourceSymbol<'a> {
    pub name: &'a str,
    pub kind: SourceSymbolKind,
    pub range: Span,
    pub selection: Span,
}

impl Session {
    pub fn document_symbols(&self, path: &str) -> Option<(&str, Vec<SourceSymbol<'_>>)> {
        let file = self
            .files
            .get(path)
            .filter(|file| file.snapshot == self.snapshot)?;
        outline(file)
    }

    pub fn project_symbols(&self) -> impl Iterator<Item = (&str, &str, SourceSymbol<'_>)> {
        self.files
            .iter()
            .filter_map(|(path, file)| {
                if file.snapshot != self.snapshot {
                    return None;
                }
                let (namespace, symbols) = outline(file)?;
                Some(
                    symbols
                        .into_iter()
                        .map(move |symbol| (path.as_str(), namespace, symbol)),
                )
            })
            .flatten()
    }
}

fn outline(file: &FileCache) -> Option<(&str, Vec<SourceSymbol<'_>>)> {
    let (declarations, forms) = file.navigation_source()?;
    Some((declarations.namespace(), declarations.outline(forms)))
}
