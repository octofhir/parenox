//! Render findings and linked source locations for the editor.
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ops::Range as ByteRange;

use parenox_analysis::{Diagnostic, Session};
use serde::Serialize;
use serde_json::{Value, json};

use crate::rename::Document;
use crate::text::{Lines, Range};

pub const QUICK_FIX_KIND: &str = "quickfix";

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

/// Build edits from current server diagnostics and the document's lexical scope.
pub fn quick_fix_actions(
    session: &Session,
    path: &str,
    document: &Document<'_>,
    selection: ByteRange<usize>,
    published: &[Value],
    requested: &[Value],
    versioned: bool,
) -> Vec<Value> {
    let lines = Lines::new(document.source);
    let mut seen = BTreeSet::new();
    published
        .iter()
        .filter_map(|finding| {
            let code = finding["code"].as_str()?;
            if finding["source"] != "parenox" || !matches!(code, "PNX2304" | "PNX9001") {
                return None;
            }
            if !requested.is_empty()
                && !requested.iter().any(|received| {
                    ["source", "code", "range", "message"]
                        .into_iter()
                        .all(|field| received[field] == finding[field])
                })
            {
                return None;
            }
            let range = finding.get("range")?;
            let span = lines.byte_range(serde_json::from_value::<Range>(range.clone()).ok()?)?;
            let selected = if selection.is_empty() {
                (span.start..=span.end).contains(&selection.start)
            } else {
                span.start < selection.end && selection.start < span.end
            };
            let occurrence = (span.start, span.end);
            if !selected || seen.contains(&occurrence) {
                return None;
            }
            let (verb, replacement) = if code == "PNX2304" {
                (
                    "Replace with",
                    Cow::Borrowed(session.local_spelling_fix(path, span)?),
                )
            } else {
                (
                    "Qualify as",
                    session.dotted_var_fix(path, span, finding["message"].as_str()?)?,
                )
            };
            seen.insert(occurrence);
            let edit = document.workspace_edit(
                vec![json!({"range":range,"newText":replacement})],
                versioned,
            );
            Some(json!({
                "title": format!("{verb} {replacement}"),
                "kind": QUICK_FIX_KIND,
                "diagnostics": [finding],
                "edit": edit,
            }))
        })
        .collect()
}
