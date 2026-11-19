//! LSP rendering and ranking of source symbols.
use std::collections::{BTreeMap, BTreeSet};

use parenox_analysis::{SourceSymbol, SourceSymbolKind};
use serde::Deserialize;
use serde_json::{Value, json};

use super::text::{Lines, file_uri};

#[derive(Deserialize)]
pub struct Search {
    pub query: String,
}

pub struct Candidate<'a> {
    pub path: &'a str,
    pub editor_uri: Option<&'a str>,
    pub source: &'a str,
    pub namespace: &'a str,
    pub symbol: SourceSymbol<'a>,
}

pub fn document(
    symbols: Vec<SourceSymbol<'_>>,
    namespace: &str,
    uri: &str,
    source: &str,
    hierarchical: bool,
) -> Vec<Value> {
    let lines = Lines::new(source);
    if hierarchical {
        let mut symbols = symbols.into_iter().peekable();
        let mut tree = Vec::new();
        while let Some(symbol) = symbols.next() {
            tree.extend(document_node(symbol, &mut symbols, &lines));
        }
        tree
    } else {
        symbols
            .into_iter()
            .filter_map(|symbol| {
                Some(json!({
                    "name":symbol.name,"kind":kind(symbol.kind),"containerName":namespace,
                    "location":{"uri":uri,"range":lines.range(symbol.selection)?}
                }))
            })
            .collect()
    }
}

fn document_node<'a>(
    symbol: SourceSymbol<'a>,
    remaining: &mut std::iter::Peekable<impl Iterator<Item = SourceSymbol<'a>>>,
    lines: &Lines<'_>,
) -> Option<Value> {
    let mut children = Vec::new();
    while remaining.peek().is_some_and(|next| {
        next.range.start.offset > symbol.range.start.offset
            && next.range.end.offset <= symbol.range.end.offset
    }) {
        let next = remaining.next().unwrap();
        children.extend(document_node(next, remaining, lines));
    }
    Some(json!({
        "name":symbol.name, "kind":kind(symbol.kind), "range":lines.range(symbol.range)?,
        "selectionRange":lines.range(symbol.selection)?, "children":children
    }))
}

pub fn search<'a>(query: &str, symbols: impl Iterator<Item = Candidate<'a>>) -> Vec<Value> {
    const LIMIT: usize = 200;
    let query = query.trim().to_lowercase();
    let mut matches = symbols
        .filter_map(|candidate| {
            let name = candidate.symbol.name.to_lowercase();
            let rank = if name == query {
                0
            } else if name.starts_with(&query) {
                1
            } else if name.contains(&query) {
                2
            } else if format!("{}/{}", candidate.namespace, name)
                .to_lowercase()
                .contains(&query)
            {
                3
            } else {
                return None;
            };
            Some((rank, candidate))
        })
        .collect::<Vec<_>>();
    matches.sort_unstable_by_key(|(rank, candidate)| {
        (
            *rank,
            candidate.symbol.name,
            candidate.path,
            candidate.symbol.selection.start.offset,
        )
    });
    let mut seen = BTreeSet::new();
    let mut files = BTreeMap::new();
    matches.into_iter().filter_map(|(_, candidate)| {
        let symbol = candidate.symbol;
        if !seen.insert((candidate.path, symbol.selection.start.offset, symbol.name)) {
            return None;
        }
        let uri = candidate.editor_uri.map_or_else(|| file_uri(candidate.path), str::to_owned);
        let lines = files.entry((candidate.path, candidate.source.as_ptr()))
            .or_insert_with(|| Lines::new(candidate.source));
        let range = lines.range(symbol.selection)?;
        Some(json!({"name":symbol.name,"kind":kind(symbol.kind),"containerName":candidate.namespace,
                   "location":{"uri":uri,"range":range}}))
    }).take(LIMIT).collect()
}

fn kind(kind: SourceSymbolKind) -> u8 {
    use SourceSymbolKind::*;
    match kind {
        Namespace => 3,
        Function | Macro | Test => 12,
        Method => 6,
        Protocol => 11,
        Type => 5,
        Variable => 13,
        Spec => 14,
    }
}
