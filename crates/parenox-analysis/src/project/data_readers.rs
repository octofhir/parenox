//! Validate classpath reader tables without invoking their handlers.
use std::collections::BTreeMap;
use std::path::Path;

use crate::Diagnostic;
use crate::reader::{CheckRead, Form, Kind, OmittedDispatch, Position, ReadError, Span};

pub(super) fn is_reader_table(path: &str, forms: &[Form]) -> bool {
    matches!(
        Path::new(path).file_name().and_then(|name| name.to_str()),
        Some("data_readers.clj" | "data_readers.cljc")
    ) && !forms.iter().any(|form| {
        matches!(&form.without_metadata().kind, Kind::List(items)
            if matches!(items.first().and_then(Form::symbol),
                Some("ns" | "clojure.core/ns" | "in-ns" | "clojure.core/in-ns")))
    })
}

pub(super) fn validate(read: &CheckRead) -> Result<(), ReadError> {
    if let Some(dispatch) = read
        .unchecked_dispatches
        .as_ref()
        .and_then(|d| d.first_omitted)
    {
        let span = match dispatch {
            OmittedDispatch::Tag(span)
            | OmittedDispatch::Regex(span)
            | OmittedDispatch::MetadataTag(span)
            | OmittedDispatch::MetadataRegex(span) => span,
        };
        return Err(ReadError::invalid(
            span,
            "Reader table entries must be literal symbols",
        ));
    }
    let [form] = read.forms.as_slice() else {
        let span = read.forms.first().map_or_else(
            || {
                let start = Position {
                    line: 1,
                    column: 1,
                    offset: 0,
                };
                Span { start, end: start }
            },
            |form| form.span,
        );
        return Err(ReadError::invalid(
            span,
            "Expected one literal data reader map",
        ));
    };
    let Some(entries) = form.map() else {
        return Err(ReadError::invalid(
            form.span,
            "Expected a literal data reader map",
        ));
    };
    for [tag, handler] in entries.as_chunks::<2>().0 {
        if tag.symbol().is_none() {
            return Err(ReadError::invalid(
                tag.span,
                "Data reader tag must be a symbol",
            ));
        }
        if !handler
            .symbol()
            .and_then(|name| name.split_once('/'))
            .is_some_and(|(namespace, local)| !namespace.is_empty() && !local.is_empty())
        {
            return Err(ReadError::invalid(
                handler.span,
                "Data reader handler must be a qualified Var symbol",
            ));
        }
    }
    Ok(())
}

pub(super) fn check_conflicts<'a>(
    tables: impl Iterator<Item = (&'a str, &'a [Form], &'a mut Vec<Diagnostic>)>,
) {
    let mut mappings = BTreeMap::new();
    for (path, forms, diagnostics) in tables {
        let entries = forms[0].map().expect("validated reader map");
        for [tag, handler] in entries.as_chunks::<2>().0 {
            let name = tag.symbol().expect("validated reader tag");
            let handler = handler.symbol().expect("validated reader handler");
            let (earlier_path, previous) = *mappings.entry(name).or_insert((path, handler));
            if previous != handler {
                let mut diagnostic = Diagnostic::new(
                    "PNX2790",
                    tag.span,
                    format!("Conflicting data reader mapping for {name}"),
                );
                diagnostic
                    .notes
                    .push(format!("{earlier_path} maps {name} to {previous}."));
                diagnostics.push(diagnostic);
            }
        }
    }
}
