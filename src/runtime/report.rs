//! Decode the JVM runner's EDN summary for exit status and machine output.
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
    pub fn read(source: &str) -> Option<Self> {
        let forms = reader::read(source).ok()?;
        let [summary] = forms.as_slice() else {
            return None;
        };
        let [tests, assertions, failures, errors, findings] = summary.vector()? else {
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
        })
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
            line: if matches!(line.kind, Kind::Nil) {
                None
            } else {
                Some(count(source, line)?)
            },
            message: optional_text(message)?,
        })
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
        _ => None,
    }
}
