use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::namespace_symbol;
use super::project::{execution_path, project_root};
use super::report::{Phase, RunError};

pub struct Clojurust {
    root: PathBuf,
    executable: PathBuf,
    source_paths: Vec<PathBuf>,
}

impl Clojurust {
    pub fn open(
        project: &Path,
        executable: &Path,
        source_paths: &[PathBuf],
    ) -> Result<Self, RunError> {
        let root = project_root(project)?;
        let source_paths: Vec<_> = source_paths.iter().map(|path| root.join(path)).collect();
        for path in &source_paths {
            if !path.is_dir() {
                return Err(RunError::new(
                    Phase::Project,
                    format!("{}: expected a clojurust source directory", path.display()),
                ));
            }
        }
        Ok(Self {
            root,
            executable: execution_path(executable)?,
            source_paths,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn source_paths(&self) -> &[PathBuf] {
        &self.source_paths
    }

    /// Build a portable script invocation using only explicitly specified source roots.
    pub fn run_command(&self, script: &Path, args: &[OsString]) -> Result<Command, RunError> {
        if !matches!(
            script.extension().and_then(|ext| ext.to_str()),
            Some("cljc" | "cljrs")
        ) {
            return Err(RunError::new(
                Phase::Selection,
                format!(
                    "{}: clojurust run expects a .cljc or .cljrs script",
                    script.display()
                ),
            ));
        }
        let script = self.root.join(script);
        if !script.is_file() {
            return Err(RunError::new(
                Phase::Selection,
                format!("{}: expected a script file", script.display()),
            ));
        }
        let mut command = self.command("run");
        command.arg(script);
        if !args.is_empty() {
            command.arg("--").args(args);
        }
        Ok(command)
    }

    /// Run explicitly selected clojure.test namespaces through clojurust.
    pub fn test_command(&self, namespaces: &[String]) -> Result<Command, RunError> {
        validate_namespaces(namespaces)?;
        let mut command = self.command("test");
        command.arg("--").args(namespaces);
        Ok(command)
    }

    fn command(&self, subcommand: &str) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .current_dir(&self.root)
            .args(["--no-config", "--no-native", subcommand]);
        for path in &self.source_paths {
            command.arg("--src-path").arg(path);
        }
        command
    }
}

pub(super) fn validate_namespaces(namespaces: &[String]) -> Result<(), RunError> {
    if namespaces.is_empty() {
        return Err(RunError::new(Phase::Selection, "Specify a test namespace"));
    }
    for name in namespaces {
        if !namespace_symbol(name) {
            return Err(RunError::new(
                Phase::Selection,
                format!("Expected a Clojure namespace, got {name:?}"),
            ));
        }
    }
    Ok(())
}
