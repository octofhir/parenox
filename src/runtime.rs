//! Explicit Clojure CLI invocations. Static analysis remains source-only.
mod discover;
mod report;

use std::env;
use std::fs::{self, DirBuilder};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use parenox::reader::{self, Form};

use crate::cli::{TestFormat, TestOptions};
use report::{Phase, RunError, TestReport};

const RUNNER: &str = include_str!("runtime/clojure_test.clj");
const RUNNER_ALIAS: &str = "parenox/test-runner";
const RUNNER_DEPS: &str = "{:aliases {:parenox/test-runner {:main-opts []}}}";

pub(super) fn run_tests(options: TestOptions) -> Result<u8, String> {
    let format = options.format;
    let report = execute_tests(options).unwrap_or_else(TestReport::incomplete);
    if format == TestFormat::Json {
        println!(
            "{}",
            serde_json::to_string(&report).map_err(|error| error.to_string())?
        );
    }
    if let Some(error) = report.run_error {
        return Err(error.to_string());
    }
    Ok(u8::from(!report.ok))
}

fn execute_tests(options: TestOptions) -> Result<TestReport, RunError> {
    let project = options.project.canonicalize().map_err(|error| {
        RunError::new(
            Phase::Project,
            format!("{}: {error}", options.project.display()),
        )
    })?;
    if !project.join("deps.edn").is_file() {
        return Err(RunError::new(
            Phase::Project,
            format!("{}: expected a deps.edn project", project.display()),
        ));
    }
    let aliases =
        cli_aliases(&options.aliases).map_err(|error| RunError::new(Phase::Selection, error))?;
    let (mode, selectors) = if options.discover {
        (
            "namespaces",
            discover::namespaces(&project, &options.aliases)?,
        )
    } else if options.vars.is_empty() {
        for name in &options.namespaces {
            if !namespace_symbol(name) {
                return Err(RunError::new(
                    Phase::Selection,
                    format!("Expected a Clojure namespace, got {name:?}"),
                ));
            }
        }
        ("namespaces", options.namespaces)
    } else {
        for name in &options.vars {
            let qualified = name.split_once('/').is_some_and(|(namespace, local)| {
                namespace_symbol(namespace)
                    && !local.is_empty()
                    && (local == "/" || !local.contains(['/', ':']))
                    && single_form(name).is_some_and(|form| form.symbol() == Some(name))
            });
            if !qualified {
                return Err(RunError::new(
                    Phase::Selection,
                    format!("Expected a qualified Clojure test var, got {name:?}"),
                ));
            }
        }
        ("vars", options.vars)
    };

    let executable =
        if options.clojure.is_relative() && options.clojure.parent() != Some(Path::new("")) {
            env::current_dir()
                .map_err(|error| RunError::new(Phase::Launch, error.to_string()))?
                .join(options.clojure)
        } else {
            options.clojure
        };
    let run = RunDirectory::create().map_err(|error| {
        RunError::new(Phase::Launch, format!("create test run directory: {error}"))
    })?;
    let script = run.0.join("clojure_test.clj");
    let result = run.0.join("result");
    fs::write(&script, RUNNER)
        .map_err(|error| RunError::new(Phase::Launch, format!("write test runner: {error}")))?;
    let mut command = Command::new(&executable);
    command
        .current_dir(&project)
        .args(["-Sdeps", RUNNER_DEPS, &aliases])
        .arg(script)
        .arg(&result)
        .arg(mode)
        .arg(if options.format == TestFormat::Json {
            "json"
        } else {
            "text"
        })
        .args(selectors);
    if options.format == TestFormat::Json {
        command.stdout(Stdio::piped());
    }
    let mut child = command.spawn().map_err(|error| {
        RunError::new(
            Phase::Launch,
            format!("could not start {}: {error}", executable.display()),
        )
    })?;
    // Drain the runtime log before waiting so a large test log can keep flowing.
    let log = child
        .stdout
        .take()
        .map(|mut stdout| io::copy(&mut stdout, &mut io::stderr().lock()))
        .transpose();
    let status = child.wait().map_err(|error| {
        RunError::new(Phase::Execution, format!("wait for Clojure CLI: {error}"))
    })?;
    log.map_err(|error| {
        RunError::new(
            Phase::Execution,
            format!("forward Clojure test output: {error}"),
        )
    })?;
    if !status.success() {
        return Err(RunError::new(
            Phase::Execution,
            format!("Clojure test run did not complete ({status})"),
        ));
    }
    let result = fs::read_to_string(result).map_err(|error| {
        RunError::new(
            Phase::Report,
            format!("Clojure exited without a test result ({error})"),
        )
    })?;
    let report = TestReport::read(&result)
        .ok_or_else(|| RunError::new(Phase::Report, "Clojure returned an invalid test report"))?;
    if report.tests == 0 && report.run_error.is_none() {
        return Err(RunError::new(
            Phase::Execution,
            "no clojure.test vars ran for the requested selection",
        ));
    }
    Ok(report)
}

fn cli_aliases(aliases: &[String]) -> Result<String, String> {
    let mut combined = String::from("-M");
    for alias in aliases {
        let name = alias.strip_prefix(':').unwrap_or(alias);
        let keyword = format!(":{name}");
        if name.is_empty()
            || name.contains(':')
            || name == RUNNER_ALIAS
            || !single_form(&keyword).is_some_and(|form| form.keyword() == Some(&keyword))
        {
            return Err(format!("{alias:?} is not a valid test alias"));
        }
        combined.push_str(&keyword);
    }
    combined.push(':');
    combined.push_str(RUNNER_ALIAS);
    Ok(combined)
}

fn single_form(source: &str) -> Option<Form> {
    let mut forms = reader::read(source).ok()?;
    (forms.len() == 1).then(|| forms.remove(0))
}

fn namespace_symbol(name: &str) -> bool {
    !name.contains(['/', ':']) && single_form(name).is_some_and(|form| form.symbol() == Some(name))
}

struct RunDirectory(PathBuf);

impl RunDirectory {
    fn create() -> io::Result<Self> {
        let root = env::temp_dir().canonicalize()?;
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        for attempt in 0..16 {
            let path = root.join(format!(
                "parenox-test-{}-{started}-{attempt}",
                process::id()
            ));
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "test run directory already exists",
        ))
    }
}

impl Drop for RunDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
