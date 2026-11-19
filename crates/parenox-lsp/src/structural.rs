//! Folding declaration ranges and expanding selections over the lossless CST.
use std::ops::Range as Bytes;

use parenox_analysis::SourceSymbol;
use parenox_analysis::reader;
use parenox_analysis::syntax::{SyntaxKind, SyntaxNode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::text::{Lines, Position, Range};

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FoldingOptions {
    pub line_folding_only: bool,
    pub range_limit: Option<usize>,
}

pub fn folding_ranges<'a>(
    source: &str,
    symbols: impl IntoIterator<Item = SourceSymbol<'a>>,
    options: &FoldingOptions,
) -> Vec<Value> {
    let lines = Lines::new(source);
    symbols
        .into_iter()
        .filter_map(|symbol| {
            let Range { start, end } = lines.range(symbol.range)?;
            let end_line = if options.line_folding_only && end.character == 0 {
                end.line.saturating_sub(1)
            } else {
                end.line
            };
            if start.line >= end_line {
                return None;
            }
            let mut fold = json!({"startLine":start.line,"endLine":end_line,"kind":"region"});
            if !options.line_folding_only {
                fold["startCharacter"] = json!(start.character);
                fold["endCharacter"] = json!(end.character);
            }
            Some(fold)
        })
        .take(options.range_limit.unwrap_or(usize::MAX))
        .collect()
}

pub fn selection_ranges(source: &str, edn: bool, positions: &[Position]) -> Vec<Value> {
    let root = if edn {
        reader::parse_edn(source)
    } else {
        reader::parse(source)
    }
    .syntax();
    let lines = Lines::new(source);
    positions
        .iter()
        .map(|&position| {
            let Some(offset) = lines.offset(position) else {
                return json!({"range":Range {start:position, end:position}});
            };
            let mut ranges = enclosing_ranges(&root, offset);
            if ranges.is_empty() {
                ranges.push(offset..offset);
            }
            ranges
                .into_iter()
                .rev()
                .filter_map(|range| {
                    Some(Range {
                        start: lines.position(range.start)?,
                        end: lines.position(range.end)?,
                    })
                })
                .fold(Value::Null, |parent, range| {
                    if parent.is_null() {
                        json!({"range":range})
                    } else {
                        json!({"range":range,"parent":parent})
                    }
                })
        })
        .collect()
}

fn enclosing_ranges(root: &SyntaxNode, offset: usize) -> Vec<Bytes<usize>> {
    let mut ranges: Vec<Bytes<usize>> = Vec::new();
    let mut include = |range: Bytes<usize>| {
        if range.start <= offset
            && offset <= range.end
            && ranges.last().is_none_or(|inner| {
                range != *inner && range.start <= inner.start && inner.end <= range.end
            })
        {
            ranges.push(range);
        }
    };
    let Ok(offset) = u32::try_from(offset) else {
        return ranges;
    };
    if let Some(token) = root.token_at_offset(offset.into()).right_biased() {
        if !matches!(token.kind(), SyntaxKind::Whitespace | SyntaxKind::Comma) {
            let span = token.text_range();
            include(usize::from(span.start())..usize::from(span.end()));
        }
        for node in token.parent_ancestors().take_while(|node| node != root) {
            if let Some(inner) = delimited_contents(&node) {
                include(inner);
            }
            let span = node.text_range();
            include(usize::from(span.start())..usize::from(span.end()));
        }
    }
    ranges
}

fn delimited_contents(node: &SyntaxNode) -> Option<Bytes<usize>> {
    use SyntaxKind::*;
    let first = node.first_child_or_token()?;
    let last = node.last_child_or_token()?;
    if matches!(
        (first.kind(), last.kind()),
        (OpenParen | OpenAnonymousFunction, CloseParen)
            | (OpenBracket, CloseBracket)
            | (OpenBrace | OpenSet, CloseBrace)
    ) {
        Some(usize::from(first.text_range().end())..usize::from(last.text_range().start()))
    } else {
        None
    }
}
