//! Inspect the current workspace and source analysis over the editor protocol.
use std::io::{self, Write};

use parenox::reader::{self, Kind, Span};
use parenox::syntax::{SyntaxElement, SyntaxKind};
use parenox::{InspectedReference, SemanticSymbol, SemanticSymbolKind, SourceSymbolKind};
use serde_json::{Value, json};

use super::{DocumentUri, Lines, Position, TextQuery, Workspace, file_uri, write_message};

impl Workspace {
    pub(in crate::lsp) fn inspect_server(&self) -> Value {
        let roots: Vec<_> = self
            .editable_roots
            .iter()
            .map(|root| root.to_string_lossy())
            .collect();
        let classpath: Vec<_> = self
            .source_roots
            .iter()
            .chain(self.dependency_completions.jars())
            .map(|path| path.to_string_lossy())
            .collect();
        let symbols = self
            .session_ready
            .then(|| self.session.project_symbols().count());
        json!({
            "project-root-uri": file_uri(&self.root.to_string_lossy()),
            "server-version": env!("CARGO_PKG_VERSION"),
            "classpath": classpath,
            "final-settings": {
                "source-paths": roots,
                "source-aliases": self.options.aliases,
            },
            "analysis-summary": {
                "internal": {
                    "indexed-files": self.documents.values().filter(|document| document.indexed).count(),
                    "source-symbols": symbols,
                },
                "open-documents": self.documents.values().filter(|document| document.editor.is_some()).count(),
                "session-ready": self.session_ready,
            },
        })
    }

    pub(in crate::lsp) fn inspect_cursor(&self, params: TextQuery) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let document = self.documents.get(&path)?;
        let lines = Lines::new(&document.source);
        let offset = u32::try_from(offset).ok()?.into();
        let parsed = reader::parse(&document.source);
        let root = parsed.syntax();
        let token = root
            .token_at_offset(offset)
            .right_biased()
            .filter(|token| token.text_range().contains(offset));
        let Some(token) = token else {
            return Some(json!({"elements": []}));
        };
        let start = usize::from(token.text_range().start());
        let end = usize::from(token.text_range().end());
        let node = match token.kind() {
            SyntaxKind::OpenParen
            | SyntaxKind::CloseParen
            | SyntaxKind::OpenBracket
            | SyntaxKind::CloseBracket
            | SyntaxKind::OpenBrace
            | SyntaxKind::CloseBrace
            | SyntaxKind::OpenSet
            | SyntaxKind::OpenAnonymousFunction => {
                let parent = token.parent()?;
                let children_count = parent
                    .children_with_tokens()
                    .filter(|child| match child {
                        SyntaxElement::Node(_) => true,
                        SyntaxElement::Token(token) => !matches!(
                            token.kind(),
                            SyntaxKind::OpenParen
                                | SyntaxKind::CloseParen
                                | SyntaxKind::OpenBracket
                                | SyntaxKind::CloseBracket
                                | SyntaxKind::OpenBrace
                                | SyntaxKind::CloseBrace
                                | SyntaxKind::OpenSet
                                | SyntaxKind::OpenAnonymousFunction
                        ),
                    })
                    .count();
                json!({"kind": format!("{:?}",parent.kind()), "children-count": children_count})
            }
            kind => token_node(kind, token.text()),
        };
        let elements: Vec<_> = if matches!(token.kind(), SyntaxKind::Symbol | SyntaxKind::Keyword) {
            self.inspect_element(&path, &lines, start, end)
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        Some(json!({"node": node, "elements": elements}))
    }

    fn inspect_element(
        &self,
        path: &str,
        lines: &Lines<'_>,
        start: usize,
        end: usize,
    ) -> Option<Value> {
        let document = self.documents.get(path)?;
        let uri = document.uri(path);
        let session = self.session_for(path)?;
        let outline = session.document_symbols(path);
        let caller = outline.as_ref().map(|(namespace, _)| *namespace);
        let semantic = session
            .semantic_symbols(path)
            .into_iter()
            .find(|symbol| symbol.span.start.offset == start && symbol.span.end.offset == end);

        if let Some((namespace, symbols)) = outline
            && let Some(symbol) = symbols.into_iter().find(|symbol| {
                symbol.selection.start.offset == start && symbol.selection.end.offset == end
            })
        {
            let bucket = match symbol.kind {
                SourceSymbolKind::Namespace => "namespace-definitions",
                SourceSymbolKind::Spec => "keyword-definitions",
                _ => "var-definitions",
            };
            let mut element = entry(bucket, &uri, symbol.name, lines, start, end, symbol.range)?;
            if bucket == "var-definitions" {
                element["ns"] = json!(namespace);
            }
            return Some(with_semantic(element.clone(), Some(element), semantic));
        }
        let reference = session.inspect_reference(path, start)?;
        let (target_path, span, usage_bucket, definition_bucket, var) = match reference {
            InspectedReference::Local(span) => (path, span, "local-usages", "locals", None),
            InspectedReference::Spec { path, span } => {
                (path, span, "keyword-usages", "keyword-definitions", None)
            }
            InspectedReference::Var {
                path,
                span,
                namespace,
                name,
            } => (
                path,
                span,
                "var-usages",
                "var-definitions",
                Some((namespace, name)),
            ),
            InspectedReference::Core { name, kind } => {
                let range = coordinates(lines, start, end)?;
                let mut element = entry_with_ranges("var-usages", &uri, name, range, range);
                element["to"] = json!("clojure.core");
                if let Some(caller) = caller {
                    element["from"] = json!(caller);
                }
                return Some(json!({
                    "element": element,
                    "semantic-tokens": [semantic_token(kind, false, false)],
                }));
            }
        };
        let target = self.documents.get(target_path)?;
        let target_lines = Lines::new(&target.source);
        let target_uri = target.uri(target_path);
        let def_start = span.start.offset;
        let def_end = span.end.offset;
        let target_name = target.source.get(def_start..def_end)?;
        let target_symbol = var.and_then(|_| {
            self.session_for(target_path)?
                .document_symbols(target_path)?
                .1
                .into_iter()
                .find(|symbol| symbol.selection == span)
        });
        let mut defined = entry(
            definition_bucket,
            &target_uri,
            target_name,
            &target_lines,
            def_start,
            def_end,
            target_symbol.map_or(span, |symbol| symbol.range),
        )?;
        if let Some((namespace, _)) = var {
            defined["ns"] = json!(namespace);
            if let Some(info) = session.var_at(path, start)
                && let Some(contract) = &info.contract
            {
                defined["parenox-contract"] =
                    json!({"origin":contract, "signatures":info.signatures});
            }
        }
        let mut element = if path == target_path && start == def_start && end == def_end {
            defined.clone()
        } else {
            let range = coordinates(lines, start, end)?;
            entry_with_ranges(usage_bucket, &uri, target_name, range, range)
        };
        if let Some((namespace, name)) = var {
            element["to"] = json!(namespace);
            element["name"] = json!(name);
            if let Some(caller) = caller {
                element["from"] = json!(caller);
            }
        }
        Some(with_semantic(element, Some(defined), semantic))
    }
}

pub(in crate::lsp) fn command_cursor(arguments: &[Value]) -> Option<TextQuery> {
    let [uri, line, character, row, col] = arguments else {
        return None;
    };
    line.as_u64()?;
    character.as_u64()?;
    let line = usize::try_from(row.as_u64()?.checked_sub(1)?).ok()?;
    let character = usize::try_from(col.as_u64()?.checked_sub(1)?).ok()?;
    Some(TextQuery {
        text_document: DocumentUri {
            uri: uri.as_str()?.to_owned(),
        },
        position: Position { line, character },
    })
}

pub(in crate::lsp) fn cursor_uri(params: &TextQuery) -> &str {
    &params.text_document.uri
}

pub(in crate::lsp) fn log(output: &mut impl Write, title: &str, report: &Value) -> io::Result<()> {
    write_message(
        output,
        &json!({
            "jsonrpc": "2.0",
            "method": "window/showMessage",
            "params": {"type": 3, "message": format!("{title}\n{report:#}")},
        }),
    )
}

fn token_node(kind: SyntaxKind, spelling: &str) -> Value {
    match kind {
        SyntaxKind::Symbol => json!({
            "value": spelling,
            "string-value": spelling,
            "map-qualifier": null,
        }),
        SyntaxKind::Keyword => {
            let name = spelling.trim_start_matches(':');
            json!({"k":name, "auto-resolved?":spelling.starts_with("::"), "map-qualifier":null})
        }
        SyntaxKind::Whitespace if spelling.contains(['\r', '\n']) => {
            json!({"newlines": spelling})
        }
        SyntaxKind::Whitespace => json!({"whitespace": spelling}),
        SyntaxKind::Comment => json!({"prefix": ";", "s": spelling.strip_prefix(';')}),
        SyntaxKind::String => match reader::read(spelling).ok().as_deref() {
            Some([form]) if let Kind::String(content) = &form.kind => {
                json!({"lines": content.split('\n').collect::<Vec<_>>()})
            }
            _ => json!({"kind": "String", "text": spelling}),
        },
        _ => json!({"kind": format!("{kind:?}"), "text": spelling}),
    }
}

fn coordinates(lines: &Lines<'_>, start: usize, end: usize) -> Option<(Position, Position)> {
    Some((lines.position(start)?, lines.position(end)?))
}

fn entry(
    bucket: &str,
    uri: &str,
    name: &str,
    lines: &Lines<'_>,
    start: usize,
    end: usize,
    parent: Span,
) -> Option<Value> {
    Some(entry_with_ranges(
        bucket,
        uri,
        name,
        coordinates(lines, start, end)?,
        coordinates(lines, parent.start.offset, parent.end.offset)?,
    ))
}

fn entry_with_ranges(
    bucket: &str,
    uri: &str,
    name: &str,
    (start, end): (Position, Position),
    (parent, parent_end): (Position, Position),
) -> Value {
    json!({
        "bucket": bucket, "name": name, "uri": uri,
        "row": parent.line + 1, "col": parent.character + 1,
        "end-row": parent_end.line + 1, "end-col": parent_end.character + 1,
        "name-row": start.line + 1, "name-col": start.character + 1,
        "name-end-row": end.line + 1, "name-end-col": end.character + 1,
    })
}

fn with_semantic(
    element: Value,
    definition: Option<Value>,
    semantic: Option<SemanticSymbol>,
) -> Value {
    let mut report = json!({"element": element});
    if let Some(definition) = definition {
        report["definition"] = definition;
    }
    if let Some(symbol) = semantic {
        report["semantic-tokens"] = json!([semantic_token(
            symbol.kind,
            symbol.definition,
            symbol.default_library,
        )]);
    }
    report
}

fn semantic_token(kind: SemanticSymbolKind, definition: bool, default_library: bool) -> Value {
    let kind = match kind {
        SemanticSymbolKind::Namespace => "namespace",
        SemanticSymbolKind::Type => "type",
        SemanticSymbolKind::Function => "function",
        SemanticSymbolKind::Macro => "macro",
        SemanticSymbolKind::Class => "class",
        SemanticSymbolKind::Variable => "variable",
        SemanticSymbolKind::Method => "method",
        SemanticSymbolKind::Interface => "interface",
    };
    let mut modifiers = Vec::new();
    if definition {
        modifiers.push("definition");
    }
    if default_library {
        modifiers.push("defaultLibrary");
    }
    json!({"token-type": kind, "token-modifier": modifiers})
}
