//! Render findings and linked source locations for the editor.
use parenox_analysis::Diagnostic;
use serde::Serialize;
use serde_json::{Value, json};

use crate::text::Range;

#[derive(Serialize)]
pub struct RelatedInformation {
    location: Location,
    message: String,
}

#[derive(Serialize)]
struct Location {
    uri: String,
    range: Range,
}

impl RelatedInformation {
    pub fn new(uri: String, range: Range, message: String) -> Self {
        Self {
            location: Location { uri, range },
            message,
        }
    }
}

pub fn render(
    range: Range,
    diagnostic: &Diagnostic,
    message: &str,
    related: Vec<RelatedInformation>,
) -> Value {
    let severity = if diagnostic.severity == "warning" {
        2
    } else {
        1
    };
    let mut finding = json!({
        "range": range,
        "severity": severity,
        "source": "parenox",
        "code": diagnostic.code,
        "message": message,
    });
    if !related.is_empty() {
        finding["relatedInformation"] = json!(related);
    }
    finding
}
