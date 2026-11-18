//! Project snapshots loaded from disk for CLI, editor and agent consumers.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use parenox::Source;
use serde::Deserialize;

use crate::config::ProjectConfig;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileSnapshot {
    pub path: String,
    pub source: String,
}

impl FileSnapshot {
    pub fn as_source(&self) -> Source<'_> {
        Source {
            path: &self.path,
            source: &self.source,
        }
    }
}

pub(super) struct DiskProject {
    pub files: BTreeMap<String, String>,
    pub externs: Vec<FileSnapshot>,
    pub dependency_externs: Vec<FileSnapshot>,
}

impl DiskProject {
    pub fn read(settings: ProjectConfig) -> Result<Self, String> {
        Ok(Self {
            files: read_source_roots(&settings.paths)?,
            externs: read_externs(&settings.externs)?,
            dependency_externs: read_externs(&settings.dependency_externs)?,
        })
    }

    pub fn require_source_files(&self) -> Result<(), String> {
        if self.files.is_empty() {
            return Err("No Clojure source files found".into());
        }
        Ok(())
    }
}

fn read_externs(paths: &[String]) -> Result<Vec<FileSnapshot>, String> {
    paths
        .iter()
        .map(|path| {
            fs::read_to_string(path)
                .map(|source| FileSnapshot {
                    path: path.clone(),
                    source,
                })
                .map_err(|error| format!("{path}: {error}"))
        })
        .collect()
}

pub(super) fn read_source_roots(paths: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();
    let mut pending: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    while let Some(path) = pending.pop() {
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            for child in fs::read_dir(&path).map_err(|error| error.to_string())? {
                let child = child.map_err(|error| error.to_string())?.path();
                if child
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                pending.push(child);
            }
        } else if is_clojure_source(&path) {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            files.insert(path.to_string_lossy().into_owned(), source);
        }
    }
    Ok(files)
}

pub(super) fn is_clojure_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("clj" | "cljc")
    )
}
