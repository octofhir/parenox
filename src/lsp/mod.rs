//! Editor-facing LSP requests over stdin and stdout.
mod clojuredocs;
mod contract_locations;
mod extern_buffers;
mod projects;
mod workspace;

use std::collections::BTreeMap;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

use parenox_lsp::{completion, project_tree, protocol, refactor, semantic, symbols, text};
use serde::Deserialize;
use serde_json::{Value, json};

use self::clojuredocs::Docs;
use self::projects::{ClientFeatures, Projects};
use self::protocol::{read_message, write_message};
use self::text::file_path;
use self::workspace::file_rename::RenameFiles;
use self::workspace::inspection;
use crate::cli::{LspOptions, ProjectOptions};

const WATCH_REQUEST_ID: &str = "parenox/file-watch";

struct ServerOptions {
    project: ProjectOptions,
    clojuredocs: Docs,
}

#[derive(Default)]
struct ServerState {
    workspace: Option<Projects>,
    shutdown: bool,
    edits: ApplyEdits,
    inlay_refresh: InlayRefresh,
}

#[derive(Default)]
struct ApplyEdits {
    next: u64,
    pending: BTreeMap<String, (Value, Value)>,
}

#[derive(Default)]
struct InlayRefresh {
    enabled: bool,
    initialized: bool,
    pending: Option<String>,
    deferred: bool,
    next: u64,
}

impl InlayRefresh {
    fn request(&mut self, output: &mut impl Write) -> io::Result<()> {
        if !self.enabled || !self.initialized {
            return Ok(());
        }
        if self.pending.is_some() {
            self.deferred = true;
            return Ok(());
        }
        self.next += 1;
        let id = format!("parenox/inlay-hint-refresh/{}", self.next);
        write_message(
            output,
            &json!({
                "jsonrpc":"2.0", "id":id, "method":"workspace/inlayHint/refresh"
            }),
        )?;
        self.pending = Some(id);
        Ok(())
    }

    fn acknowledge(&mut self, shutdown: bool, output: &mut impl Write) -> io::Result<()> {
        self.pending = None;
        if std::mem::take(&mut self.deferred) && !shutdown {
            self.request(output)?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Message {
    jsonrpc: String,
    id: Option<Value>,
    method: Option<String>,
    error: Option<Value>,
    result: Option<Value>,
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

#[derive(Deserialize)]
struct ExecuteCommand {
    command: String,
    #[serde(default)]
    arguments: Vec<Value>,
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

pub(super) fn run(options: LspOptions) -> Result<u8, String> {
    let options = ServerOptions {
        project: options.project,
        clojuredocs: Docs::open(options.clojuredocs.as_deref(), options.clojuredocs_offline)?,
    };
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut buffer = Vec::new();
    let mut state = ServerState::default();
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
            } else if let Some(id) = message.id.as_ref().and_then(Value::as_str)
                && state.inlay_refresh.pending.as_deref() == Some(id)
            {
                state
                    .inlay_refresh
                    .acknowledge(state.shutdown, &mut output)
                    .map_err(|err| err.to_string())?;
            } else if let Some((request, edit)) = message
                .id
                .as_ref()
                .and_then(Value::as_str)
                .and_then(|id| state.edits.pending.remove(id))
            {
                let applied = message
                    .result
                    .as_ref()
                    .and_then(|response| response.get("applied"))
                    .and_then(Value::as_bool)
                    == Some(true);
                if message.error.is_none() && applied {
                    result(&mut output, request, edit)
                } else {
                    let reason = message
                        .result
                        .as_ref()
                        .and_then(|response| response.get("failureReason"))
                        .and_then(Value::as_str)
                        .unwrap_or("Client did not apply the edit");
                    send_error(&mut output, request, -32803, reason)
                }
                .map_err(|err| err.to_string())?;
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
                &mut state,
                id,
                &method,
                message.params,
            )
        } else if method == "exit" {
            return Ok(if state.shutdown { 0 } else { 1 });
        } else if !state.shutdown
            && let Some(workspace) = &mut state.workspace
        {
            notification(
                &mut output,
                workspace,
                &mut state.inlay_refresh,
                &method,
                message.params,
            )
        } else {
            Ok(())
        };
        response.map_err(|err| err.to_string())?;
    }
    Ok(0)
}

fn request(
    output: &mut impl Write,
    options: &ServerOptions,
    state: &mut ServerState,
    id: Value,
    method: &str,
    params: Value,
) -> io::Result<()> {
    let ServerState {
        workspace,
        shutdown,
        edits,
        inlay_refresh,
    } = state;
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
        inlay_refresh.enabled = init
            .capabilities
            .pointer("/workspace/inlayHint/refreshSupport")
            .and_then(Value::as_bool)
            == Some(true);
        let versioned_edits = init
            .capabilities
            .pointer("/workspace/workspaceEdit/documentChanges")
            .and_then(Value::as_bool)
            == Some(true);
        let related_information = init
            .capabilities
            .pointer("/textDocument/publishDiagnostics/relatedInformation")
            .and_then(Value::as_bool)
            == Some(true);
        let apply_edit = init
            .capabilities
            .pointer("/workspace/applyEdit")
            .and_then(Value::as_bool)
            == Some(true);
        let snippets = init
            .capabilities
            .pointer("/textDocument/completion/completionItem/snippetSupport")
            .and_then(Value::as_bool)
            == Some(true);
        let markdown_hover = init
            .capabilities
            .pointer("/textDocument/hover/contentFormat")
            .and_then(Value::as_array)
            .and_then(|formats| {
                formats.iter().find_map(|format| match format.as_str() {
                    Some("markdown") => Some(true),
                    Some("plaintext") => Some(false),
                    _ => None,
                })
            })
            .unwrap_or_default();
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
            &options.project,
            watch,
            ClientFeatures {
                apply_edit,
                versioned_edits,
                snippets,
                markdown_hover,
                hierarchical_symbols,
                folding,
                test_tree,
                related_information,
            },
        ));
        let rename_files = json!({"filters":[{
            "scheme":"file", "pattern":{"glob":"**/*.{clj,cljc}", "matches":"file"}
        }]});
        let commands = ["server-info", "cursor-info"]
            .into_iter()
            .chain(refactor::Command::ALL.map(refactor::Command::name))
            .collect::<Vec<_>>();
        return result(
            output,
            id,
            json!({"capabilities": {
            "positionEncoding":"utf-16", "textDocumentSync":{"openClose":true,"change":2,"save":{"includeText":false}},
            "definitionProvider":true, "declarationProvider":true, "implementationProvider":true,
            "hoverProvider":true, "inlayHintProvider":true, "documentFormattingProvider":true,
            "documentRangeFormattingProvider":true,
            "foldingRangeProvider":true, "selectionRangeProvider":true,
            "renameProvider":{"prepareProvider":true},"linkedEditingRangeProvider":true,
            "callHierarchyProvider":true,"semanticTokensProvider":semantic::capability(),
            "referencesProvider":true,"documentHighlightProvider":true,
            "codeLensProvider":{"resolveProvider":true},
            "documentSymbolProvider":true,"workspaceSymbolProvider":true,
            "completionProvider":{"resolveProvider":true,"triggerCharacters":[":","/","."]},
            "signatureHelpProvider":{"triggerCharacters":[" "],"retriggerCharacters":[")"]},
            "codeActionProvider":{"codeActionKinds":[refactor::REFACTOR_KIND,refactor::REWRITE_KIND,refactor::EXTRACT_KIND]},
            "executeCommandProvider":{"commands":commands},
            "workspace":{"fileOperations":{"willRename":rename_files,"didRename":rename_files}},
            "experimental":{"testTree":true,"projectTree":true,"serverInfo":true,"cursorInfo":true,"clojuredocs":true}
        }, "serverInfo":{"name":"parenox", "version":env!("CARGO_PKG_VERSION")}}),
        );
    }
    let Some(projects) = workspace else {
        return send_error(output, id, -32002, "Server not initialized");
    };
    if method == "clojure/serverInfo/raw" {
        return result(output, id, projects.for_document(None).inspect_server());
    }
    if method == "clojure/clojuredocs/raw" {
        return result(
            output,
            id,
            json!(options.clojuredocs.snapshot().raw(&params)),
        );
    }
    if method == "workspace/executeCommand" {
        let command: ExecuteCommand = match serde_json::from_value(params) {
            Ok(command) => command,
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        };
        match command.command.as_str() {
            "server-info" => inspection::log(
                output,
                "Server info",
                &projects.for_document(None).inspect_server(),
            )?,
            "cursor-info" => {
                let Some(query) = inspection::command_cursor(&command.arguments) else {
                    return send_error(output, id, -32602, "Invalid cursor-info arguments");
                };
                log_cursor(projects, query, output)?;
            }
            name => {
                let Some(refactoring) = refactor::Command::named(name) else {
                    return send_error(output, id, -32602, "Unknown command");
                };
                let Some((uri, position)) = command_position(&command.arguments) else {
                    return send_error(
                        output,
                        id,
                        -32602,
                        &format!("Invalid {} arguments", command.command),
                    );
                };
                let collection = if matches!(refactoring, refactor::Command::ChangeCollection) {
                    let Some(collection) = command
                        .arguments
                        .get(3)
                        .and_then(Value::as_str)
                        .and_then(refactor::Collection::named)
                    else {
                        return send_error(
                            output,
                            id,
                            -32602,
                            "Expected change-coll target list, map, vector or set",
                        );
                    };
                    Some(collection)
                } else {
                    None
                };
                let apply = projects.apply_edit();
                let workspace = projects.for_document_mut(Some(uri));
                workspace.ensure_index(output)?;
                let Some(edit) = workspace.refactor(uri, position, refactoring, collection) else {
                    let (code, message) = refactoring.missing_edit();
                    return send_error(output, id, code, message);
                };
                if !apply {
                    return result(output, id, edit);
                }
                return edits.send(output, id, edit);
            }
        }
        return result(output, id, Value::Null);
    }
    if method == "workspace/symbol" {
        let search: symbols::Search = match serde_json::from_value(params) {
            Ok(search) => search,
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        };
        let matches = projects.workspace_symbols(&search.query, output)?;
        return result(output, id, json!(matches));
    }
    if method == "clojure/workspace/projectTree/nodes" {
        let query: Option<project_tree::Leaf> = match serde_json::from_value(params) {
            Ok(query) => query,
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        };
        let value = projects.project_tree(query, output)?.unwrap_or(Value::Null);
        return result(output, id, value);
    }
    if method == "clojure/dependencyContents" {
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return send_error(output, id, -32602, "Expected a dependency URI");
        };
        return result(output, id, json!(projects.dependency_contents(uri)));
    }
    if method == "workspace/willRenameFiles" {
        let query: RenameFiles = match serde_json::from_value(params) {
            Ok(query) => query,
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        };
        return match projects.will_rename_files(&query.files, output)? {
            Ok(value) => result(output, id, value),
            Err(reason) => send_error(output, id, -32803, &reason),
        };
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
            | "textDocument/hover"
            | "textDocument/inlayHint"
            | "textDocument/signatureHelp"
            | "textDocument/codeAction"
            | "codeLens/resolve"
            | "completionItem/resolve"
            | "clojure/cursorInfo/raw"
    ) {
        projects.for_document_mut(document).ensure_index(output)?;
    }
    let workspace = projects.for_document(document);
    let value = match method {
        "shutdown" => {
            *shutdown = true;
            for (_, (request, _)) in std::mem::take(&mut edits.pending) {
                send_error(output, request, -32800, "Server is shut down")?;
            }
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
        "textDocument/inlayHint" => match serde_json::from_value(params) {
            Ok(params) => json!(workspace.inlay_hints(params).unwrap_or_default()),
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
            Ok(params) => workspace
                .hover(
                    params,
                    projects.markdown_hover(),
                    &options.clojuredocs.snapshot(),
                )
                .unwrap_or(Value::Null),
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
        "clojure/cursorInfo/raw" => match serde_json::from_value(params) {
            Ok(params) => workspace.inspect_cursor(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
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

impl ApplyEdits {
    fn send(&mut self, output: &mut impl Write, command: Value, edit: Value) -> io::Result<()> {
        self.next += 1;
        let id = format!("parenox/apply-edit/{}", self.next);
        write_message(
            output,
            &json!({
                "jsonrpc":"2.0","id":id,"method":"workspace/applyEdit","params":{"edit":edit}
            }),
        )?;
        self.pending.insert(id, (command, edit));
        Ok(())
    }
}

fn command_position(arguments: &[Value]) -> Option<(&str, text::Position)> {
    let [uri, line, character, ..] = arguments else {
        return None;
    };
    Some((
        uri.as_str()?,
        text::Position {
            line: usize::try_from(line.as_u64()?).ok()?,
            character: usize::try_from(character.as_u64()?).ok()?,
        },
    ))
}

fn notification(
    output: &mut impl Write,
    projects: &mut Projects,
    inlay_refresh: &mut InlayRefresh,
    method: &str,
    params: Value,
) -> io::Result<()> {
    match method {
        "initialized" => {
            projects.initialized(output)?;
            inlay_refresh.initialized = true;
        }
        "textDocument/didOpen" => {
            let uri = document_uri(&params).map(str::to_owned);
            if let Ok(params) = serde_json::from_value(params) {
                projects.track_open(uri.as_deref(), output)?;
                projects
                    .for_document_mut(uri.as_deref())
                    .open(params, output)?;
                projects.update_extern_buffer(uri.as_deref(), output)?;
            }
        }
        "textDocument/didChange" => {
            let uri = document_uri(&params).map(str::to_owned);
            if let Ok(params) = serde_json::from_value(params) {
                projects
                    .for_document_mut(uri.as_deref())
                    .change(params, output)?;
                projects.update_extern_buffer(uri.as_deref(), output)?;
            }
        }
        "textDocument/didClose" => {
            let uri = document_uri(&params).map(str::to_owned);
            if let Ok(params) = serde_json::from_value(params) {
                projects
                    .for_document_mut(uri.as_deref())
                    .close(params, output)?;
                projects.update_extern_buffer(uri.as_deref(), output)?;
                projects.track_close(uri.as_deref());
            }
        }
        "textDocument/didSave" => {
            if let Ok(params) = serde_json::from_value(params)
                && projects.save(params, output)?
            {
                inlay_refresh.request(output)?;
            }
        }
        "workspace/didChangeWatchedFiles" => {
            if let Ok(params) = serde_json::from_value(params)
                && projects.changed_files(params, output)?
            {
                inlay_refresh.request(output)?;
            }
        }
        "workspace/didRenameFiles" => {
            if let Ok(query) = serde_json::from_value::<RenameFiles>(params)
                && projects.did_rename_files(&query.files, output)?
            {
                inlay_refresh.request(output)?;
            }
        }
        "clojure/serverInfo/log" => {
            inspection::log(
                output,
                "Server info",
                &projects.for_document(None).inspect_server(),
            )?;
        }
        "clojure/cursorInfo/log" => {
            if let Ok(query) = serde_json::from_value(params) {
                log_cursor(projects, query, output)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn log_cursor(
    projects: &mut Projects,
    query: workspace::TextQuery,
    output: &mut impl Write,
) -> io::Result<()> {
    let workspace = projects.for_document_mut(Some(inspection::cursor_uri(&query)));
    workspace.ensure_index(output)?;
    let report = workspace.inspect_cursor(query).unwrap_or(Value::Null);
    inspection::log(output, "Cursor info", &report)
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
