//! One workspace and its in-memory editor overlays.
#[path = "call_hierarchy.rs"]
mod call_hierarchy;
#[path = "file_rename.rs"]
pub(super) mod file_rename;
#[path = "inspection.rs"]
pub(super) mod inspection;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use parenox::formatter::{self, FormatOptions, Language};
use parenox::{
    Definition, Diagnostic, Session, Source, output::contract_note, reader::validate_edn,
};
use parenox_lsp::structural::{self, FoldingOptions};
use parenox_lsp::{hover, lenses, project_tree, refactor, rename, semantic, test_tree};
use serde::Deserialize;
use serde_json::{Value, json};

use super::WATCH_REQUEST_ID;
use super::completion;
use super::protocol::write_message;
use super::symbols;
use super::text::{Lines, Position, Range, file_path, file_uri, normalize};
use crate::cli::ProjectOptions;
use crate::workspace::{DiskProject, FileSnapshot, is_clojure_source};

pub(super) struct Workspace {
    root: PathBuf,
    options: ProjectOptions,
    source_roots: Vec<PathBuf>,
    editable_roots: Vec<PathBuf>,
    source_aliases: Vec<(PathBuf, PathBuf)>,
    register_watches: bool,
    versioned_edits: bool,
    test_tree: bool,
    format_options: FormatOptions,
    dependency_completions: completion::DependencyCompletions,
    documents: BTreeMap<String, Document>,
    externs: Vec<FileSnapshot>,
    session: Session,
    session_ready: bool,
    load_error: Option<String>,
}

struct Document {
    source: String,
    indexed: bool,
    standalone: Option<Box<Session>>,
    editor: Option<Editor>,
}

struct Editor {
    uri: String,
    version: i64,
    published: Option<Vec<Value>>,
    test_tree: Option<Value>,
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
    content_changes: Vec<ContentChange>,
}

#[derive(Deserialize)]
struct VersionedUri {
    uri: String,
    version: i64,
}

#[derive(Deserialize)]
struct ContentChange {
    text: String,
    range: Option<Range>,
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
#[serde(rename_all = "camelCase")]
pub(super) struct SelectionQuery {
    text_document: DocumentUri,
    positions: Vec<Position>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FormatRangeQuery {
    text_document: DocumentUri,
    range: Range,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RenameQuery {
    #[serde(flatten)]
    query: TextQuery,
    new_name: String,
}

#[derive(Deserialize)]
pub(super) struct ReferenceQuery {
    #[serde(flatten)]
    query: TextQuery,
    context: ReferenceContext,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceContext {
    include_declaration: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CodeActionQuery {
    text_document: DocumentUri,
    range: Range,
    context: CodeActionContext,
}

#[derive(Deserialize)]
struct CodeActionContext {
    only: Option<Vec<String>>,
}

impl Workspace {
    pub fn load(
        root: PathBuf,
        options: &ProjectOptions,
        register_watches: bool,
        versioned_edits: bool,
        test_tree: bool,
    ) -> Self {
        let mut workspace = Self {
            root,
            options: options.clone(),
            source_roots: Vec::new(),
            editable_roots: Vec::new(),
            source_aliases: Vec::new(),
            register_watches,
            versioned_edits,
            test_tree,
            format_options: FormatOptions::default(),
            dependency_completions: completion::DependencyCompletions::default(),
            documents: BTreeMap::new(),
            externs: Vec::new(),
            session: Session::default(),
            session_ready: false,
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
        let mut settings = self.options.load_for_format(&self.root)?;
        let format_options = settings.format.clone();
        let editable_roots = settings
            .paths
            .iter()
            .map(|path| normalize(Path::new(path)))
            .collect();
        let source_roots: Vec<_> = settings
            .analysis_paths()
            .map(|path| normalize(Path::new(path)))
            .collect();
        let classpath = settings.classpath.take().unwrap_or_else(|| {
            source_roots
                .iter()
                .cloned()
                .chain(self.options.dependency_jars(&self.root))
                .collect()
        });
        let source_aliases: Vec<_> = source_roots
            .iter()
            .filter_map(|root| {
                let physical = root.canonicalize().ok()?;
                (physical != *root).then(|| (physical, root.clone()))
            })
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
                        standalone: None,
                        editor: None,
                    },
                )
            })
            .collect();
        for (path, mut document) in std::mem::take(&mut self.documents) {
            if let Some(editor) = &document.editor {
                let editor_path = file_path(&editor.uri);
                let indexed = editor_path
                    .as_deref()
                    .and_then(|file| indexed_document(&documents, &source_aliases, file));
                document.indexed = indexed.is_some();
                let key = indexed
                    .or_else(|| editor_path.map(|path| path.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| path.clone());
                if document.indexed
                    || in_source_roots(Path::new(&key), &source_roots, &source_aliases)
                {
                    document.standalone = None;
                } else if is_clojure_source(Path::new(&key)) {
                    document.standalone.get_or_insert_with(Box::default);
                }
                if key != path || document.standalone.is_some() {
                    self.session.forget_file(&path);
                }
                documents.insert(key, document);
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
        self.editable_roots = editable_roots;
        self.source_aliases = source_aliases;
        self.externs = project.externs;
        self.format_options = format_options;
        self.dependency_completions = completion::DependencyCompletions::new(classpath);
        self.session_ready = false;
        Ok(())
    }

    pub fn open(&mut self, params: Open, output: &mut impl Write) -> io::Result<()> {
        let OpenDocument { uri, version, text } = params.text_document;
        let Some(path) = self.document_key(&uri) else {
            return Ok(());
        };
        let indexed = self
            .documents
            .get(&path)
            .is_some_and(|document| document.indexed);
        let file = Path::new(&path);
        let standalone = !indexed
            && is_clojure_source(file)
            && !in_source_roots(file, &self.source_roots, &self.source_aliases);
        self.documents.insert(
            path.clone(),
            Document {
                source: text,
                indexed,
                standalone: standalone.then(Box::default),
                editor: Some(Editor {
                    uri,
                    version,
                    published: None,
                    test_tree: None,
                }),
            },
        );
        self.check_document(&path, output)
    }

    pub fn change(&mut self, params: Change, output: &mut impl Write) -> io::Result<()> {
        let VersionedUri { uri, version } = params.text_document;
        let Some(path) = self.document_key(&uri) else {
            return Ok(());
        };
        let Some(document) = self.documents.get_mut(&path) else {
            return Ok(());
        };
        let Some(editor) = &mut document.editor else {
            return Ok(());
        };
        if version <= editor.version {
            return Ok(());
        }
        // Each range refers to the source after the preceding change in this batch.
        let mut snapshot = None;
        for change in params.content_changes {
            if let Some(range) = change.range {
                let source = snapshot.get_or_insert_with(|| document.source.clone());
                let Some(bytes) = Lines::new(source).byte_range(range) else {
                    return Ok(());
                };
                source.replace_range(bytes, &change.text);
            } else {
                snapshot = Some(change.text);
            }
        }
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        document.source = snapshot;
        editor.version = version;
        editor.published = None;
        self.check_document(&path, output)
    }

    pub fn contains_project_source(&self, path: &Path) -> bool {
        !self.source_roots.is_empty()
            && in_source_roots(path, &self.source_roots, &self.source_aliases)
    }

    pub fn save(&mut self, params: &DocumentParams, output: &mut impl Write) -> io::Result<()> {
        let Some(file) = file_path(&params.text_document.uri) else {
            return Ok(());
        };
        let new_source = is_clojure_source(&file)
            && self
                .document_key(&params.text_document.uri)
                .and_then(|key| self.documents.get(&key))
                .is_some_and(|doc| !doc.indexed && doc.editor.is_some());
        if new_source || self.is_project_input(&file) {
            self.refresh(output)?;
        }
        Ok(())
    }

    pub fn changed_files(
        &mut self,
        params: &WatchedFiles,
        output: &mut impl Write,
    ) -> io::Result<()> {
        for event in &params.changes {
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
        if let Some(document) = indexed_document(&self.documents, &self.source_aliases, path)
            .and_then(|key| self.documents.get(&key))
        {
            // A change to an open document arrives through its editor overlay.
            return kind != 2 || document.editor.is_none();
        }
        // A newly created source can also create a previously absent manifest root.
        let workspace = (kind == 1).then_some(&self.root);
        self.source_roots
            .iter()
            .chain(self.source_aliases.iter().map(|(physical, _)| physical))
            .chain(workspace)
            .any(|root| visible_under(path, root))
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
            Ok(()) => {
                self.check(output)?;
                for (path, document) in &mut self.documents {
                    document.check_standalone(path, output)?;
                }
                if self.test_tree {
                    let paths: Vec<_> = self
                        .documents
                        .iter()
                        .filter(|(path, document)| {
                            document.editor.is_some() && is_clojure_source(Path::new(path))
                        })
                        .map(|(path, _)| path.clone())
                        .collect();
                    for path in paths {
                        self.publish_test_tree(&path, output)?;
                    }
                }
                Ok(())
            }
            Err(message) => log_load_error(output, &message),
        }
    }

    pub fn close(&mut self, params: DocumentParams, output: &mut impl Write) -> io::Result<()> {
        let Some(path) = self.document_key(&params.text_document.uri) else {
            return Ok(());
        };
        let Some(document) = self.documents.get_mut(&path) else {
            return Ok(());
        };
        let Some(editor) = document.editor.take() else {
            return Ok(());
        };
        let standalone = document.standalone.is_some();
        if document.indexed
            && let Ok(source) = fs::read_to_string(&path)
        {
            document.source = source;
        } else {
            self.documents.remove(&path);
            self.session.forget_file(&path);
        }
        publish(output, json!({"uri":editor.uri, "diagnostics":[]}))?;
        if standalone || !is_clojure_source(Path::new(&path)) {
            Ok(())
        } else {
            self.check(output)
        }
    }

    pub fn definition(&self, params: TextQuery) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let session = self.session_for(&path)?;
        session
            .definition(&path, offset)
            .and_then(|target| self.location(&target))
            .or_else(|| {
                self.dependency_completions
                    .definition(session, &path, offset)
            })
    }

    pub fn declaration(&self, params: TextQuery) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let target = self.session_for(&path)?.declaration(&path, offset)?;
        self.location(&target)
    }

    pub fn implementations(&self, params: TextQuery) -> Option<Vec<Value>> {
        let (path, offset) = self.query_offset(params)?;
        Some(
            self.session_for(&path)?
                .implementations(&path, offset)
                .iter()
                .filter_map(|target| self.location(target))
                .collect(),
        )
    }

    fn location(&self, target: &Definition) -> Option<Value> {
        let document = self.documents.get(&target.path)?;
        let range = Lines::new(&document.source).range(target.span)?;
        Some(json!({"uri":document.uri(&target.path),"range":range}))
    }

    pub fn references(&self, params: ReferenceQuery) -> Vec<Value> {
        let Some((path, offset)) = self.query_offset(params.query) else {
            return Vec::new();
        };
        let Some(session) = self.session_for(&path) else {
            return Vec::new();
        };
        let usages = session.references(&path, offset, params.context.include_declaration);
        let mut values = Vec::with_capacity(usages.len());
        for group in usages.chunk_by(|a, b| a.path == b.path) {
            let path = &group[0].path;
            let Some(document) = self.documents.get(path) else {
                continue;
            };
            let lines = Lines::new(&document.source);
            let uri = document.uri(path);
            values.extend(group.iter().filter_map(|Definition { span, .. }| {
                Some(json!({"uri":uri,"range":lines.range(*span)?}))
            }));
        }
        values
    }

    pub fn highlights(&self, params: TextQuery) -> Option<Vec<Value>> {
        let (path, offset) = self.query_offset(params)?;
        let document = self.documents.get(&path)?;
        let lines = Lines::new(&document.source);
        Some(
            self.session_for(&path)?
                .document_highlights(&path, offset)
                .into_iter()
                .filter_map(|usage| Some(json!({"range":lines.range(usage.span)?,"kind":1})))
                .collect(),
        )
    }

    pub fn prepare_rename(&self, params: TextQuery) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let session = self.session_for(&path)?;
        let target = session.definition(&path, offset);
        self.rename_document(target.as_ref().map_or(path.as_str(), |item| &item.path))?;
        rename::prepare(session, &path, offset, &self.documents.get(&path)?.source)
    }

    pub fn rename(&self, params: RenameQuery) -> Result<Value, String> {
        let (path, offset) = self
            .query_offset(params.query)
            .ok_or("No source at the rename position")?;
        let session = self.session_for(&path).ok_or("No source project")?;
        rename::edits(
            session,
            &path,
            offset,
            &params.new_name,
            self.versioned_edits,
            |file| self.rename_document(file),
        )
    }

    pub fn linked_editing(&self, params: TextQuery) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let document = self.rename_document(&path)?;
        rename::linked(self.session_for(&path)?, &path, offset, document.source)
    }

    fn rename_document(&self, path: &str) -> Option<rename::Document<'_>> {
        let document = self.documents.get(path)?;
        if document.standalone.is_none()
            && !self
                .editable_roots
                .iter()
                .any(|root| visible_under(Path::new(path), root))
        {
            return None;
        }
        Some(rename::Document {
            source: &document.source,
            uri: document.uri(path),
            version: document.editor.as_ref().map(|editor| editor.version),
        })
    }

    pub fn document_symbols(
        &self,
        params: DocumentParams,
        hierarchical: bool,
    ) -> Option<Vec<Value>> {
        let path = self.document_key(&params.text_document.uri)?;
        let document = self.documents.get(&path)?;
        let (namespace, entries) = self.session_for(&path)?.document_symbols(&path)?;
        Some(symbols::document(
            entries,
            namespace,
            &document.uri(&path),
            &document.source,
            hierarchical,
        ))
    }

    pub fn folding_ranges(
        &self,
        params: DocumentParams,
        options: &FoldingOptions,
    ) -> Option<Vec<Value>> {
        let path = self.document_key(&params.text_document.uri)?;
        let document = self.documents.get(&path)?;
        let (_, symbols) = self.session_for(&path)?.document_symbols(&path)?;
        Some(structural::folding_ranges(
            &document.source,
            symbols,
            options,
        ))
    }

    pub fn selection_ranges(&self, params: SelectionQuery) -> Option<Vec<Value>> {
        let path = self.document_key(&params.text_document.uri)?;
        let document = self.documents.get(&path)?;
        let language = Language::for_path(Path::new(&path))?;
        Some(structural::selection_ranges(
            &document.source,
            language == Language::Edn,
            &params.positions,
        ))
    }

    pub fn semantic_tokens_full(&self, params: DocumentParams) -> Option<Value> {
        self.semantic_tokens(&params.text_document.uri, None)
    }

    pub fn code_lenses(&self, params: DocumentParams) -> Option<Vec<Value>> {
        let path = self.document_key(&params.text_document.uri)?;
        let document = self.documents.get(&path)?;
        lenses::list(
            self.session_for(&path)?,
            &path,
            &document.source,
            &document.uri(&path),
        )
    }

    pub fn resolve_code_lens(&self, query: lenses::Query) -> Option<Value> {
        let path = self.document_key(&query.data.0)?;
        let document = self.documents.get(&path)?;
        lenses::resolve(self.session_for(&path)?, &path, &document.source, query)
    }

    pub fn semantic_tokens_range(&self, params: FormatRangeQuery) -> Option<Value> {
        self.semantic_tokens(&params.text_document.uri, Some(params.range))
    }

    fn semantic_tokens(&self, uri: &str, range: Option<Range>) -> Option<Value> {
        let path = self.document_key(uri)?;
        if !is_clojure_source(Path::new(&path)) {
            return None;
        }
        let document = self.documents.get(&path)?;
        let bytes = match range {
            Some(range) => Some(Lines::new(&document.source).byte_range(range)?),
            None => None,
        };
        Some(semantic::tokens(
            &document.source,
            self.session_for(&path)?,
            &path,
            bytes,
        ))
    }

    pub fn symbol_candidates(&self) -> impl Iterator<Item = symbols::Candidate<'_>> {
        self.session
            .project_symbols()
            .filter_map(|(path, namespace, symbol)| {
                let document = self.documents.get(path)?;
                Some(symbols::Candidate {
                    path,
                    namespace,
                    symbol,
                    source: &document.source,
                    editor_uri: document.editor.as_ref().map(|editor| editor.uri.as_str()),
                })
            })
    }

    pub fn ensure_index(&mut self, output: &mut impl Write) -> io::Result<()> {
        if !self.session_ready {
            self.check(output)?;
        }
        Ok(())
    }

    pub fn hover(
        &self,
        params: TextQuery,
        markdown: bool,
        clojuredocs: &parenox_lsp::clojuredocs::ClojureDocs,
    ) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let session = self.session_for(&path)?;
        let range = session
            .hover_symbol_span(&path, offset)
            .and_then(|span| Lines::new(&self.documents.get(&path)?.source).range(span));
        let Some(source) = session.hover_at(&path, offset) else {
            return hover::dependency(
                session,
                &path,
                offset,
                &self.dependency_completions,
                markdown,
                range,
                clojuredocs,
            );
        };
        let mut checked = source.info.signatures;
        checked.push(source.info.contract.as_ref().map_or_else(
            || "Checked type information unavailable".into(),
            contract_note,
        ));
        Some(hover::render(
            &source.info.name,
            &source.arglists,
            &checked,
            source.documentation.as_deref(),
            markdown,
            range,
            clojuredocs.find(&source.info.name),
        ))
    }

    pub fn signature_help(&self, params: TextQuery) -> Option<Value> {
        let (path, offset) = self.query_offset(params)?;
        let info = self.session_for(&path)?.call_signatures(&path, offset)?;
        let signatures: Vec<_> = info
            .signatures
            .iter()
            .map(|signature| {
                let parameters: Vec<_> = signature
                    .parameters
                    .iter()
                    .map(|label| json!({"label":label}))
                    .collect();
                let mut item = json!({
                    "label":format!("({} [{}])", info.name, signature.parameters.join(" ")),
                    "parameters":parameters,
                });
                if let Some(documentation) = &info.documentation {
                    item["documentation"] = json!(documentation);
                }
                item
            })
            .collect();
        let mut help = json!({"signatures":signatures,"activeSignature":info.active_signature});
        if let Some(active) = info.active_parameter {
            help["activeParameter"] = json!(active);
        }
        Some(help)
    }

    pub fn project_tree(&self, query: Option<project_tree::Leaf>) -> Option<Value> {
        project_tree::nodes(
            &self.root,
            &self.editable_roots,
            self.dependency_completions.jars(),
            self.symbol_candidates(),
            query,
        )
    }

    pub fn dependency_contents(&self, uri: &str) -> Option<String> {
        self.dependency_completions.dependency_contents(uri)
    }

    pub fn completion(&self, params: TextQuery, snippets: bool) -> Option<Value> {
        let path = self.document_key(&params.text_document.uri)?;
        if !is_clojure_source(Path::new(&path)) {
            return None;
        }
        let document = self.documents.get(&path)?;
        document.editor.as_ref()?;
        let lines = Lines::new(&document.source);
        completion::complete(
            self.session_for(&path)?,
            &path,
            &params.text_document.uri,
            &lines,
            lines.offset(params.position)?,
            snippets,
            &self.dependency_completions,
        )
    }

    pub fn resolve_completion(&self, item: Value) -> Value {
        let Some(path) = completion::resolve_uri(&item).and_then(|uri| self.document_key(uri))
        else {
            return item;
        };
        match self.session_for(&path) {
            Some(session) => completion::resolve(session, &path, item),
            None => item,
        }
    }

    pub fn format_document(&self, params: DocumentParams) -> Option<Value> {
        self.format(&params.text_document.uri, None)
    }

    pub fn format_range(&self, params: FormatRangeQuery) -> Option<Value> {
        self.format(&params.text_document.uri, Some(params.range))
    }

    fn format(&self, uri: &str, range: Option<Range>) -> Option<Value> {
        let path = self.document_key(uri)?;
        let language = Language::for_path(Path::new(&path))?;
        let document = self.documents.get(&path)?;
        document.editor.as_ref()?;
        let lines = Lines::new(&document.source);
        let text = if let Some(range) = range {
            formatter::format_lines(
                &document.source,
                language,
                &self.format_options,
                lines.line_selection(range)?,
            )
        } else {
            formatter::format(&document.source, language, &self.format_options)
        }
        .ok()?;
        if text == document.source {
            return None;
        }
        Some(json!(lines.formatting_edits(&text)))
    }

    pub fn code_actions(&self, params: CodeActionQuery) -> Vec<Value> {
        let only = params.context.only.as_deref();
        let rewrite = refactor::action_allowed(refactor::REWRITE_KIND, only);
        if !rewrite
            && !refactor::action_allowed(refactor::REFACTOR_KIND, only)
            && !refactor::action_allowed(refactor::EXTRACT_KIND, only)
        {
            return Vec::new();
        }
        let Some(path) = self.document_key(&params.text_document.uri) else {
            return Vec::new();
        };
        let Some(document) = self.documents.get(&path) else {
            return Vec::new();
        };
        let lines = Lines::new(&document.source);
        let Some(offset) = lines.byte_range(params.range).map(|range| range.start) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if rewrite {
            actions.extend(self.fdef_action(&path, &lines, offset));
            if let Some(edit) =
                self.refactor_edit(&path, &lines, offset, refactor::Command::CyclePrivacy, None)
            {
                actions.push(json!({"title":refactor::CYCLE_PRIVACY_TITLE,"kind":refactor::REWRITE_KIND,"edit":edit}));
            }
        }
        if is_clojure_source(Path::new(&path))
            && self.rename_document(&path).is_some()
            && let Some(session) = self.session_for(&path)
        {
            actions.extend(refactor::command_actions(
                session,
                &path,
                &lines,
                offset,
                &params.text_document.uri,
                only,
            ));
        }
        actions
    }

    pub fn refactor(
        &self,
        uri: &str,
        position: Position,
        command: refactor::Command,
        collection: Option<refactor::Collection>,
    ) -> Option<Value> {
        let path = self.document_key(uri)?;
        let lines = Lines::new(&self.documents.get(&path)?.source);
        self.refactor_edit(&path, &lines, lines.offset(position)?, command, collection)
    }

    fn refactor_edit(
        &self,
        path: &str,
        lines: &Lines<'_>,
        offset: usize,
        command: refactor::Command,
        collection: Option<refactor::Collection>,
    ) -> Option<Value> {
        if !is_clojure_source(Path::new(path)) {
            return None;
        }
        let document = self.rename_document(path)?;
        let edits = command.edits(self.session_for(path)?, path, lines, offset, collection)?;
        Some(document.workspace_edit(edits, self.versioned_edits))
    }

    fn fdef_action(&self, path: &str, lines: &Lines<'_>, offset: usize) -> Option<Value> {
        let editor = self.documents.get(path)?.editor.as_ref()?;
        let suggestion = self.session_for(path)?.suggest_fdef(path, offset)?;
        let title = if suggestion.draft {
            "Draft s/fdef (review unknown types)"
        } else {
            "Generate s/fdef"
        };
        let edits = suggestion.edits.into_iter().map(|edit| {
            let position = lines.position(edit.offset)?;
            Some(json!({"range": {"start": position, "end": position}, "newText": edit.new_text}))
        }).collect::<Option<Vec<_>>>()?;
        let document = rename::Document {
            source: lines.source(),
            uri: editor.uri.clone(),
            version: Some(editor.version),
        };
        let edit = document.workspace_edit(edits, self.versioned_edits);
        Some(json!({"title":title,"kind":"refactor.rewrite","edit":edit}))
    }

    fn session_for(&self, path: &str) -> Option<&Session> {
        Some(
            self.documents
                .get(path)?
                .standalone
                .as_deref()
                .unwrap_or(&self.session),
        )
    }

    fn query_offset(&self, params: TextQuery) -> Option<(String, usize)> {
        let path = self.document_key(&params.text_document.uri)?;
        if !is_clojure_source(Path::new(&path)) {
            return None;
        }
        let document = self.documents.get(&path)?;
        let offset = Lines::new(&document.source).offset(params.position)?;
        Some((path, offset))
    }

    fn document_key(&self, uri: &str) -> Option<String> {
        let path = document_path(uri)?;
        if self.documents.contains_key(&path) || is_edn(Path::new(&path)) {
            return Some(path);
        }
        Some(
            indexed_document(&self.documents, &self.source_aliases, Path::new(&path))
                .unwrap_or(path),
        )
    }

    fn check_document(&mut self, path: &str, output: &mut impl Write) -> io::Result<()> {
        let document = self.documents.get_mut(path).unwrap();
        if is_edn(Path::new(path)) {
            let diagnostic = validate_edn(&document.source).err().map(Diagnostic::from);
            return document.publish(diagnostic.as_slice(), output);
        } else if !is_clojure_source(Path::new(path)) {
            return Ok(());
        } else if document.standalone.is_some() {
            document.check_standalone(path, output)?;
        } else {
            self.check(output)?;
        }
        self.publish_test_tree(path, output)
    }

    fn publish_test_tree(&mut self, path: &str, output: &mut impl Write) -> io::Result<()> {
        if !self.test_tree {
            return Ok(());
        }
        let Some(document) = self.documents.get_mut(path) else {
            return Ok(());
        };
        let Some(editor) = &mut document.editor else {
            return Ok(());
        };
        let session = document.standalone.as_deref().unwrap_or(&self.session);
        let tree = test_tree::params(session, path, &editor.uri, &document.source);
        if tree != editor.test_tree {
            if let Some(params) = &tree {
                write_message(
                    output,
                    &json!({
                        "jsonrpc":"2.0","method":"clojure/textDocument/testTree","params":params
                    }),
                )?;
            }
            editor.test_tree = tree;
        }
        Ok(())
    }

    fn check(&mut self, output: &mut impl Write) -> io::Result<()> {
        let mut sources = Vec::new();
        let mut open = BTreeSet::new();
        for (path, doc) in &self.documents {
            if doc.standalone.is_some() || !is_clojure_source(Path::new(path)) {
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
        self.session_ready = true;
        for file in report.files {
            if let Some(document) = self.documents.get_mut(&file.path) {
                document.publish(&file.diagnostics, output)?;
            }
        }
        Ok(())
    }
}

fn in_source_roots(path: &Path, roots: &[PathBuf], aliases: &[(PathBuf, PathBuf)]) -> bool {
    roots.is_empty()
        || roots
            .iter()
            .chain(aliases.iter().map(|(physical, _)| physical))
            .any(|root| visible_under(path, root))
}

fn visible_under(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|tail| {
        !tail
            .components()
            .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
    })
}

fn indexed_document(
    documents: &BTreeMap<String, Document>,
    aliases: &[(PathBuf, PathBuf)],
    path: &Path,
) -> Option<String> {
    let indexed = |path: &Path| {
        let key = path.to_str()?;
        documents
            .get(key)
            .is_some_and(|document| document.indexed)
            .then(|| key.to_owned())
    };
    let through_alias = |path: &Path| {
        aliases.iter().find_map(|(physical, logical)| {
            let tail = path.strip_prefix(physical).ok()?;
            indexed(&logical.join(tail))
        })
    };
    indexed(path).or_else(|| through_alias(path)).or_else(|| {
        let canonical = path.canonicalize().ok()?;
        indexed(&canonical).or_else(|| through_alias(&canonical))
    })
}

impl Document {
    fn uri(&self, path: &str) -> String {
        self.editor
            .as_ref()
            .map_or_else(|| file_uri(path), |editor| editor.uri.clone())
    }

    fn check_standalone(&mut self, path: &str, output: &mut impl Write) -> io::Result<()> {
        if let Some(session) = &mut self.standalone {
            let report = session.check(path, &self.source);
            self.publish(&report.diagnostics, output)?;
        }
        Ok(())
    }

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
    Language::for_path(&path)?;
    Some(path.to_string_lossy().into_owned())
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
