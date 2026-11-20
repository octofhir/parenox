//! Inert EDN project settings for command-line checks.
mod deps;

use std::fs;
use std::path::{Path, PathBuf};

use parenox::formatter::{FormatOptions, IndentRule};
use parenox::reader::{self, Form, Kind, Span};

use crate::cli::ProjectOptions;

const DEFAULT_PATH: &str = "parenox.edn";
pub(super) use deps::deps_paths;

#[derive(Default)]
pub(super) struct ProjectConfig {
    pub paths: Vec<String>,
    pub classpath: Option<Vec<PathBuf>>,
    pub dependency_sources: Vec<String>,
    pub externs: Vec<String>,
    pub dependency_externs: Vec<String>,
    pub format: FormatOptions,
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
        self.load_context(root, false)
    }

    pub(super) fn dependency_jars(&self, root: &Path) -> Vec<PathBuf> {
        let [_, manifest] = self.manifests(root);
        parenox_classpath::manifest_jars(&manifest, &self.aliases)
    }

    pub(super) fn load_for_format(&self, root: &Path) -> Result<ProjectConfig, String> {
        self.load_context(root, true)
    }

    fn load_context(
        &self,
        root: &Path,
        read_default_config: bool,
    ) -> Result<ProjectConfig, String> {
        let [config, manifest] = self.manifests(root);
        if self.classpath.is_some()
            && (!self.paths.is_empty() || self.deps.is_some() || !self.aliases.is_empty())
        {
            return Err(
                "Classpath input cannot be combined with source paths, deps or aliases".into(),
            );
        }
        let mut settings = if self.config.is_some()
            || ((read_default_config || self.paths.is_empty()) && file_exists(&config)?)
        {
            ProjectConfig::read(&config)?
        } else {
            ProjectConfig::default()
        };
        if let Some(file) = &self.classpath {
            let path = root.join(file);
            let entries = parenox_classpath::explicit_entries(root, &path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            settings.paths = entries
                .iter()
                .filter(|entry| entry.is_dir())
                .map(|entry| entry.to_string_lossy().into_owned())
                .collect();
            settings.classpath = Some(entries);
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
    pub fn analysis_paths(&self) -> impl Iterator<Item = &String> {
        self.paths.iter().chain(&self.dependency_sources)
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let (source, entries) = read_map_with_source(path)?;
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let mut config = Self::default();
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
                ":dependency-sources" => &mut config.dependency_sources,
                ":externs" => &mut config.externs,
                ":dependency-externs" => &mut config.dependency_externs,
                ":format" => {
                    config.format = format_options(path, &source, value)?;
                    continue;
                }
                _ => {
                    return Err(invalid(
                        path,
                        key.span,
                        &format!("unknown configuration key {name}"),
                    ));
                }
            };
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

fn format_options(path: &Path, source: &str, form: &Form) -> Result<FormatOptions, String> {
    let Some(entries) = form.map() else {
        return Err(invalid(path, form.span, ":format must be an EDN map"));
    };
    let mut options = FormatOptions::default();
    for [key, value] in entries.as_chunks::<2>().0 {
        match key.keyword() {
            Some(":indent-width") => {
                let digits = &source[value.span.start.offset..value.span.end.offset];
                options.indent_width = digits
                    .parse::<u8>()
                    .ok()
                    .filter(|width| (1..=8).contains(width))
                    .ok_or_else(|| {
                        invalid(
                            path,
                            value.span,
                            ":indent-width must be an integer from 1 to 8",
                        )
                    })?;
            }
            Some(":insert-final-newline?") => {
                let Kind::Bool(enabled) = value.kind else {
                    return Err(invalid(
                        path,
                        value.span,
                        ":insert-final-newline? must be a Boolean",
                    ));
                };
                options.insert_final_newline = enabled;
            }
            Some(":indent-rules") => {
                let Some(rules) = value.map() else {
                    return Err(invalid(
                        path,
                        value.span,
                        ":indent-rules must be an EDN map",
                    ));
                };
                for [name, rule] in rules.as_chunks::<2>().0 {
                    let Some(name) = name.symbol() else {
                        return Err(invalid(path, name.span, "indent rule keys must be symbols"));
                    };
                    let rule = match rule.keyword() {
                        Some(":block") => IndentRule::Block,
                        Some(":align") => IndentRule::Align,
                        _ => {
                            return Err(invalid(
                                path,
                                rule.span,
                                "indent rules must be :block or :align",
                            ));
                        }
                    };
                    options.indent_rules.insert(name.into(), rule);
                }
            }
            _ => return Err(invalid(path, key.span, "unknown :format setting")),
        }
    }
    Ok(options)
}

fn read_map(path: &Path) -> Result<Vec<Form>, String> {
    read_map_with_source(path).map(|(_, forms)| forms)
}

fn read_map_with_source(path: &Path) -> Result<(String, Vec<Form>), String> {
    let source = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut forms =
        reader::read_edn(&source).map_err(|err| invalid(path, err.span, &err.message))?;
    if forms.len() == 1
        && let Kind::Map(entries) = forms.remove(0).kind
    {
        return Ok((source, entries));
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
