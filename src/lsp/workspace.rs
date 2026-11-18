//! One workspace and its in-memory editor overlays.
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use parenox::{Session, Source, output::contract_note};
use serde::Deserialize;
use serde_json::{Value, json};

use super::protocol::write_message;
use super::text::{Lines, Position, file_path, file_uri, normalize};
use crate::cli::ProjectOptions;
use crate::serve::Snapshot;
use crate::{is_clojure_source, read_externs, read_source_roots};

pub(super) struct Workspace {
    documents: BTreeMap<String, Document>,
    externs: Vec<Snapshot>,
    session: Session,
}

struct Document {
    source: String,
    indexed: bool,
    editor: Option<Editor>,
}

struct Editor {
    uri: String,
    version: i64,
    published: Option<Vec<Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Open {
    text_document: OpenDocument,
}

#[derive(Deserialize)]
struct OpenDocument {
    uri: String,
    version: i64,
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Change {
    text_document: VersionedUri,
    content_changes: Vec<FullChange>,
}

#[derive(Deserialize)]
struct VersionedUri {
    uri: String,
    version: i64,
}

#[derive(Deserialize)]
struct FullChange {
    text: String,
    range: Option<Value>,
}

#[derive(Deserialize)]
struct DocumentUri {
    uri: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Close {
    text_document: DocumentUri,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TextQuery {
    text_document: DocumentUri,
    position: Position,
}

impl Workspace {
    pub fn load(root: &Path, options: &ProjectOptions) -> Result<Self, String> {
        let mut config = options.load_at(root)?;
        config.externs.append(&mut config.dependency_externs);
        let externs = read_externs(&config.externs)?;
        let documents = read_source_roots(&config.paths)?
            .into_iter()
            .map(|(path, source)| {
                let path = normalize(Path::new(&path)).to_string_lossy().into_owned();
                (
                    path,
                    Document {
                        source,
                        indexed: true,
                        editor: None,
                    },
                )
            })
            .collect();
        Ok(Self {
            documents,
            externs,
            session: Session::default(),
        })
    }

    pub fn open(&mut self, params: Open, output: &mut impl Write) -> io::Result<()> {
        let OpenDocument { uri, version, text } = params.text_document;
        let Some(path) = source_path(&uri) else {
            return Ok(());
        };
        let indexed = self
            .documents
            .get(&path)
            .is_some_and(|document| document.indexed);
        self.documents.insert(
            path,
            Document {
                source: text,
                indexed,
                editor: Some(Editor {
                    uri,
                    version,
                    published: None,
                }),
            },
        );
        self.check(output)
    }

    pub fn change(&mut self, params: Change, output: &mut impl Write) -> io::Result<()> {
        let VersionedUri { uri, version } = params.text_document;
        let Some(path) = source_path(&uri) else {
            return Ok(());
        };
        let Some(document) = self.documents.get_mut(&path) else {
            return Ok(());
        };
        let Some(editor) = &mut document.editor else {
            return Ok(());
        };
        if version <= editor.version || params.content_changes.len() != 1 {
            return Ok(());
        }
        let change = params.content_changes.into_iter().next().unwrap();
        if change.range.is_some() {
            return Ok(());
        }
        document.source = change.text;
        editor.version = version;
        editor.published = None;
        self.check(output)
    }

    pub fn close(&mut self, params: Close, output: &mut impl Write) -> io::Result<()> {
        let Some(path) = source_path(&params.text_document.uri) else {
            return Ok(());
        };
        let Some(document) = self.documents.get_mut(&path) else {
            return Ok(());
        };
        let Some(editor) = document.editor.take() else {
            return Ok(());
        };
        if document.indexed
            && let Ok(source) = fs::read_to_string(&path)
        {
            document.source = source;
        } else {
            self.documents.remove(&path);
            self.session.forget_file(&path);
        }
        publish(output, json!({"uri":editor.uri, "diagnostics":[]}))?;
        self.check(output)
    }

    pub fn definition(&self, params: TextQuery) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let target = self.session.definition(&path, offset)?;
        let document = self.documents.get(&target.path)?;
        let range = Lines::new(&document.source).range(target.span)?;
        let uri = document
            .editor
            .as_ref()
            .map_or_else(|| file_uri(&target.path), |editor| editor.uri.clone());
        Some(json!({"uri": uri, "range": range}))
    }

    pub fn hover(&self, params: TextQuery) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let info = self.session.var_at(&path, offset)?;
        let mut lines = vec![info.name];
        lines.extend(info.signatures);
        if let Some(contract) = &info.contract {
            lines.push(contract_note(contract));
        } else {
            lines.push("Checked type information unavailable".into());
        }
        Some(json!({"contents":{"kind":"plaintext","value":lines.join("\n")}}))
    }

    fn query_offset(&self, params: TextQuery) -> Option<(String, usize)> {
        let path = source_path(&params.text_document.uri)?;
        let document = self.documents.get(&path)?;
        let offset = Lines::new(&document.source).offset(params.position)?;
        Some((path, offset))
    }

    fn check(&mut self, output: &mut impl Write) -> io::Result<()> {
        let sources: Vec<_> = self
            .documents
            .iter()
            .map(|(path, doc)| Source {
                path,
                source: &doc.source,
            })
            .collect();
        let externs: Vec<_> = self.externs.iter().map(Snapshot::as_source).collect();
        let report = self.session.check_project_with_externs(&sources, &externs);
        for file in report.files {
            let Some(document) = self.documents.get_mut(&file.path) else {
                continue;
            };
            let Some(editor) = &mut document.editor else {
                continue;
            };
            let diagnostics = if file.diagnostics.is_empty() {
                Vec::new()
            } else {
                let lines = Lines::new(&document.source);
                file.diagnostics.into_iter().filter_map(|diagnostic| {
                    let range = lines.range(diagnostic.span)?;
                    let severity = if diagnostic.severity == "warning" { 2 } else { 1 };
                    Some(json!({"range":range,"severity":severity,"source":"parenox","code":diagnostic.code,"message":diagnostic.message}))
                }).collect()
            };
            if editor.published.as_ref() != Some(&diagnostics) {
                publish(
                    output,
                    json!({"uri":editor.uri,"version":editor.version,"diagnostics":diagnostics}),
                )?;
                editor.published = Some(diagnostics);
            }
        }
        Ok(())
    }
}

fn source_path(uri: &str) -> Option<String> {
    let path = file_path(uri)?;
    is_clojure_source(&path).then(|| path.to_string_lossy().into_owned())
}

fn publish(output: &mut impl Write, params: Value) -> io::Result<()> {
    write_message(
        output,
        &json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":params}),
    )
}
