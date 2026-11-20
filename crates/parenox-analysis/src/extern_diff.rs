//! Public results for comparing two declared library surfaces.
pub use parenox_types::{ExternMismatch, ExternMismatchReason, ExternPathStep};
use serde::Serialize;

use crate::reader::Span;
use crate::{ContractOrigin, ProjectReport, Source};

#[derive(Clone, Copy)]
pub struct ExternSnapshot<'a> {
    /// Optional source files that provide named specs. Bodies are not checked.
    pub sources: &'a [Source<'a>],
    pub externs: &'a [Source<'a>],
}

#[derive(Debug, Serialize)]
pub struct ExternDiff {
    pub version: u32,
    /// True only when all promises from the old snapshot are still covered.
    pub compatible: bool,
    /// False if either snapshot is invalid or any comparison is unproven.
    pub complete: bool,
    pub before: ProjectReport,
    pub after: ProjectReport,
    pub changes: Vec<ExternChange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternChangeKind {
    Added,
    Removed,
    Compatible,
    Breaking,
    ValueChanged,
    Unproven,
}

impl ExternChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Compatible => "compatible",
            Self::Breaking => "breaking",
            Self::ValueChanged => "value changed",
            Self::Unproven => "unproven",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExternChange {
    pub name: String,
    pub kind: ExternChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<ExternMismatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<ExternDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<ExternDefinition>,
}

#[derive(Debug, Serialize)]
pub struct ExternDefinition {
    pub origin: ContractOrigin,
    pub span: Span,
    pub declaration: String,
}
