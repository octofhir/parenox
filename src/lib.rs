mod checker;
pub mod output;
mod project;
pub mod reader;
mod types;

use reader::Span;
use serde::Serialize;

pub use project::{ProjectReport, Session, Source};

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: &'static str,
    pub span: Span,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<Suggestion>,
}

/// A proposed edit, never applied automatically. Offsets refer to the report's source snapshot.
#[derive(Clone, Debug, Serialize)]
pub struct Suggestion {
    pub message: String,
    pub span: Span,
    pub replacement: String,
    pub applicability: &'static str,
}

impl Diagnostic {
    fn new(code: &str, span: Span, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: "error",
            span,
            message: message.into(),
            expected: None,
            actual: None,
            notes: Vec::new(),
            help: None,
            suggestions: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Stats {
    pub elapsed_us: u64,
    pub parsed_files: usize,
    pub reused_parses: usize,
    pub checked_functions: usize,
    pub reused_functions: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub version: u32,
    pub path: String,
    /// Session fingerprint, not a cryptographic digest or a persistent cache key.
    pub source_hash: String,
    pub ok: bool,
    pub complete: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub stats: Stats,
}
