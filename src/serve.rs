//! JSON Lines requests and an optional in-memory project for incremental edits.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use parenox::{ProjectReport, Session, Source, VarInfo};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::cli::ProjectOptions;
use crate::workspace::{DiskProject, FileSnapshot};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRequest {
    #[serde(default)]
    id: Value,
    path: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExternSnapshots {
    One(FileSnapshot),
    Many(Vec<FileSnapshot>),
}

fn deserialize_externs<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<FileSnapshot>, D::Error> {
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
    files: Vec<FileSnapshot>,
    #[serde(default, deserialize_with = "deserialize_externs")]
    externs: Vec<FileSnapshot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenProjectRequest {
    #[serde(default)]
    id: Value,
    root: String,
    #[serde(default)]
    aliases: Vec<String>,
    classpath: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateRequest {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    changes: Vec<FileSnapshot>,
    #[serde(default)]
    remove: Vec<String>,
    #[serde(default)]
    externs: ExternsChange,
}

#[derive(Default)]
enum ExternsChange {
    #[default]
    Keep,
    Replace(Vec<FileSnapshot>),
}

impl<'de> Deserialize<'de> for ExternsChange {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_externs(deserializer).map(Self::Replace)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    #[serde(default)]
    id: Value,
    path: String,
    offset: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefreshProjectRequest {
    #[serde(default)]
    id: Value,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ProjectOperation {
    Open(ProjectRequest),
    OpenProject(OpenProjectRequest),
    RefreshProject(RefreshProjectRequest),
    Update(UpdateRequest),
    Definition(QueryRequest),
    Hover(QueryRequest),
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
    externs: Vec<FileSnapshot>,
    session: Session,
    disk: Option<DiskState>,
}

struct DiskState {
    root: PathBuf,
    options: ProjectOptions,
    overlays: BTreeSet<String>,
    externs_overridden: bool,
}

impl TrackedProject {
    fn open(files: Vec<FileSnapshot>, externs: Vec<FileSnapshot>) -> Result<Self, String> {
        let mut project = Self::default();
        project.update(files, Vec::new(), ExternsChange::Replace(externs))?;
        Ok(project)
    }

    fn open_project(
        root: &str,
        aliases: Vec<String>,
        classpath: Option<String>,
    ) -> Result<Self, String> {
        let path = Path::new(root)
            .canonicalize()
            .map_err(|error| format!("{root}: {error}"))?;
        if !path.is_dir() {
            return Err(format!("{root}: expected a project directory"));
        }
        let options = ProjectOptions {
            aliases,
            classpath,
            ..ProjectOptions::default()
        };
        let project = DiskProject::read(options.load_at(&path)?)?;
        project.require_source_files()?;
        let mut tracked = Self {
            files: project.files,
            ..Self::default()
        };
        let externs = project
            .externs
            .into_iter()
            .chain(project.dependency_externs)
            .collect();
        tracked.update(Vec::new(), Vec::new(), ExternsChange::Replace(externs))?;
        tracked.disk = Some(DiskState {
            root: path,
            options,
            overlays: BTreeSet::new(),
            externs_overridden: false,
        });
        Ok(tracked)
    }

    fn refresh_disk(&mut self) -> Result<(), String> {
        let disk = self
            .disk
            .as_ref()
            .ok_or("Open a disk project before refreshing")?;
        let mut settings = disk.options.load_at(&disk.root)?;
        if disk.externs_overridden {
            settings.externs.clear();
            settings.dependency_externs.clear();
        }
        let DiskProject {
            mut files,
            externs,
            dependency_externs,
        } = DiskProject::read(settings)?;
        for path in &disk.overlays {
            if let Some(source) = self.files.get(path) {
                files.insert(path.clone(), source.clone());
            } else {
                files.remove(path);
            }
        }
        let disk_externs: Vec<_> = externs.into_iter().chain(dependency_externs).collect();
        let active_externs = if disk.externs_overridden {
            &self.externs
        } else {
            &disk_externs
        };
        let active_paths = extern_paths(active_externs, |path| files.contains_key(path))?;

        for path in self.files.keys().filter(|path| !files.contains_key(*path)) {
            self.session.forget_file(path);
        }
        for file in self
            .externs
            .iter()
            .filter(|file| !active_paths.contains(file.path.as_str()))
        {
            self.session.forget_file(&file.path);
        }
        self.files = files;
        if !disk.externs_overridden {
            self.externs = disk_externs;
        }
        Ok(())
    }

    fn update(
        &mut self,
        changes: Vec<FileSnapshot>,
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
                    return Err(extern_path_conflict(&file.path));
                }
            }
            ExternsChange::Replace(files) => {
                let paths = extern_paths(files, |path| {
                    updated.contains(path)
                        || (!removed.contains(path) && self.files.contains_key(path))
                })?;
                for previous in &self.externs {
                    if !paths.contains(previous.path.as_str()) {
                        self.session.forget_file(&previous.path);
                    }
                }
            }
        }

        if let Some(disk) = &mut self.disk {
            disk.overlays.extend(remove.iter().cloned());
            disk.overlays
                .extend(changes.iter().map(|file| file.path.clone()));
            disk.externs_overridden |= matches!(externs, ExternsChange::Replace(_));
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
        let externs: Vec<_> = self.externs.iter().map(FileSnapshot::as_source).collect();
        self.session.check_project_with_externs(&sources, &externs)
    }
}

fn extern_paths(
    files: &[FileSnapshot],
    mut is_source: impl FnMut(&str) -> bool,
) -> Result<BTreeSet<&str>, String> {
    let mut paths = BTreeSet::new();
    for file in files {
        if !paths.insert(file.path.as_str()) {
            return Err(format!("Duplicate externs path {:?}", file.path));
        }
        if is_source(&file.path) {
            return Err(extern_path_conflict(&file.path));
        }
    }
    Ok(paths)
}

fn extern_path_conflict(path: &str) -> String {
    format!("Externs path {path:?} is also a source path")
}

#[derive(Serialize)]
struct CheckResponse<T> {
    id: Value,
    result: T,
}

#[derive(Serialize)]
struct HoverResponse {
    #[serde(flatten)]
    info: VarInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    arglists: Vec<String>,
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
        ProjectOperation::OpenProject(request) => {
            match TrackedProject::open_project(&request.root, request.aliases, request.classpath) {
                Ok(project) => write_check(output, request.id, tracked.insert(project).check()),
                Err(message) => write_error(output, request.id, message),
            }
        }
        ProjectOperation::RefreshProject(request) => match tracked {
            Some(project) => match project.refresh_disk() {
                Ok(()) => write_check(output, request.id, project.check()),
                Err(message) => write_error(output, request.id, message),
            },
            None => write_error(
                output,
                request.id,
                "Open a disk project before refreshing".into(),
            ),
        },
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
        ProjectOperation::Hover(request) => match tracked {
            Some(project) => write_check(
                output,
                request.id,
                project
                    .session
                    .hover_at(&request.path, request.offset)
                    .map(|hover| HoverResponse {
                        info: hover.info,
                        documentation: hover.documentation,
                        arglists: hover.arglists,
                    }),
            ),
            None => write_error(
                output,
                request.id,
                "Open a project before querying hover information".into(),
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
                let sources: Vec<_> = request.files.iter().map(FileSnapshot::as_source).collect();
                let externs: Vec<_> = request
                    .externs
                    .iter()
                    .map(FileSnapshot::as_source)
                    .collect();
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
