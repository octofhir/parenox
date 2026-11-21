//! Display inferred binding types at the end of their source identifiers.
use parenox_analysis::BindingType;
use serde::Serialize;

use crate::text::{Lines, Position};

const MAX_TYPE_LENGTH: usize = 80;

#[derive(Serialize)]
pub struct InlayHint {
    position: Position,
    label: String,
    kind: u8,
}

pub fn bindings(types: Vec<BindingType>, lines: &Lines<'_>) -> Vec<InlayHint> {
    types
        .into_iter()
        .filter_map(|binding| {
            if binding
                .inferred_type
                .chars()
                .take(MAX_TYPE_LENGTH + 1)
                .count()
                > MAX_TYPE_LENGTH
            {
                return None;
            }
            Some(InlayHint {
                position: lines.position(binding.span.end.offset)?,
                label: format!(": {}", binding.inferred_type),
                kind: 1,
            })
        })
        .collect()
}
