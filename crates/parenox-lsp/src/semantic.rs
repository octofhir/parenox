//! LSP semantic token legend and UTF-16 delta encoding.
use std::cmp::Reverse;
use std::ops::Range;

use parenox_analysis::reader;
use parenox_analysis::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};
use parenox_analysis::{SemanticSymbolKind, Session};
use serde_json::{Value, json};

use crate::text::Lines;

const TYPES: [&str; 11] = [
    "namespace",
    "type",
    "function",
    "macro",
    "keyword",
    "class",
    "variable",
    "method",
    "event",
    "interface",
    "comment",
];
const MODIFIERS: [&str; 3] = ["definition", "defaultLibrary", "implementation"];

#[repr(usize)]
#[derive(Clone, Copy)]
enum TokenKind {
    Namespace,
    Type,
    Function,
    Macro,
    Keyword,
    Class,
    Variable,
    Method,
    Event,
    Interface,
    Comment,
}

impl From<SemanticSymbolKind> for TokenKind {
    fn from(kind: SemanticSymbolKind) -> Self {
        match kind {
            SemanticSymbolKind::Namespace => Self::Namespace,
            SemanticSymbolKind::Type => Self::Type,
            SemanticSymbolKind::Function => Self::Function,
            SemanticSymbolKind::Macro => Self::Macro,
            SemanticSymbolKind::Class => Self::Class,
            SemanticSymbolKind::Variable => Self::Variable,
            SemanticSymbolKind::Method => Self::Method,
            SemanticSymbolKind::Interface => Self::Interface,
        }
    }
}

struct Token {
    bytes: Range<usize>,
    kind: TokenKind,
    modifiers: usize,
}

pub fn capability() -> Value {
    json!({"legend":{"tokenTypes":TYPES,"tokenModifiers":MODIFIERS},"full":true,"range":true})
}

pub fn tokens(
    source: &str,
    session: &Session,
    path: &str,
    selection: Option<Range<usize>>,
) -> Value {
    if selection.as_ref().is_some_and(Range::is_empty) {
        return json!({"data":[]});
    }
    let syntax = reader::parse(source).syntax();
    let mut tokens = Vec::new();
    syntax_tokens(&syntax, source, &mut tokens);
    for symbol in session.semantic_symbols(path) {
        let bytes = symbol.span.start.offset..symbol.span.end.offset;
        let Some(spelling) = source.get(bytes.clone()) else {
            continue;
        };
        let kind = symbol.kind.into();
        let modifiers = usize::from(symbol.definition) | (usize::from(symbol.default_library) << 1);
        if matches!(
            kind,
            TokenKind::Function | TokenKind::Macro | TokenKind::Keyword
        ) {
            named_token(&mut tokens, spelling, bytes.start, kind, modifiers);
        } else {
            tokens.push(Token {
                bytes,
                kind,
                modifiers,
            });
        }
    }
    // Longer tokens take precedence, so a discarded form cannot overlap
    // another token even if a document snapshot was incomplete.
    tokens.sort_unstable_by_key(|token| {
        (
            token.bytes.start,
            Reverse(token.bytes.len()),
            Reverse(token.modifiers),
        )
    });
    let lines = Lines::new(source);
    let mut data = Vec::new();
    let (mut end, mut line, mut column) = (0, 0, 0);
    for token in tokens {
        if token.bytes.is_empty() || token.bytes.start < end {
            continue;
        }
        end = token.bytes.end;
        if selection
            .as_ref()
            .is_some_and(|range| token.bytes.start >= range.end || token.bytes.end <= range.start)
        {
            continue;
        }
        let Some(start) = lines.position(token.bytes.start) else {
            continue;
        };
        let length = source[token.bytes].encode_utf16().count();
        data.extend([
            start.line - line,
            if start.line == line {
                start.character - column
            } else {
                start.character
            },
            length,
            token.kind as usize,
            token.modifiers,
        ]);
        line = start.line;
        column = start.character;
    }
    json!({"data":data})
}

fn syntax_tokens(node: &SyntaxNode, source: &str, output: &mut Vec<Token>) {
    for child in node.children_with_tokens() {
        match child {
            SyntaxElement::Node(node) if node.kind() == SyntaxKind::Discard => {
                let bytes = node.text_range();
                let mut start = usize::from(bytes.start());
                for piece in source[start..usize::from(bytes.end())].split_inclusive(['\r', '\n']) {
                    let end = start + piece.trim_end_matches(['\r', '\n']).len();
                    output.push(Token {
                        bytes: start..end,
                        kind: TokenKind::Comment,
                        modifiers: 0,
                    });
                    start += piece.len();
                }
            }
            SyntaxElement::Node(node) => syntax_tokens(&node, source, output),
            SyntaxElement::Token(token) if token.kind() == SyntaxKind::Keyword => {
                let text = token.text();
                let name = text.trim_start_matches(':');
                named_token(
                    output,
                    name,
                    usize::from(token.text_range().start()) + text.len() - name.len(),
                    TokenKind::Keyword,
                    0,
                );
            }
            _ => {}
        }
    }
}

fn named_token(
    output: &mut Vec<Token>,
    name: &str,
    mut start: usize,
    kind: TokenKind,
    modifiers: usize,
) {
    let end = start + name.len();
    if let Some((qualifier, local)) = name.split_once('/')
        && !qualifier.is_empty()
        && !local.is_empty()
    {
        let slash = start + qualifier.len();
        output.push(Token {
            bytes: start..slash,
            kind: TokenKind::Type,
            modifiers: 0,
        });
        output.push(Token {
            bytes: slash..slash + 1,
            kind: TokenKind::Event,
            modifiers: 0,
        });
        start = slash + 1;
    }
    output.push(Token {
        bytes: start..end,
        kind,
        modifiers,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use parenox_analysis::Source;

    #[test]
    fn distinguishes_shadowed_locals_from_source_functions_and_macros() {
        let provider = "(ns lib) (defn render [x] x) (defmacro twice [x] x)";
        let source = "(ns demo (:require [lib :as lib]))\n(defn draw [render]\n  (let [result (lib/render render)] (lib/twice result)))";
        let mut session = Session::default();
        session.check_project(
            &[
                Source {
                    path: "lib.clj",
                    source: provider,
                },
                Source {
                    path: "demo.clj",
                    source,
                },
            ],
            None,
        );
        let result = tokens(source, &session, "demo.clj", None);
        let tokens = decoded(source, &result);
        assert!(tokens.contains(&("draw", "function", 1)), "{tokens:?}");
        assert!(tokens.contains(&("render", "function", 0)), "{tokens:?}");
        assert!(
            tokens
                .iter()
                .filter(|&&token| token == ("render", "variable", 0))
                .count()
                == 2,
            "{tokens:?}"
        );
        assert!(tokens.contains(&("twice", "macro", 0)), "{tokens:?}");
        assert!(tokens.contains(&("let", "macro", 0)), "{tokens:?}");
        assert!(!tokens.contains(&("draw", "variable", 0)), "{tokens:?}");
    }

    #[test]
    fn encodes_utf16_discard_lines_and_range_boundary_tokens() {
        let source = "\"🦀\" :demo/first #_ (fake\r\n :ignored) ::second :later";
        let session = Session::default();
        let result = tokens(source, &session, "demo.clj", None);
        assert_eq!(
            decoded(source, &result),
            [
                ("demo", "type", 0),
                ("/", "event", 0),
                ("first", "keyword", 0),
                ("#_ (fake", "comment", 0),
                (" :ignored)", "comment", 0),
                ("second", "keyword", 0),
                ("later", "keyword", 0),
            ]
        );
        // The byte offset includes a 4-byte symbol occupying two UTF-16 units.
        let first = result["data"].as_array().unwrap();
        assert_eq!(first[1], 6);
        let start = source.find("second").unwrap();
        let range = tokens(source, &session, "demo.clj", Some(start + 1..start + 2));
        assert_eq!(decoded(source, &range), [("second", "keyword", 0)]);
    }

    fn decoded<'a>(source: &'a str, result: &Value) -> Vec<(&'a str, &'static str, usize)> {
        let (mut line, mut column) = (0, 0);
        result["data"]
            .as_array()
            .unwrap()
            .as_chunks::<5>()
            .0
            .iter()
            .map(|entry| {
                let [row, col, length, kind, modifiers] =
                    entry.each_ref().map(|part| part.as_u64().unwrap() as usize);
                line += row;
                column = if row == 0 { column + col } else { col };
                let row = source.lines().nth(line).unwrap();
                let byte_at = |column: usize| {
                    let mut remaining = column;
                    row.char_indices()
                        .find_map(|(offset, ch)| {
                            if remaining == 0 {
                                Some(offset)
                            } else {
                                remaining -= ch.len_utf16();
                                None
                            }
                        })
                        .unwrap_or(row.len())
                };
                (
                    &row[byte_at(column)..byte_at(column + length)],
                    TYPES[kind],
                    modifiers,
                )
            })
            .collect()
    }
}
