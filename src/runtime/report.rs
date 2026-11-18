//! Decode the JVM runner's EDN summary for exit status and machine output.
use std::fmt;

use parenox::reader::{self, Form, Kind};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct TestReport {
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
pub(super) enum Phase {
    Project,
    Selection,
    Launch,
    Load,
    Execution,
    Report,
}

#[derive(Serialize)]
pub(super) struct RunError {
    phase: Phase,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
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

    pub fn read(source: &str) -> Option<Self> {
        let forms = reader::read(source).ok()?;
        let [summary] = forms.as_slice() else {
            return None;
        };
        let fields = summary.vector()?;
        if let [marker, phase, message, path, line, column] = fields
            && marker.keyword() == Some(":run-error")
        {
            let phase = match phase.keyword()? {
                ":selection" => Phase::Selection,
                ":load" => Phase::Load,
                ":execution" => Phase::Execution,
                _ => return None,
            };
            return Some(Self::incomplete(RunError {
                phase,
                message: optional_text(message)??,
                path: optional_text(path)?,
                line: optional_count(source, line)?,
                column: optional_count(source, column)?,
            }));
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

impl RunError {
    pub fn new(phase: Phase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
            path: None,
            line: None,
            column: None,
        }
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
        let [kind, test, path, line, message] = form.vector()? else {
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
        })
    }
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

fn optional_text(form: &Form) -> Option<Option<String>> {
    match &form.kind {
        Kind::Nil => Some(None),
        Kind::String(value) => Some(Some(value.clone())),
        Kind::Utf16String(value) => Some(Some(String::from_utf16_lossy(value))),
        _ => None,
    }
}
