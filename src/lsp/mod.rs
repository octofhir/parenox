//! Editor-facing LSP requests over stdin and stdout.
mod projects;
mod workspace;

use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

use parenox_lsp::{completion, protocol, semantic, symbols, text};
use serde::Deserialize;
use serde_json::{Value, json};

use self::projects::{ClientFeatures, Projects};
use self::protocol::{read_message, write_message};
use self::text::file_path;
use crate::cli::ProjectOptions;

const WATCH_REQUEST_ID: &str = "parenox/file-watch";

#[derive(Deserialize)]
struct Message {
    jsonrpc: String,
    id: Option<Value>,
    method: Option<String>,
    error: Option<Value>,
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Initialize {
    root_uri: Option<String>,
    root_path: Option<String>,
    workspace_folders: Option<Vec<Folder>>,
    #[serde(default)]
    capabilities: Value,
}

#[derive(Deserialize)]
struct Folder {
    uri: String,
}

impl Initialize {
    fn root(&self) -> Result<PathBuf, String> {
        let uri = self.root_uri.as_deref().or_else(|| {
            self.workspace_folders
                .as_ref()?
                .first()
                .map(|folder| folder.uri.as_str())
        });
        if let Some(uri) = uri {
            return file_path(uri).ok_or_else(|| "Expected a local workspace file URI".into());
        }
        let cwd = env::current_dir().map_err(|err| err.to_string())?;
        Ok(self
            .root_path
            .as_ref()
            .map_or_else(|| cwd.clone(), |path| cwd.join(path)))
    }
}

pub(super) fn run(options: ProjectOptions) -> Result<u8, String> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut buffer = Vec::new();
    let mut workspace = None;
    let mut shutdown = false;
    while read_message(&mut input, &mut buffer).map_err(|err| err.to_string())? {
        let message = match serde_json::from_slice::<Message>(&buffer) {
            Ok(message) if message.jsonrpc == "2.0" => message,
            Err(error) if error.is_syntax() || error.is_eof() => {
                send_error(&mut output, Value::Null, -32700, "Parse error")
                    .map_err(|err| err.to_string())?;
                continue;
            }
            _ => {
                send_error(&mut output, Value::Null, -32600, "Invalid request")
                    .map_err(|err| err.to_string())?;
                continue;
            }
        };
        let Some(method) = message.method else {
            if message.id.as_ref().and_then(Value::as_str) == Some(WATCH_REQUEST_ID) {
                if let Some(error) = message.error {
                    write_message(&mut output, &json!({
                        "jsonrpc":"2.0", "method":"window/logMessage", "params":{
                            "type":2, "message":format!("Could not register Parenox file watches ({error})")
                        }
                    }))
                    .map_err(|err| err.to_string())?;
                }
            } else {
                send_error(&mut output, Value::Null, -32600, "Invalid request")
                    .map_err(|err| err.to_string())?;
            }
            continue;
        };
        let response = if let Some(id) = message.id {
            request(
                &mut output,
                &options,
                &mut workspace,
                &mut shutdown,
                id,
                &method,
                message.params,
            )
        } else if method == "exit" {
            return Ok(if shutdown { 0 } else { 1 });
        } else if !shutdown && let Some(workspace) = &mut workspace {
            notification(&mut output, workspace, &method, message.params)
        } else {
            Ok(())
        };
        response.map_err(|err| err.to_string())?;
    }
    Ok(0)
}

fn request(
    output: &mut impl Write,
    options: &ProjectOptions,
    workspace: &mut Option<Projects>,
    shutdown: &mut bool,
    id: Value,
    method: &str,
    params: Value,
) -> io::Result<()> {
    if *shutdown {
        return send_error(output, id, -32600, "Server is shut down");
    }
    if method == "initialize" {
        if workspace.is_some() {
            return send_error(output, id, -32600, "Server is already initialized");
        }
        let init: Initialize = match serde_json::from_value(params) {
            Ok(init) => init,
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        };
        let root = match init.root() {
            Ok(root) => root,
            Err(message) => return send_error(output, id, -32602, &message),
        };
        let watch = init
            .capabilities
            .pointer("/workspace/didChangeWatchedFiles/dynamicRegistration")
            .and_then(Value::as_bool)
            == Some(true);
        let versioned_edits = init
            .capabilities
            .pointer("/workspace/workspaceEdit/documentChanges")
            .and_then(Value::as_bool)
            == Some(true);
        let snippets = init
            .capabilities
            .pointer("/textDocument/completion/completionItem/snippetSupport")
            .and_then(Value::as_bool)
            == Some(true);
        let hierarchical_symbols = init
            .capabilities
            .pointer("/textDocument/documentSymbol/hierarchicalDocumentSymbolSupport")
            .and_then(Value::as_bool)
            == Some(true);
        let folding = init
            .capabilities
            .pointer("/textDocument/foldingRange")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default();
        let test_tree = init
            .capabilities
            .pointer("/experimental/testTree")
            .and_then(Value::as_bool)
            == Some(true);
        *workspace = Some(Projects::load(
            root,
            options,
            watch,
            ClientFeatures {
                versioned_edits,
                snippets,
                hierarchical_symbols,
                folding,
                test_tree,
            },
        ));
        return result(
            output,
            id,
            json!({"capabilities": {
            "positionEncoding":"utf-16", "textDocumentSync":{"openClose":true,"change":2,"save":{"includeText":false}},
            "definitionProvider":true, "declarationProvider":true, "implementationProvider":true,
            "hoverProvider":true, "documentFormattingProvider":true,
            "documentRangeFormattingProvider":true,
            "foldingRangeProvider":true, "selectionRangeProvider":true,
            "renameProvider":{"prepareProvider":true},"linkedEditingRangeProvider":true,
            "callHierarchyProvider":true,"semanticTokensProvider":semantic::capability(),
            "referencesProvider":true,"documentHighlightProvider":true,
            "codeLensProvider":{"resolveProvider":true},
            "documentSymbolProvider":true,"workspaceSymbolProvider":true,
            "completionProvider":{"resolveProvider":true,"triggerCharacters":[":","/"]},
            "signatureHelpProvider":{"triggerCharacters":[" "],"retriggerCharacters":[")"]},
            "codeActionProvider":{"codeActionKinds":["refactor.rewrite"]},
            "experimental":{"testTree":true}
        }, "serverInfo":{"name":"parenox", "version":env!("CARGO_PKG_VERSION")}}),
        );
    }
    let Some(projects) = workspace else {
        return send_error(output, id, -32002, "Server not initialized");
    };
    if method == "workspace/symbol" {
        let search: symbols::Search = match serde_json::from_value(params) {
            Ok(search) => search,
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        };
        let matches = projects.workspace_symbols(&search.query, output)?;
        return result(output, id, json!(matches));
    }
    let document = document_uri(&params);
    if matches!(
        method,
        "textDocument/foldingRange"
            | "textDocument/declaration"
            | "textDocument/implementation"
            | "textDocument/prepareRename"
            | "textDocument/rename"
            | "textDocument/linkedEditingRange"
            | "textDocument/prepareCallHierarchy"
            | "callHierarchy/incomingCalls"
            | "callHierarchy/outgoingCalls"
            | "textDocument/semanticTokens/full"
            | "textDocument/semanticTokens/range"
            | "textDocument/codeLens"
            | "codeLens/resolve"
            | "completionItem/resolve"
    ) {
        projects.for_document_mut(document).ensure_index(output)?;
    }
    let workspace = projects.for_document(document);
    let value = match method {
        "shutdown" => {
            *shutdown = true;
            Value::Null
        }
        "textDocument/definition" => match serde_json::from_value(params) {
            Ok(params) => workspace.definition(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/declaration" => match serde_json::from_value(params) {
            Ok(params) => workspace.declaration(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/implementation" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.implementations(params).unwrap_or_default()),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/references" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.references(params)),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/documentHighlight" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.highlights(params)),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/documentSymbol" => match serde_json::from_value(params) {
            Ok(params) => {
                json!(workspace.document_symbols(params, projects.hierarchical_symbols()))
            }
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/foldingRange" => match serde_json::from_value(params) {
            Ok(params) => json!(
                workspace
                    .folding_ranges(params, projects.folding())
                    .unwrap_or_default()
            ),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/selectionRange" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.selection_ranges(params).unwrap_or_default()),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/prepareRename" => match serde_json::from_value(params) {
            Ok(params) => workspace.prepare_rename(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/rename" => match serde_json::from_value(params) {
            Ok(params) => match workspace.rename(params) {
                Ok(edit) => edit,
                Err(error) => return send_error(output, id, -32602, &error),
            },
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/linkedEditingRange" => match serde_json::from_value(params) {
            Ok(params) => workspace.linked_editing(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/prepareCallHierarchy" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.prepare_call_hierarchy(params)),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "callHierarchy/incomingCalls" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.incoming_calls(params)),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "callHierarchy/outgoingCalls" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.outgoing_calls(params)),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/semanticTokens/full" => match serde_json::from_value(params) {
            Ok(params) => workspace
                .semantic_tokens_full(params)
                .unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/semanticTokens/range" => match serde_json::from_value(params) {
            Ok(params) => workspace
                .semantic_tokens_range(params)
                .unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/codeLens" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.code_lenses(params).unwrap_or_default()),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "codeLens/resolve" => match serde_json::from_value(params.clone()) {
            Ok(query) => workspace.resolve_code_lens(query).unwrap_or(params),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/hover" => match serde_json::from_value(params) {
            Ok(params) => workspace.hover(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/signatureHelp" => match serde_json::from_value(params) {
            Ok(params) => workspace.signature_help(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/completion" => match serde_json::from_value(params) {
            Ok(params) => workspace
                .completion(params, projects.snippets())
                .unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "completionItem/resolve" => workspace.resolve_completion(params),
        "textDocument/codeAction" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.code_actions(params)),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/formatting" => match serde_json::from_value(params) {
            Ok(params) => workspace
                .format_document(params)
                .unwrap_or_else(|| json!([])),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/rangeFormatting" => match serde_json::from_value(params) {
            Ok(params) => workspace.format_range(params).unwrap_or_else(|| json!([])),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        _ => return send_error(output, id, -32601, "Method not found"),
    };
    result(output, id, value)
}

fn notification(
    output: &mut impl Write,
    projects: &mut Projects,
    method: &str,
    params: Value,
) -> io::Result<()> {
    match method {
        "initialized" => projects.initialized(output)?,
        "textDocument/didOpen" => {
            let uri = document_uri(&params).map(str::to_owned);
            if let Ok(params) = serde_json::from_value(params) {
                projects.track_open(uri.as_deref(), output)?;
                projects
                    .for_document_mut(uri.as_deref())
                    .open(params, output)?;
            }
        }
        "textDocument/didChange" => {
            let workspace = projects.for_document_mut(document_uri(&params));
            if let Ok(params) = serde_json::from_value(params) {
                workspace.change(params, output)?;
            }
        }
        "textDocument/didClose" => {
            let uri = document_uri(&params).map(str::to_owned);
            if let Ok(params) = serde_json::from_value(params) {
                projects
                    .for_document_mut(uri.as_deref())
                    .close(params, output)?;
                projects.track_close(uri.as_deref());
            }
        }
        "textDocument/didSave" => {
            if let Ok(params) = serde_json::from_value(params) {
                projects.save(params, output)?;
            }
        }
        "workspace/didChangeWatchedFiles" => {
            if let Ok(params) = serde_json::from_value(params) {
                projects.changed_files(params, output)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn document_uri(params: &Value) -> Option<&str> {
    params
        .pointer("/textDocument/uri")
        .or_else(|| params.pointer("/item/uri"))
        .or_else(|| params.pointer("/data/parenoxSource/uri"))
        .or_else(|| params.pointer("/data/0"))?
        .as_str()
}

fn result(output: &mut impl Write, id: Value, value: Value) -> io::Result<()> {
    write_message(output, &json!({"jsonrpc":"2.0","id":id,"result":value}))
}

fn send_error(output: &mut impl Write, id: Value, code: i32, message: &str) -> io::Result<()> {
    write_message(
        output,
        &json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}),
    )
}
