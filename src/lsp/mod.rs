//! Editor-facing LSP requests over stdin and stdout.
mod protocol;
mod text;
mod workspace;

use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Value, json};

use self::protocol::{read_message, write_message};
use self::text::file_path;
use self::workspace::Workspace;
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
    workspace: &mut Option<Workspace>,
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
        *workspace = Some(Workspace::load(root, options, watch));
        return result(
            output,
            id,
            json!({"capabilities": {
            "positionEncoding":"utf-16", "textDocumentSync":{"openClose":true,"change":2,"save":{"includeText":false}},
            "definitionProvider":true, "hoverProvider":true
        }, "serverInfo":{"name":"parenox", "version":env!("CARGO_PKG_VERSION")}}),
        );
    }
    let Some(workspace) = workspace else {
        return send_error(output, id, -32002, "Server not initialized");
    };
    let value = match method {
        "shutdown" => {
            *shutdown = true;
            Value::Null
        }
        "textDocument/definition" => match serde_json::from_value(params) {
            Ok(params) => workspace.definition(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        "textDocument/hover" => match serde_json::from_value(params) {
            Ok(params) => workspace.hover(params).unwrap_or(Value::Null),
            Err(error) => return send_error(output, id, -32602, &error.to_string()),
        },
        _ => return send_error(output, id, -32601, "Method not found"),
    };
    result(output, id, value)
}

fn notification(
    output: &mut impl Write,
    workspace: &mut Workspace,
    method: &str,
    params: Value,
) -> io::Result<()> {
    match method {
        "initialized" => workspace.initialized(output)?,
        "textDocument/didOpen" => {
            if let Ok(params) = serde_json::from_value(params) {
                workspace.open(params, output)?;
            }
        }
        "textDocument/didChange" => {
            if let Ok(params) = serde_json::from_value(params) {
                workspace.change(params, output)?;
            }
        }
        "textDocument/didClose" => {
            if let Ok(params) = serde_json::from_value(params) {
                workspace.close(params, output)?;
            }
        }
        "textDocument/didSave" => {
            if let Ok(params) = serde_json::from_value(params) {
                workspace.save(params, output)?;
            }
        }
        "workspace/didChangeWatchedFiles" => {
            if let Ok(params) = serde_json::from_value(params) {
                workspace.changed_files(params, output)?;
            }
        }
        _ => {}
    }
    Ok(())
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
