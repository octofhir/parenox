//! Descriptions of failed type compatibility obligations.
use std::fmt;

use serde::Serialize;

/// One failed obligation, not a list of all differences between declarations.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ExternMismatch {
    pub path: Vec<ExternPathStep>,
    #[serde(flatten)]
    pub reason: ExternMismatchReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

impl fmt::Display for ExternPathStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Argument { index } => write!(f, "argument {index}"),
            Self::Result => f.write_str("result"),
            Self::Field { key } => write!(f, "field {key}"),
            Self::MapKey => f.write_str("map key"),
            Self::MapValue => f.write_str("map value"),
            Self::Element => f.write_str("element"),
            Self::Item { index } => write!(f, "item {index}"),
            Self::UnionMember => f.write_str("union member"),
            Self::CallbackArgument { index } => write!(f, "callback argument {index}"),
            Self::CallbackRest => f.write_str("callback rest arguments"),
            Self::CallbackResult => f.write_str("callback result"),
        }
    }
}

impl fmt::Display for ExternMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ExternMismatchReason as Reason;
        for (index, step) in self.path.iter().enumerate() {
            if index > 0 {
                f.write_str(" → ")?;
            }
            step.fmt(f)?;
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
