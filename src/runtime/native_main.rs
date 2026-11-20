//! Select an ordinary source file for clojurust's existing namespace entry point.
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use parenox::reader::{self, Form, Kind};
use parenox_runtime::{Clojurust, Phase, RunError, namespace_symbol};

use super::NATIVE_CONDITIONALS;

pub(super) fn resolve(project: &Clojurust, name: &str) -> Result<PathBuf, RunError> {
    let resource = parenox_classpath::namespace_resource(name, false)
        .filter(|_| namespace_symbol(name))
        .ok_or_else(|| {
            RunError::new(
                Phase::Selection,
                format!("Expected a Clojure namespace, got {name:?}"),
            )
        })?;
    if project.source_paths().is_empty() {
        return Err(RunError::new(
            Phase::Selection,
            "--main with clojurust requires at least one --src-path or an existing --deps-paths root",
        ));
    }

    for root in project.source_paths() {
        for extension in ["cljrs", "cljc"] {
            let path = root.join(&resource).with_extension(extension);
            let Some(source) = read_entry(&path)? else {
                continue;
            };
            check_entry(name, &path, &source)?;
            return Ok(path);
        }
    }
    Err(RunError::new(
        Phase::Selection,
        format!("Could not find namespace {name} in the selected source roots"),
    ))
}

fn read_entry(path: &Path) -> Result<Option<String>, RunError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => fs::read_to_string(path)
            .map(Some)
            .map_err(|error| RunError::new(Phase::Project, format!("{}: {error}", path.display()))),
        Ok(_) => Err(RunError::new(
            Phase::Selection,
            format!("{}: expected a namespace source file", path.display()),
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(RunError::new(
            Phase::Project,
            format!("{}: {error}", path.display()),
        )),
    }
}

fn check_entry(name: &str, path: &Path, source: &str) -> Result<(), RunError> {
    let read = reader::read_with(source, NATIVE_CONDITIONALS).map_err(|error| {
        RunError::new(Phase::Selection, error.message)
            .at_file(path.display().to_string(), Some(error.span))
    })?;
    let mut entry = Entry::default();
    entry
        .scan(&read, name)
        .map_err(|error| error.at_file(path.display().to_string(), None))?;
    if !entry.namespace {
        return Err(RunError::new(
            Phase::Selection,
            format!("Expected namespace declaration for {name}"),
        )
        .at_file(path.display().to_string(), None));
    }
    if !entry.main {
        return Err(RunError::new(
            Phase::Selection,
            format!("Expected a direct -main declaration in namespace {name}"),
        )
        .at_file(path.display().to_string(), None));
    }
    Ok(())
}

#[derive(Default)]
struct Entry {
    namespace: bool,
    main: bool,
}

impl Entry {
    fn scan(&mut self, forms: &[Form], name: &str) -> Result<(), RunError> {
        for form in forms {
            let Kind::List(items) = &form.without_metadata().kind else {
                continue;
            };
            let Some((head, args)) = items.split_first() else {
                continue;
            };
            match (head.symbol(), args) {
                (Some("do"), body) => self.scan(body, name)?,
                (Some("ns" | "clojure.core/ns"), [namespace, ..]) => {
                    if namespace.symbol() != Some(name) {
                        return Err(RunError::new(
                            Phase::Selection,
                            format!(
                                "Expected namespace {name}, got a different namespace declaration"
                            ),
                        ));
                    }
                    self.namespace = true;
                }
                (Some("in-ns" | "clojure.core/in-ns"), [namespace]) => {
                    if !matches!(&namespace.without_metadata().kind, Kind::Quote(symbol) if symbol.symbol() == Some(name))
                    {
                        return Err(RunError::new(
                            Phase::Selection,
                            format!("Expected namespace {name}, got an in-ns switch"),
                        ));
                    }
                }
                (
                    Some(
                        "defn"
                        | "defn-"
                        | "def"
                        | "defonce"
                        | "clojure.core/defn"
                        | "clojure.core/defn-"
                        | "clojure.core/def"
                        | "clojure.core/defonce",
                    ),
                    [symbol, _, ..],
                ) if self.namespace && symbol.symbol() == Some("-main") => self.main = true,
                _ => {}
            }
        }
        Ok(())
    }
}
