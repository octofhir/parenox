//! JSON Lines requests and an optional in-memory project for incremental edits.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Write};

use parenox::{ProjectReport, Session, Source};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRequest {
    #[serde(default)]
    id: Value,
    path: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    pub path: String,
    pub source: String,
}

impl Snapshot {
    pub(super) fn as_source(&self) -> Source<'_> {
        Source {
            path: &self.path,
            source: &self.source,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExternSnapshots {
    One(Snapshot),
    Many(Vec<Snapshot>),
}

fn deserialize_externs<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Snapshot>, D::Error> {
    Option::<ExternSnapshots>::deserialize(deserializer).map(|files| match files {
        None => Vec::new(),
        Some(ExternSnapshots::One(file)) => vec![file],
        Some(ExternSnapshots::Many(files)) => files,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectRequest {
    #[serde(default)]
    id: Value,
    files: Vec<Snapshot>,
    #[serde(default, deserialize_with = "deserialize_externs")]
    externs: Vec<Snapshot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateRequest {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    changes: Vec<Snapshot>,
    #[serde(default)]
    remove: Vec<String>,
    #[serde(default)]
    externs: ExternsChange,
}

#[derive(Default)]
enum ExternsChange {
    #[default]
    Keep,
    Replace(Vec<Snapshot>),
}

impl<'de> Deserialize<'de> for ExternsChange {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_externs(deserializer).map(Self::Replace)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionRequest {
    #[serde(default)]
    id: Value,
    path: String,
    offset: usize,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ProjectOperation {
    Open(ProjectRequest),
    Update(UpdateRequest),
    Definition(DefinitionRequest),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Request {
    File(FileRequest),
    Project(ProjectRequest),
    Tracked(ProjectOperation),
}

#[derive(Default)]
struct TrackedProject {
    files: BTreeMap<String, String>,
    externs: Vec<Snapshot>,
    session: Session,
}

impl TrackedProject {
    fn open(files: Vec<Snapshot>, externs: Vec<Snapshot>) -> Result<Self, String> {
        let mut project = Self::default();
        project.update(files, Vec::new(), ExternsChange::Replace(externs))?;
        Ok(project)
    }

    fn update(
        &mut self,
        changes: Vec<Snapshot>,
        remove: Vec<String>,
        externs: ExternsChange,
    ) -> Result<(), String> {
        let mut updated = BTreeSet::new();
        for file in &changes {
            if !updated.insert(file.path.as_str()) {
                return Err(format!("Duplicate source path {:?}", file.path));
            }
        }
        let removed: BTreeSet<_> = remove.iter().map(String::as_str).collect();
        if let Some(path) = removed.intersection(&updated).next() {
            return Err(format!("Source path {path:?} is both changed and removed"));
        }
        let path_conflict = |path: &str| format!("Externs path {path:?} is also a source path");
        match &externs {
            ExternsChange::Keep => {
                // Existing pairs have already been validated. Only newly
                // supplied sources can introduce a path collision.
                if !updated.is_empty()
                    && let Some(file) = self
                        .externs
                        .iter()
                        .find(|file| updated.contains(file.path.as_str()))
                {
                    return Err(path_conflict(&file.path));
                }
            }
            ExternsChange::Replace(files) => {
                let mut paths = BTreeSet::new();
                for file in files {
                    if !paths.insert(file.path.as_str()) {
                        return Err(format!("Duplicate externs path {:?}", file.path));
                    }
                    if updated.contains(file.path.as_str())
                        || (!removed.contains(file.path.as_str())
                            && self.files.contains_key(&file.path))
                    {
                        return Err(path_conflict(&file.path));
                    }
                }
                for previous in &self.externs {
                    if !paths.contains(previous.path.as_str()) {
                        self.session.forget_file(&previous.path);
                    }
                }
            }
        }

        for path in &remove {
            if self.files.remove(path).is_some() {
                self.session.forget_file(path);
            }
        }
        self.files
            .extend(changes.into_iter().map(|file| (file.path, file.source)));
        if let ExternsChange::Replace(files) = externs {
            self.externs = files;
        }
        Ok(())
    }

    fn check(&mut self) -> ProjectReport {
        let sources: Vec<_> = self
            .files
            .iter()
            .map(|(path, source)| Source { path, source })
            .collect();
        let externs: Vec<_> = self.externs.iter().map(Snapshot::as_source).collect();
        self.session.check_project_with_externs(&sources, &externs)
    }
}

#[derive(Serialize)]
struct CheckResponse<T> {
    id: Value,
    result: T,
}

#[derive(Serialize)]
struct ErrorResponse {
    id: Value,
    error: RequestError,
}

#[derive(Serialize)]
struct RequestError {
    code: &'static str,
    message: String,
}

fn write_check<T: Serialize>(
    output: &mut impl Write,
    id: Value,
    result: T,
) -> serde_json::Result<()> {
    serde_json::to_writer(output, &CheckResponse { id, result })
}

fn write_error(output: &mut impl Write, id: Value, message: String) -> serde_json::Result<()> {
    serde_json::to_writer(
        output,
        &ErrorResponse {
            id,
            error: RequestError {
                code: "invalid_request",
                message,
            },
        },
    )
}

fn write_tracked(
    output: &mut impl Write,
    tracked: &mut Option<TrackedProject>,
    op: ProjectOperation,
) -> serde_json::Result<()> {
    match op {
        ProjectOperation::Open(request) => {
            match TrackedProject::open(request.files, request.externs) {
                Ok(project) => write_check(output, request.id, tracked.insert(project).check()),
                Err(message) => write_error(output, request.id, message),
            }
        }
        ProjectOperation::Update(request) => {
            let Some(project) = tracked else {
                return write_error(
                    output,
                    request.id,
                    "Open a project before sending updates".into(),
                );
            };
            match project.update(request.changes, request.remove, request.externs) {
                Ok(()) => write_check(output, request.id, project.check()),
                Err(message) => write_error(output, request.id, message),
            }
        }
        ProjectOperation::Definition(request) => match tracked {
            Some(project) => write_check(
                output,
                request.id,
                project.session.definition(&request.path, request.offset),
            ),
            None => write_error(
                output,
                request.id,
                "Open a project before querying a definition".into(),
            ),
        },
    }
}

pub(super) fn run() -> Result<u8, String> {
    let mut snapshot_session = Session::default();
    let mut tracked = None;
    let mut input = io::stdin().lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut line = String::new();
    while input.read_line(&mut line).map_err(|e| e.to_string())? != 0 {
        match serde_json::from_str::<Request>(&line) {
            Ok(Request::File(request)) => write_check(
                &mut output,
                request.id,
                snapshot_session.check(&request.path, &request.source),
            ),
            Ok(Request::Project(request)) => {
                let sources: Vec<_> = request.files.iter().map(Snapshot::as_source).collect();
                let externs: Vec<_> = request.externs.iter().map(Snapshot::as_source).collect();
                let report = snapshot_session.check_project_with_externs(&sources, &externs);
                write_check(&mut output, request.id, report)
            }
            Ok(Request::Tracked(op)) => write_tracked(&mut output, &mut tracked, op),
            Err(error) => write_error(&mut output, Value::Null, error.to_string()),
        }
        .map_err(|e| e.to_string())?;
        writeln!(output).map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
        line.clear();
    }
    Ok(0)
}
