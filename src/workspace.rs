//! Project snapshots loaded from disk for CLI, editor and agent consumers.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use parenox::Source;
use serde::Deserialize;

use crate::config::ProjectConfig;

#[derive(Deserialize, PartialEq, Eq)]
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
            files: Self::read_sources(&settings)?,
            externs: read_externs(&settings.externs)?,
            dependency_externs: read_externs(&settings.dependency_externs)?,
        })
    }

    pub fn read_sources(settings: &ProjectConfig) -> Result<BTreeMap<String, String>, String> {
        matching_paths(settings.analysis_paths(), is_clojure_source)?
            .into_iter()
            .map(|path| {
                let source = fs::read_to_string(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                Ok((path.to_string_lossy().into_owned(), source))
            })
            .collect()
    }

    pub fn require_source_files(&self) -> Result<(), String> {
        if self.files.is_empty() {
            return Err("No Clojure source files found".into());
        }
        Ok(())
    }

    pub fn sources(&self) -> Vec<Source<'_>> {
        self.files
            .iter()
            .map(|(path, source)| Source { path, source })
            .collect()
    }

    pub fn all_externs(&self) -> Vec<Source<'_>> {
        self.externs
            .iter()
            .chain(&self.dependency_externs)
            .map(FileSnapshot::as_source)
            .collect()
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

pub(super) fn source_paths(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    matching_paths(paths, is_clojure_source)
}

pub(super) fn matching_paths(
    paths: impl IntoIterator<Item = impl AsRef<Path>>,
    include: impl Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut roots = BTreeSet::new();
    let mut pending = Vec::new();
    for path in paths.into_iter().map(|path| path.as_ref().to_owned()) {
        let metadata =
            fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_dir() && !include(&path) {
            continue;
        }
        let resolved = path
            .canonicalize()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if roots.insert(resolved) {
            pending.push((path, Some(metadata)));
        }
    }
    while let Some((path, root_metadata)) = pending.pop() {
        let metadata = match root_metadata {
            Some(metadata) => metadata,
            None => fs::symlink_metadata(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?,
        };
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
                pending.push((child, None));
            }
        } else if include(&path) {
            files.push(path);
        }
    }
    files.sort_unstable();
    files.dedup();
    Ok(files)
}

pub(super) fn is_clojure_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("clj" | "cljc")
    )
}
