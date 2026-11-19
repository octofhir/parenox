//! Translate source refactoring edits to LSP workspace edits.
use std::collections::BTreeMap;

use parenox_analysis::Session;
use serde_json::{Value, json};

use super::text::Lines;

pub struct Document<'a> {
    pub source: &'a str,
    pub uri: String,
    pub version: Option<i64>,
}

pub fn prepare(session: &Session, path: &str, offset: usize, source: &str) -> Option<Value> {
    let span = session.prepare_rename(path, offset)?;
    let placeholder = source.get(span.start.offset..span.end.offset)?;
    Some(json!({"range":Lines::new(source).range(span)?,"placeholder":placeholder}))
}

pub fn linked(session: &Session, path: &str, offset: usize, source: &str) -> Option<Value> {
    let lines = Lines::new(source);
    let ranges = session
        .linked_editing_ranges(path, offset)?
        .into_iter()
        .map(|span| lines.range(span))
        .collect::<Option<Vec<_>>>()?;
    Some(json!({"ranges":ranges}))
}

pub fn edits<'a>(
    session: &Session,
    path: &str,
    offset: usize,
    new_name: &str,
    versioned: bool,
    mut lookup: impl FnMut(&str) -> Option<Document<'a>>,
) -> Result<Value, String> {
    let changes = session.rename(path, offset, new_name)?;
    let mut document_changes = Vec::new();
    let mut changes_by_uri = BTreeMap::new();
    for group in changes.chunk_by(|a, b| a.path == b.path) {
        let file = &group[0].path;
        let document =
            lookup(file).ok_or_else(|| format!("Rename needs an editable source file: {file}"))?;
        let lines = Lines::new(document.source);
        let edits = group
            .iter()
            .map(|edit| Some(json!({"range":lines.range(edit.span)?,"newText":new_name})))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| format!("Rename span is stale: {file}"))?;
        if versioned {
            document_changes.push(json!({
                "textDocument":{"uri":document.uri,"version":document.version},"edits":edits
            }));
        } else {
            changes_by_uri.insert(document.uri, edits);
        }
    }
    Ok(if versioned {
        json!({"documentChanges":document_changes})
    } else {
        json!({"changes":changes_by_uri})
    })
}
