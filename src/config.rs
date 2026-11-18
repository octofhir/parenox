//! Inert EDN project settings for command-line checks.
mod classpath;
mod deps;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use parenox::reader::{self, Form, Kind, Span};

use crate::cli::ProjectOptions;

const DEFAULT_PATH: &str = "parenox.edn";
pub(super) use deps::deps_paths;

#[derive(Default)]
pub(super) struct ProjectConfig {
    pub paths: Vec<String>,
    pub externs: Vec<String>,
    pub dependency_externs: Vec<String>,
}

impl ProjectOptions {
    pub(super) fn load(&self) -> Result<ProjectConfig, String> {
        let settings = self.load_at(Path::new(""))?;
        if settings.paths.is_empty() {
            return Err(
                "No source paths. Specify paths or configure :paths in parenox.edn or deps.edn"
                    .into(),
            );
        }
        Ok(settings)
    }

    pub(super) fn manifests(&self, root: &Path) -> [PathBuf; 2] {
        let config = self
            .config
            .as_ref()
            .map_or_else(|| root.join(DEFAULT_PATH), |path| root.join(path));
        let deps = self.deps.as_ref().map_or_else(
            || config.parent().unwrap_or(root).join("deps.edn"),
            |path| root.join(path),
        );
        [config, deps]
    }

    pub(super) fn load_at(&self, root: &Path) -> Result<ProjectConfig, String> {
        let [config, manifest] = self.manifests(root);
        if self.classpath.is_some()
            && (!self.paths.is_empty() || self.deps.is_some() || !self.aliases.is_empty())
        {
            return Err(
                "Classpath input cannot be combined with source paths, deps or aliases".into(),
            );
        }
        let mut settings =
            if self.config.is_some() || (self.paths.is_empty() && file_exists(&config)?) {
                ProjectConfig::read(&config)?
            } else {
                ProjectConfig::default()
            };
        if let Some(file) = &self.classpath {
            settings.paths = classpath::directories(root, &root.join(file))?;
        } else if !self.paths.is_empty() {
            settings.paths = self
                .paths
                .iter()
                .map(|path| root.join(path).to_string_lossy().into_owned())
                .collect();
        } else if self.deps.is_some()
            || !self.aliases.is_empty()
            || (settings.paths.is_empty() && file_exists(&manifest)?)
        {
            settings.paths = deps_paths(&manifest, &self.aliases)?;
        }
        Ok(settings)
    }
}

fn file_exists(path: &Path) -> Result<bool, String> {
    path.try_exists()
        .map_err(|error| format!("{}: {error}", path.display()))
}

impl ProjectConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let entries = read_map(path)?;
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let mut config = Self::default();
        let mut seen = BTreeSet::new();
        for [key, value] in entries.as_chunks::<2>().0 {
            let Kind::Keyword(name) = &key.kind else {
                return Err(invalid(
                    path,
                    key.span,
                    "configuration keys must be keywords",
                ));
            };
            let slot = match name.as_str() {
                ":paths" => &mut config.paths,
                ":externs" => &mut config.externs,
                ":dependency-externs" => &mut config.dependency_externs,
                _ => {
                    return Err(invalid(
                        path,
                        key.span,
                        &format!("unknown configuration key {name}"),
                    ));
                }
            };
            if !seen.insert(name) {
                return Err(invalid(
                    path,
                    key.span,
                    &format!("duplicate configuration key {name}"),
                ));
            }
            let Kind::Vector(values) = &value.kind else {
                return Err(invalid(
                    path,
                    value.span,
                    &format!("{name} must be a vector of paths"),
                ));
            };
            for entry in values {
                if let Kind::String(value) = &entry.kind
                    && !value.is_empty()
                {
                    slot.push(parent.join(value).to_string_lossy().into_owned());
                } else {
                    return Err(invalid(
                        path,
                        entry.span,
                        &format!("{name} requires non-empty path strings"),
                    ));
                }
            }
        }
        Ok(config)
    }
}

fn read_map(path: &Path) -> Result<Vec<Form>, String> {
    let source = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut forms = reader::read(&source).map_err(|err| invalid(path, err.span, &err.message))?;
    if forms.len() == 1
        && let Kind::Map(entries) = forms.remove(0).kind
    {
        return Ok(entries);
    }
    Err(format!(
        "{}: expected one EDN configuration map",
        path.display()
    ))
}

fn invalid(path: &Path, span: Span, message: &str) -> String {
    format!(
        "{}:{}:{}: {message}",
        path.display(),
        span.start.line,
        span.start.column
    )
}
