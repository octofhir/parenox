mod checker;
pub mod expand;
mod extern_diff;
mod project;
mod suppressions;
mod types;

pub use parenox_frontend::{reader, syntax};

use reader::Span;
use serde::Serialize;

pub use extern_diff::{
    ExternChange, ExternChangeKind, ExternDefinition, ExternDiff, ExternMismatch,
    ExternMismatchReason, ExternPathStep, ExternSnapshot,
};
pub use project::{
    CallHierarchyCalls, CallHierarchyItem, CallSignature, CallSignatures, Definition, ExternExport,
    FdefSuggestion, ProjectReport, SemanticSymbol, SemanticSymbolKind, Session, Source, SourceEdit,
    SourceSymbol, SourceSymbolKind, SourceTestKind, SourceTestNode, VarInfo, compare_externs,
};

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    /// Occurrence identity assigned in a check report, not a suppression rule code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub code: String,
    pub severity: &'static str,
    pub span: Span,
    pub message: String,
    #[serde(skip)]
    headline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<ContractOrigin>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<Suggestion>,
}

/// The contract used to check an argument or a result.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ContractOrigin {
    pub name: String,
    pub kind: ContractKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    TSig,
    SFdef,
    Extern,
    ExternLiteral,
    Builtin,
    MacroScope,
}

impl ContractOrigin {
    fn builtin(name: &str) -> Self {
        Self {
            name: name.into(),
            kind: ContractKind::Builtin,
            path: None,
        }
    }
}

/// A proposed edit, never applied automatically. Offsets refer to the report's source snapshot.
#[derive(Clone, Debug, Serialize)]
pub struct Suggestion {
    pub message: String,
    pub span: Span,
    pub replacement: String,
    pub applicability: &'static str,
}

impl From<reader::ReadError> for Diagnostic {
    fn from(error: reader::ReadError) -> Self {
        Self::new("PNX9000", error.span, error.message)
    }
}

impl Diagnostic {
    pub fn title(&self) -> &str {
        self.headline.as_deref().unwrap_or(&self.message)
    }

    fn type_mismatch(span: Span, context: &str, expected: String, actual: String) -> Self {
        let mut diagnostic = Self::new(
            "PNX2322",
            span,
            format!("{context}: expected {expected}, got {actual}"),
        );
        diagnostic.headline = Some(format!("{context}: mismatched types"));
        diagnostic.expected = Some(expected);
        diagnostic.actual = Some(actual);
        diagnostic
    }

    fn new(code: &str, span: Span, message: impl Into<String>) -> Self {
        Self {
            id: None,
            code: code.into(),
            severity: match code {
                "PNX9002" | "PNX9009" => "warning",
                _ => "error",
            },
            span,
            message: message.into(),
            headline: None,
            expected: None,
            actual: None,
            contract: None,
            notes: Vec::new(),
            help: None,
            suggestions: Vec::new(),
        }
    }

    fn analysis_gap(code: &str, span: Span, message: impl Into<String>) -> Self {
        let mut diagnostic = Self::new(code, span, message);
        diagnostic.severity = "warning";
        diagnostic
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Stats {
    pub elapsed_us: u64,
    pub parsed_files: usize,
    pub reused_parses: usize,
    /// Annotated definitions checked as roots; inferred private work belongs to their callers.
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suppressed: Vec<SuppressedDiagnostic>,
    pub stats: Stats,
}

/// A finding explicitly suppressed at a source form; the original occurrence ID is retained.
#[derive(Debug, Serialize)]
pub struct SuppressedDiagnostic {
    pub diagnostic: Diagnostic,
    pub reason: String,
    pub directive: Span,
}
