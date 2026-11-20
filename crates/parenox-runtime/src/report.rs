//! Collect and decode EDN summaries from runtime subprocesses.
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

use parenox_frontend::reader::{self, Form, Kind, Span};
use serde::Serialize;

/// Execute a configured runtime command and route its program output.
pub type CommandExecutor<'a> = &'a mut dyn FnMut(Command) -> Result<ExitStatus, RunError>;

pub(super) fn execute_report(
    command: Command,
    result: &Path,
    name: &str,
    runtime: &str,
    launcher: &str,
    redirect_stdout: bool,
    executor: Option<CommandExecutor<'_>>,
) -> Result<String, RunError> {
    let status = if let Some(execute) = executor {
        execute(command)?
    } else {
        wait_for_report(command, name, runtime, launcher, redirect_stdout)?
    };
    if !status.success() {
        let (phase, message) = if result.is_file() {
            (
                Phase::Execution,
                format!("{runtime} {name} run did not complete"),
            )
        } else {
            (
                Phase::Launch,
                format!("{launcher} did not start the {name} runner"),
            )
        };
        return Err(RunError::new(phase, format!("{message} ({status})")));
    }
    let result = fs::read_to_string(result).map_err(|error| {
        RunError::new(
            Phase::Report,
            format!("{runtime} exited without a {name} result ({error})"),
        )
    })?;
    if result == ":started" {
        return Err(RunError::new(
            Phase::Execution,
            format!("{runtime} exited before reporting {name} results"),
        ));
    }
    Ok(result)
}

fn wait_for_report(
    mut command: Command,
    name: &str,
    runtime: &str,
    launcher: &str,
    redirect_stdout: bool,
) -> Result<ExitStatus, RunError> {
    if redirect_stdout {
        command.stdout(Stdio::piped());
    }
    let mut child = command.spawn().map_err(|error| {
        RunError::new(
            Phase::Launch,
            format!(
                "could not start {}: {error}",
                command.get_program().display()
            ),
        )
    })?;
    // Drain the runtime log before waiting so a large log can keep flowing.
    let log = child
        .stdout
        .take()
        .map(|mut stdout| io::copy(&mut stdout, &mut io::stderr().lock()))
        .transpose();
    let status = child.wait().map_err(|error| {
        RunError::new(Phase::Execution, format!("wait for {launcher}: {error}"))
    })?;
    log.map_err(|error| {
        RunError::new(
            Phase::Execution,
            format!("forward {runtime} {name} output: {error}"),
        )
    })?;
    Ok(status)
}

#[derive(Serialize)]
pub struct TestReport {
    version: u32,
    pub ok: bool,
    pub tests: usize,
    assertions: usize,
    failures: usize,
    errors: usize,
    findings: Vec<TestFinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_error: Option<RunError>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Project,
    Selection,
    Launch,
    Load,
    Execution,
    Report,
}

#[derive(Serialize)]
pub struct RunError {
    phase: Phase,
    message: String,
    #[serde(skip)]
    exit_code: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exception: Option<Box<TestException>>,
}

#[derive(Serialize)]
struct TestFinding {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    test: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exception: Option<TestException>,
}

#[derive(Serialize)]
struct TestException {
    class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl TestReport {
    pub fn incomplete(error: RunError) -> Self {
        Self {
            version: 1,
            ok: false,
            tests: 0,
            assertions: 0,
            failures: 0,
            errors: 0,
            findings: Vec::new(),
            run_error: Some(error),
        }
    }

    pub(super) fn read(source: &str) -> Option<Self> {
        let forms = reader::read(source).ok()?;
        let [summary] = forms.as_slice() else {
            return None;
        };
        let fields = summary.vector()?;
        if let Some(error) = read_run_error(source, fields) {
            return Some(Self::incomplete(error));
        }
        let [tests, assertions, failures, errors, findings] = fields else {
            return None;
        };
        let tests = count(source, tests)?;
        let failures = count(source, failures)?;
        let errors = count(source, errors)?;
        Some(Self {
            version: 1,
            ok: tests > 0 && failures == 0 && errors == 0,
            tests,
            assertions: count(source, assertions)?,
            failures,
            errors,
            findings: findings
                .vector()?
                .iter()
                .map(|finding| TestFinding::read(source, finding))
                .collect::<Option<Vec<_>>>()?,
            run_error: None,
        })
    }
}

pub(super) fn read_run_error(source: &str, fields: &[Form]) -> Option<RunError> {
    let [marker, phase, message, path, line, column, exception] = fields else {
        return None;
    };
    if marker.keyword() != Some(":run-error") {
        return None;
    }
    Some(RunError {
        phase: match phase.keyword()? {
            ":selection" => Phase::Selection,
            ":load" => Phase::Load,
            ":execution" => Phase::Execution,
            _ => return None,
        },
        message: optional_text(message)??,
        exit_code: None,
        path: optional_text(path)?,
        line: optional_count(source, line)?,
        column: optional_count(source, column)?,
        exception: read_exception(exception)?.map(Box::new),
    })
}

impl RunError {
    pub fn new(phase: Phase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
            exit_code: None,
            path: None,
            line: None,
            column: None,
            exception: None,
        }
    }

    pub fn at_file(mut self, path: String, span: Option<Span>) -> Self {
        self.path = Some(path);
        self.line = span.map(|span| span.start.line);
        self.column = span.map(|span| span.start.column);
        self
    }

    pub fn with_exit_code(mut self, code: u8) -> Self {
        self.exit_code = Some(code);
        self
    }

    pub fn exit_code(&self) -> Option<u8> {
        self.exit_code
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            f.write_str(path)?;
            if let Some(line) = self.line {
                write!(f, ":{line}")?;
                if let Some(column) = self.column {
                    write!(f, ":{column}")?;
                }
            }
            f.write_str(": ")?;
        }
        f.write_str(&self.message)
    }
}

impl TestFinding {
    fn read(source: &str, form: &Form) -> Option<Self> {
        let [kind, test, path, line, message, expected, actual, exception] = form.vector()? else {
            return None;
        };
        Some(Self {
            kind: match kind.keyword()? {
                ":fail" => "fail",
                ":error" => "error",
                _ => return None,
            },
            test: optional_text(test)?,
            path: optional_text(path)?,
            line: optional_count(source, line)?,
            message: optional_text(message)?,
            expected: optional_text(expected)?,
            actual: optional_text(actual)?,
            exception: read_exception(exception)?,
        })
    }
}

fn read_exception(form: &Form) -> Option<Option<TestException>> {
    if matches!(form.kind, Kind::Nil) {
        return Some(None);
    }
    let [class, message] = form.vector()? else {
        return None;
    };
    Some(Some(TestException {
        class: optional_text(class)??,
        message: optional_text(message)?,
    }))
}

fn optional_count(source: &str, form: &Form) -> Option<Option<usize>> {
    if matches!(form.kind, Kind::Nil) {
        Some(None)
    } else {
        Some(Some(count(source, form)?))
    }
}

fn count(source: &str, form: &Form) -> Option<usize> {
    if !matches!(form.kind, Kind::Number) {
        return None;
    }
    source[form.span.start.offset..form.span.end.offset]
        .parse()
        .ok()
}

pub(super) fn optional_text(form: &Form) -> Option<Option<String>> {
    match &form.kind {
        Kind::Nil => Some(None),
        Kind::String(value) => Some(Some(value.clone())),
        Kind::Utf16String(value) => Some(Some(String::from_utf16_lossy(value))),
        _ => None,
    }
}
