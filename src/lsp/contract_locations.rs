//! Resolve diagnostic links from the snapshots used by the checker.
use std::{collections::BTreeMap, path::Path};

use parenox::{
    ContractKind, ContractOrigin, Diagnostic, Session,
    output::{callback_contract_note, contract_note},
};
use parenox_lsp::diagnostics::RelatedInformation;

use super::extern_buffers::ExternBuffers;
use super::text::{Lines, file_uri};
use crate::workspace::FileSnapshot;

pub(super) struct ContractLocations<'a> {
    session: &'a Session,
    externs: &'a [FileSnapshot],
    buffers: Option<&'a ExternBuffers>,
    lines: BTreeMap<&'a str, Option<Lines<'a>>>,
}

impl<'a> ContractLocations<'a> {
    pub fn new(
        session: &'a Session,
        externs: &'a [FileSnapshot],
        buffers: Option<&'a ExternBuffers>,
    ) -> Self {
        Self {
            session,
            externs,
            buffers,
            lines: BTreeMap::new(),
        }
    }

    pub fn related(
        &mut self,
        finding: &Diagnostic,
        source_uri: impl Fn(&str) -> String,
    ) -> Vec<RelatedInformation> {
        if self.buffers.is_none() {
            return Vec::new();
        }
        let contract = finding.contract.as_ref();
        let callback = finding
            .callback_contract
            .as_ref()
            .filter(|&origin| Some(origin) != contract);
        let direct = contract.and_then(|origin| self.location(origin, contract_note, &source_uri));
        let context =
            callback.and_then(|origin| self.location(origin, callback_contract_note, &source_uri));
        direct.into_iter().chain(context).collect()
    }

    fn location(
        &mut self,
        origin: &ContractOrigin,
        message: fn(&ContractOrigin) -> String,
        source_uri: &impl Fn(&str) -> String,
    ) -> Option<RelatedInformation> {
        let buffers = self.buffers?;
        let (source, span, uri) = match origin.kind {
            ContractKind::TSig | ContractKind::SFdef => {
                let (source, span) = self.session.source_contract_declaration(origin)?;
                (source, span, source_uri(source.path))
            }
            ContractKind::Extern => {
                let span = self.session.extern_declaration_span(origin)?;
                let path = origin.path.as_deref()?;
                let snapshot = self.externs.iter().find(|snapshot| snapshot.path == path)?;
                (snapshot.as_source(), span, file_uri(path))
            }
            _ => return None,
        };
        let lines = self
            .lines
            .entry(source.path)
            .or_insert_with(|| {
                let visible = origin.kind != ContractKind::Extern
                    || !buffers.is_dirty(Path::new(source.path), source.source);
                visible.then(|| Lines::new(source.source))
            })
            .as_ref()?;
        Some(RelatedInformation::new(
            uri,
            lines.range(span)?,
            message(origin),
        ))
    }
}
