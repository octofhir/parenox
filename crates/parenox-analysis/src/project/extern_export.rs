//! Write and verify standalone extern snapshots without using the session's extern cache.
use std::fmt::Write;
use std::time::Instant;

use super::{
    ExternExport, FileCache, FileKind, ProjectReport, Session, Source, identify_diagnostics,
};
use crate::Diagnostic;
use crate::checker::{EncodeError, Globals, Model, SnapshotWriter};
use crate::reader::Span;
use crate::types::Signature;

const SNAPSHOT_PATH: &str = "<exported externs>";

impl Session {
    /// Checks a library and exports its public function signatures and literal values.
    /// Returns EDN only if it reconstructs those declarations in isolation.
    /// Dependency externs and guard refinements are not exported.
    pub fn export_externs(
        &mut self,
        sources: &[Source<'_>],
        dependencies: &[Source<'_>],
    ) -> ExternExport {
        let started = Instant::now();
        let mut report = self.check_project_with_externs(sources, dependencies);
        let mut edn = None;
        if report.ok && report.complete {
            let mut entries = Vec::new();
            for (index, source) in sources.iter().enumerate() {
                let cache = &self.files[source.path];
                if cache.reader_table {
                    continue;
                }
                let model = cache
                    .model
                    .as_ref()
                    .expect("a successfully checked source has a model");
                let signatures = model.exported_signatures().map(|(name, span, signature)| {
                    (name, span, signature.map(Exported::Signature))
                });
                let literals = cache
                    .public_literals()
                    .map(|(name, span, value)| (name, span, value.map(Exported::Literal)));
                for (name, span, item) in signatures.chain(literals) {
                    match item {
                        Ok(item) => entries.push(Entry {
                            name,
                            span,
                            index,
                            model,
                            item,
                        }),
                        Err(reason) => reject(&mut report, index, name, span, reason),
                    }
                }
            }
            if report.ok {
                entries.sort_unstable_by_key(|entry| entry.name);
                match encode_and_verify(&entries) {
                    Ok(snapshot) => edn = Some(snapshot),
                    Err((entry, reason)) => {
                        reject(&mut report, entry.index, entry.name, entry.span, &reason);
                    }
                }
            }
            for file in &mut report.files {
                if !file.diagnostics.is_empty() {
                    identify_diagnostics(&file.path, 0, &mut file.diagnostics);
                }
            }
        }
        report.stats.elapsed_us = started.elapsed().as_micros() as u64;
        ExternExport { report, edn }
    }
}

struct Entry<'a> {
    name: &'a str,
    span: Span,
    index: usize,
    model: &'a Model,
    item: Exported<'a>,
}

enum Exported<'a> {
    Signature(&'a [Signature]),
    Literal(String),
}

impl Entry<'_> {
    fn encode(&self, edn: &mut SnapshotWriter, separator: bool) -> Result<usize, EncodeError> {
        if separator {
            edn.write_str("\n ")?;
        }
        let offset = edn.len();
        write!(edn, "{} ", self.name)?;
        match &self.item {
            Exported::Signature(signatures) => edn.signatures(signatures)?,
            Exported::Literal(value) => write!(edn, "{{:value {value}}}")?,
        }
        Ok(offset)
    }
}

fn encode_and_verify<'a>(entries: &'a [Entry<'a>]) -> Result<String, (&'a Entry<'a>, String)> {
    let Some(last) = entries.last() else {
        return Ok("{}\n".into());
    };
    let mut edn = SnapshotWriter::new();
    let mut offsets = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let offset = entry
            .encode(&mut edn, index != 0)
            .map_err(|reason| (entry, reason.to_string()))?;
        offsets.push(offset);
    }
    let edn = edn.finish().map_err(|reason| (last, reason.to_string()))?;

    let mut snapshot = FileCache::default();
    snapshot.update(&edn, SNAPSHOT_PATH, FileKind::Externs);
    let locate = |diagnostic: &Diagnostic| {
        let position = diagnostic.span.start.offset;
        let index = offsets
            .partition_point(|&start| start <= position)
            .saturating_sub(1);
        (
            &entries[index],
            format!("generated EDN is invalid: {}", diagnostic.message),
        )
    };
    if let Some(diagnostic) = &snapshot.parse_error {
        return Err(locate(diagnostic));
    }

    // In particular, don't allow the source spec table to satisfy a name left
    // unresolved by the encoder. Reuse the actual extern parser and type reader.
    let parsed = Model::externs(&snapshot.forms, SNAPSHOT_PATH, &edn, &Globals::default());
    if let Some(diagnostic) = parsed.diagnostics.first() {
        return Err(locate(diagnostic));
    }
    for entry in entries {
        let literal = match &entry.item {
            Exported::Signature(_) => None,
            Exported::Literal(value) => Some(value.as_str()),
        };
        if !entry.model.matches_export(&parsed, entry.name, literal) {
            return Err((
                entry,
                "generated EDN did not preserve the declaration".into(),
            ));
        }
    }
    if parsed.extern_count() != entries.len() {
        return Err((
            &entries[0],
            "generated EDN changed the number of declarations".into(),
        ));
    }
    Ok(edn)
}

fn reject(report: &mut ProjectReport, index: usize, name: &str, span: Span, reason: &str) {
    let file = &mut report.files[index];
    file.diagnostics.push(Diagnostic::new(
        "PNX9012",
        span,
        format!("Cannot export {name}: {reason}"),
    ));
    file.ok = false;
    file.complete = false;
    report.ok = false;
    report.complete = false;
}
