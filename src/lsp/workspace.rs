//! One workspace and its in-memory editor overlays.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use parenox::{Diagnostic, Session, Source, output::contract_note, reader::validate_edn};
use serde::Deserialize;
use serde_json::{Value, json};

use super::WATCH_REQUEST_ID;
use super::protocol::write_message;
use super::text::{Lines, Position, file_path, file_uri, normalize};
use crate::cli::ProjectOptions;
use crate::workspace::{DiskProject, FileSnapshot, is_clojure_source};

pub(super) struct Workspace {
    root: PathBuf,
    options: ProjectOptions,
    source_roots: Vec<PathBuf>,
    register_watches: bool,
    documents: BTreeMap<String, Document>,
    externs: Vec<FileSnapshot>,
    session: Session,
    load_error: Option<String>,
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
pub(super) struct DocumentParams {
    text_document: DocumentUri,
}

#[derive(Deserialize)]
pub(super) struct WatchedFiles {
    changes: Vec<FileEvent>,
}

#[derive(Deserialize)]
struct FileEvent {
    uri: String,
    #[serde(rename = "type")]
    kind: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TextQuery {
    text_document: DocumentUri,
    position: Position,
}

impl Workspace {
    pub fn load(root: PathBuf, options: &ProjectOptions, register_watches: bool) -> Self {
        let mut workspace = Self {
            root,
            options: options.clone(),
            source_roots: Vec::new(),
            register_watches,
            documents: BTreeMap::new(),
            externs: Vec::new(),
            session: Session::default(),
            load_error: None,
        };
        workspace.load_error = workspace.reload().err();
        workspace
    }

    pub fn initialized(&mut self, output: &mut impl Write) -> io::Result<()> {
        if std::mem::take(&mut self.register_watches) {
            write_message(
                output,
                &json!({
                    "jsonrpc":"2.0", "id":WATCH_REQUEST_ID, "method":"client/registerCapability",
                    "params":{"registrations":[{
                        "id":WATCH_REQUEST_ID, "method":"workspace/didChangeWatchedFiles",
                        "registerOptions":{"watchers":[
                            {"globPattern":"**/*.clj"}, {"globPattern":"**/*.cljc"}, {"globPattern":"**/*.edn"}
                        ]}
                    }]}
                }),
            )?;
        }
        if let Some(message) = self.load_error.take() {
            log_load_error(output, &message)?;
        }
        Ok(())
    }

    fn reload(&mut self) -> Result<(), String> {
        let settings = self.options.load_at(&self.root)?;
        let source_roots = settings
            .paths
            .iter()
            .map(|path| normalize(Path::new(path)))
            .collect();
        let mut project = DiskProject::read(settings)?;
        project.externs.append(&mut project.dependency_externs);
        let mut documents: BTreeMap<_, _> = project
            .files
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
        for (path, mut document) in std::mem::take(&mut self.documents) {
            if document.editor.is_some() {
                document.indexed = documents.contains_key(&path);
                documents.insert(path, document);
            } else if !documents.contains_key(&path) {
                self.session.forget_file(&path);
            }
        }
        for externs in &self.externs {
            if !project.externs.iter().any(|file| file.path == externs.path) {
                self.session.forget_file(&externs.path);
            }
        }
        self.documents = documents;
        self.source_roots = source_roots;
        self.externs = project.externs;
        Ok(())
    }

    pub fn open(&mut self, params: Open, output: &mut impl Write) -> io::Result<()> {
        let OpenDocument { uri, version, text } = params.text_document;
        let Some(path) = document_path(&uri) else {
            return Ok(());
        };
        let indexed = self
            .documents
            .get(&path)
            .is_some_and(|document| document.indexed);
        self.documents.insert(
            path.clone(),
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
        self.check_document(&path, output)
    }

    pub fn change(&mut self, params: Change, output: &mut impl Write) -> io::Result<()> {
        let VersionedUri { uri, version } = params.text_document;
        let Some(path) = document_path(&uri) else {
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
        self.check_document(&path, output)
    }

    pub fn save(&mut self, params: DocumentParams, output: &mut impl Write) -> io::Result<()> {
        let Some(file) = file_path(&params.text_document.uri) else {
            return Ok(());
        };
        let new_source = is_clojure_source(&file)
            && self
                .documents
                .get(file.to_string_lossy().as_ref())
                .is_some_and(|doc| !doc.indexed && doc.editor.is_some());
        if new_source || self.is_project_input(&file) {
            self.refresh(output)?;
        }
        Ok(())
    }

    pub fn changed_files(
        &mut self,
        params: WatchedFiles,
        output: &mut impl Write,
    ) -> io::Result<()> {
        for event in params.changes {
            let Some(path) = file_path(&event.uri) else {
                continue;
            };
            if matches!(event.kind, 1..=3)
                && (self.is_project_input(&path) || self.is_changed_source(&path, event.kind))
            {
                return self.refresh(output);
            }
        }
        Ok(())
    }

    fn is_changed_source(&self, path: &Path, kind: u8) -> bool {
        if !is_clojure_source(path) {
            return false;
        }
        if let Some(document) = self
            .documents
            .get(path.to_string_lossy().as_ref())
            .filter(|doc| doc.indexed)
        {
            // A change to an open document arrives through its editor overlay.
            return kind != 2 || document.editor.is_none();
        }
        // A newly created source can also create a previously absent manifest root.
        let workspace = (kind == 1).then_some(&self.root);
        self.source_roots.iter().chain(workspace).any(|root| {
            path.strip_prefix(root).is_ok_and(|tail| {
                !tail
                    .components()
                    .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
            })
        })
    }

    fn is_project_input(&self, file: &Path) -> bool {
        self.options
            .manifests(&self.root)
            .iter()
            .any(|manifest| normalize(manifest) == file)
            || self
                .options
                .classpath
                .as_ref()
                .is_some_and(|path| normalize(&self.root.join(path)) == file)
            || self
                .externs
                .iter()
                .any(|snapshot| normalize(Path::new(&snapshot.path)) == file)
    }

    fn refresh(&mut self, output: &mut impl Write) -> io::Result<()> {
        match self.reload() {
            Ok(()) => self.check(output),
            Err(message) => log_load_error(output, &message),
        }
    }

    pub fn close(&mut self, params: DocumentParams, output: &mut impl Write) -> io::Result<()> {
        let Some(path) = document_path(&params.text_document.uri) else {
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
        if is_edn(Path::new(&path)) {
            Ok(())
        } else {
            self.check(output)
        }
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
        let path = document_path(&params.text_document.uri)?;
        if !is_clojure_source(Path::new(&path)) {
            return None;
        }
        let document = self.documents.get(&path)?;
        let offset = Lines::new(&document.source).offset(params.position)?;
        Some((path, offset))
    }

    fn check_document(&mut self, path: &str, output: &mut impl Write) -> io::Result<()> {
        if is_edn(Path::new(path)) {
            let document = self.documents.get_mut(path).unwrap();
            let diagnostic = validate_edn(&document.source).err().map(Diagnostic::from);
            document.publish(diagnostic.as_slice(), output)
        } else {
            self.check(output)
        }
    }

    fn check(&mut self, output: &mut impl Write) -> io::Result<()> {
        let mut sources = Vec::new();
        let mut open = BTreeSet::new();
        for (path, doc) in &self.documents {
            if !is_clojure_source(Path::new(path)) {
                continue;
            }
            sources.push(Source {
                path,
                source: &doc.source,
            });
            if doc.editor.is_some() {
                open.insert(path.as_str());
            }
        }
        let externs: Vec<_> = self.externs.iter().map(FileSnapshot::as_source).collect();
        let report = self
            .session
            .check_project_documents(&sources, &externs, &open);
        for file in report.files {
            if let Some(document) = self.documents.get_mut(&file.path) {
                document.publish(&file.diagnostics, output)?;
            }
        }
        Ok(())
    }
}

impl Document {
    fn publish(&mut self, diagnostics: &[Diagnostic], output: &mut impl Write) -> io::Result<()> {
        let Some(editor) = &mut self.editor else {
            return Ok(());
        };
        let rendered = if diagnostics.is_empty() {
            Vec::new()
        } else {
            let lines = Lines::new(&self.source);
            diagnostics.iter().filter_map(|diagnostic| {
                let range = lines.range(diagnostic.span)?;
                let severity = if diagnostic.severity == "warning" { 2 } else { 1 };
                Some(json!({"range":range,"severity":severity,"source":"parenox","code":diagnostic.code,"message":diagnostic.message}))
            }).collect()
        };
        if editor.published.as_ref() != Some(&rendered) {
            publish(
                output,
                json!({"uri":editor.uri,"version":editor.version,"diagnostics":rendered}),
            )?;
            editor.published = Some(rendered);
        }
        Ok(())
    }
}

fn is_edn(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "edn")
}

fn document_path(uri: &str) -> Option<String> {
    let path = file_path(uri)?;
    (is_clojure_source(&path) || is_edn(&path)).then(|| path.to_string_lossy().into_owned())
}

fn publish(output: &mut impl Write, params: Value) -> io::Result<()> {
    write_message(
        output,
        &json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":params}),
    )
}

fn log_load_error(output: &mut impl Write, message: &str) -> io::Result<()> {
    write_message(
        output,
        &json!({
            "jsonrpc":"2.0",
            "method":"window/logMessage",
            "params":{"type":1,"message":format!("Could not load Parenox project ({message})")}
        }),
    )
}
