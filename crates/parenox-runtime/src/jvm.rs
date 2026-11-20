use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::project::{RunDirectory, execution_path, project_root};
use super::report::{CommandExecutor, Phase, RunError, execute_report};
use super::{namespace_symbol, single_form};

pub struct JvmProject {
    root: PathBuf,
    executable: PathBuf,
    aliases: String,
}

pub enum JvmMain<'a> {
    Namespace(&'a str),
    Script(&'a Path),
    Alias,
}

impl JvmProject {
    pub fn open(project: &Path, clojure: &Path, aliases: &[String]) -> Result<Self, RunError> {
        let root = project_root(project)?;
        if !root.join("deps.edn").is_file() {
            return Err(RunError::new(
                Phase::Project,
                format!("{}: expected a deps.edn project", root.display()),
            ));
        }
        Ok(Self {
            root,
            executable: execution_path(clojure)?,
            aliases: main_aliases(aliases)?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Build a Clojure CLI application invocation with inherited standard streams.
    pub fn run_command(&self, main: JvmMain<'_>, args: &[OsString]) -> Result<Command, RunError> {
        let mut command = self.command();
        command.arg(&self.aliases);
        match main {
            JvmMain::Namespace(name) if namespace_symbol(name) => {
                command.args(["-m", name]);
            }
            JvmMain::Namespace(name) => {
                return Err(RunError::new(
                    Phase::Selection,
                    format!("Expected a Clojure namespace, got {name:?}"),
                ));
            }
            JvmMain::Script(path) => {
                if !self.root.join(path).is_file() {
                    return Err(RunError::new(
                        Phase::Selection,
                        format!("{}: expected a script file", path.display()),
                    ));
                }
                if path.as_os_str().as_encoded_bytes().starts_with(b"-")
                    || path.as_os_str().as_encoded_bytes().starts_with(b"@")
                {
                    command.arg(Path::new(".").join(path));
                } else {
                    command.arg(path);
                }
            }
            JvmMain::Alias if self.aliases == "-M" => {
                return Err(RunError::new(
                    Phase::Selection,
                    "Specify a script, --main NAMESPACE, or --alias NAME",
                ));
            }
            JvmMain::Alias => {}
        }
        command.args(args);
        Ok(command)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.current_dir(&self.root);
        command
    }

    pub(super) fn report_runner(&self, name: &'static str) -> Result<ReportRunner<'_>, RunError> {
        let alias = format!("parenox/{name}-runner");
        if self.aliases.split(':').skip(1).any(|name| name == alias) {
            return Err(RunError::new(
                Phase::Selection,
                format!(":{alias} is reserved"),
            ));
        }
        Ok(ReportRunner {
            project: self,
            name,
            aliases: format!("{}:{alias}", self.aliases),
            deps: format!("{{:aliases {{:{alias} {{:main-opts []}}}}}}"),
        })
    }
}

fn main_aliases(aliases: &[String]) -> Result<String, RunError> {
    let mut combined = String::from("-M");
    for alias in aliases {
        let name = alias.strip_prefix(':').unwrap_or(alias);
        let keyword = format!(":{name}");
        if name.is_empty()
            || name.contains(':')
            || !single_form(&keyword).is_some_and(|form| form.keyword() == Some(&keyword))
        {
            return Err(RunError::new(
                Phase::Selection,
                format!("{alias:?} is not a valid Clojure CLI alias"),
            ));
        }
        combined.push_str(&keyword);
    }
    Ok(combined)
}

pub(super) struct ReportRunner<'a> {
    project: &'a JvmProject,
    name: &'static str,
    aliases: String,
    deps: String,
}

impl ReportRunner<'_> {
    pub fn run<'a>(
        &self,
        source: &str,
        args: impl IntoIterator<Item = &'a str>,
        redirect_stdout: bool,
        executor: Option<CommandExecutor<'_>>,
    ) -> Result<String, RunError> {
        let name = self.name;
        let run = RunDirectory::create(name).map_err(|error| {
            RunError::new(
                Phase::Launch,
                format!("create {name} run directory: {error}"),
            )
        })?;
        let script = run.0.join(format!("clojure_{name}.clj"));
        let result = run.0.join("result");
        fs::write(&script, source).map_err(|error| {
            RunError::new(Phase::Launch, format!("write {name} runner: {error}"))
        })?;
        let mut command = self.project.command();
        command
            .args(["-Sdeps", &self.deps, &self.aliases])
            .arg(script)
            .arg(&result)
            .args(args);
        execute_report(
            command,
            &result,
            name,
            "Clojure",
            "Clojure CLI",
            redirect_stdout,
            executor,
        )
    }
}
