//! Public results for comparing two declared library surfaces.
use std::fmt;

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

/// One failed obligation, not a list of all differences between declarations.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ExternMismatch {
    pub path: Vec<ExternPathStep>,
    #[serde(flatten)]
    pub reason: ExternMismatchReason,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExternPathStep {
    /// Arguments are numbered starting at one.
    Argument {
        index: usize,
    },
    Result,
    Field {
        key: String,
    },
    MapKey,
    MapValue,
    Element,
    /// Clojure collection indices start at zero.
    Item {
        index: usize,
    },
    UnionMember,
    CallbackArgument {
        index: usize,
    },
    CallbackRest,
    CallbackResult,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ExternMismatchReason {
    MissingArity,
    /// The actual type no longer fits the expected type at this position.
    /// For arguments this compares the old inputs against the new inputs.
    /// For results it compares the new outputs against the old outputs.
    TypeMismatch {
        expected: String,
        actual: String,
    },
    MissingField,
    OptionalField,
    UndeclaredField,
    UnexpectedField,
    UnexpectedExtraEntries,
    TupleLength {
        expected: usize,
        actual: usize,
    },
    CallbackArity,
    GenericUnproven,
    GenericSpecialization,
    Unproven {
        expected: String,
        actual: String,
    },
}

impl ExternMismatch {
    pub(crate) fn new(reason: ExternMismatchReason) -> Self {
        Self {
            path: Vec::new(),
            reason,
        }
    }

    pub(crate) fn at(mut self, step: ExternPathStep) -> Self {
        self.path.insert(0, step);
        self
    }
}

impl fmt::Display for ExternMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ExternMismatchReason as Reason;
        use ExternPathStep as Step;
        for (index, step) in self.path.iter().enumerate() {
            if index > 0 {
                f.write_str(" → ")?;
            }
            match step {
                Step::Argument { index } => write!(f, "argument {index}")?,
                Step::Result => f.write_str("result")?,
                Step::Field { key } => write!(f, "field {key}")?,
                Step::MapKey => f.write_str("map key")?,
                Step::MapValue => f.write_str("map value")?,
                Step::Element => f.write_str("element")?,
                Step::Item { index } => write!(f, "item {index}")?,
                Step::UnionMember => f.write_str("union member")?,
                Step::CallbackArgument { index } => write!(f, "callback argument {index}")?,
                Step::CallbackRest => f.write_str("callback rest arguments")?,
                Step::CallbackResult => f.write_str("callback result")?,
            }
        }
        if !self.path.is_empty() {
            f.write_str(": ")?;
        }
        match &self.reason {
            Reason::MissingArity => {
                f.write_str("the implementation doesn't accept this argument count")
            }
            Reason::TypeMismatch { expected, actual } => {
                write!(f, "expected {expected}, got {actual}")
            }
            Reason::MissingField => f.write_str("required field is missing"),
            Reason::OptionalField => f.write_str("required field may be absent"),
            Reason::UndeclaredField => f.write_str("open map doesn't declare this field's type"),
            Reason::UnexpectedField => f.write_str("field isn't allowed in the receiving map"),
            Reason::UnexpectedExtraEntries => {
                f.write_str("receiving map doesn't allow those extra entries")
            }
            Reason::TupleLength { expected, actual } => {
                write!(f, "expected {expected} items, got {actual}")
            }
            Reason::CallbackArity => {
                f.write_str("callback doesn't accept the promised argument shape")
            }
            Reason::GenericUnproven => {
                f.write_str("cannot prove compatibility of the generic signatures")
            }
            Reason::GenericSpecialization => {
                f.write_str("generic signature cannot be specialized to the promised types")
            }
            Reason::Unproven { expected, actual } => {
                write!(f, "cannot prove that {actual} fits {expected}")
            }
        }
    }
}
