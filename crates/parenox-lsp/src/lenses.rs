//! Lazy reference counts for source var declarations.
use parenox_analysis::{Session, SourceSymbolKind};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::text::{Lines, Range};

#[derive(Deserialize)]
pub struct Query {
    pub range: Range,
    pub data: (String, usize, usize),
}

pub fn list(session: &Session, path: &str, source: &str, uri: &str) -> Option<Vec<Value>> {
    let (_, entries) = session.document_symbols(path)?;
    let lines = Lines::new(source);
    Some(
        entries
            .into_iter()
            .filter(|entry| is_var(entry.kind))
            .filter_map(|entry| {
                let range = lines.range(entry.selection)?;
                Some(json!({"range":range,"data":[uri,range.start.line+1,range.start.character+1]}))
            })
            .collect(),
    )
}

pub fn resolve(session: &Session, path: &str, source: &str, query: Query) -> Option<Value> {
    let (_, entries) = session.document_symbols(path)?;
    let lines = Lines::new(source);
    let entry = entries.into_iter().find(|entry| {
        is_var(entry.kind) && lines.range(entry.selection).as_ref() == Some(&query.range)
    })?;
    let count = session
        .references(path, entry.selection.start.offset, false)
        .len();
    let noun = if count == 1 {
        "reference"
    } else {
        "references"
    };
    let args = [
        json!(query.data.0),
        json!(query.range.start.line + 1),
        json!(query.range.start.character + 1),
    ];
    Some(json!({
        "range":query.range,
        "data":args,
        "command":{"title":format!("{count} {noun}"),"command":"code-lens-references","arguments":args}
    }))
}

fn is_var(kind: SourceSymbolKind) -> bool {
    matches!(
        kind,
        SourceSymbolKind::Function
            | SourceSymbolKind::Macro
            | SourceSymbolKind::Variable
            | SourceSymbolKind::Protocol
    )
}
