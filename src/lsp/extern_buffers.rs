//! Project snapshots for displaying locations from saved externs.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use parenox::{
    ContractOrigin, Diagnostic, Session,
    output::{callback_contract_note, contract_note},
};
use parenox_lsp::diagnostics::RelatedInformation;

use super::text::{Lines, file_uri, normalize};
use crate::workspace::FileSnapshot;

#[derive(Clone, Default)]
pub(super) struct ExternBuffers(Rc<RefCell<BTreeMap<PathBuf, OpenEdn>>>);

struct OpenEdn {
    physical: PathBuf,
    source: String,
}

impl ExternBuffers {
    pub fn update(&self, path: PathBuf, source: Option<String>) {
        let mut open = self.0.borrow_mut();
        if let Some(source) = source {
            let physical = physical_path(&path);
            open.insert(path, OpenEdn { physical, source });
        } else {
            open.remove(&path);
        }
    }

    pub fn is_dirty(&self, path: &Path, saved: &str) -> bool {
        let open = self.0.borrow();
        if open.is_empty() {
            return false;
        }
        let physical = physical_path(path);
        open.iter().any(|(opened, buffer)| {
            (opened == path || buffer.physical == physical) && buffer.source != saved
        })
    }

    pub fn previous_physical(&self, path: &Path) -> Option<PathBuf> {
        self.0
            .borrow()
            .get(path)
            .map(|buffer| buffer.physical.clone())
    }
}

pub(super) struct ExternLocations<'a> {
    session: &'a Session,
    snapshots: &'a [FileSnapshot],
    buffers: Option<&'a ExternBuffers>,
    lines: BTreeMap<&'a str, Option<Lines<'a>>>,
}

impl<'a> ExternLocations<'a> {
    pub fn new(
        session: &'a Session,
        snapshots: &'a [FileSnapshot],
        buffers: Option<&'a ExternBuffers>,
    ) -> Self {
        Self {
            session,
            snapshots,
            buffers,
            lines: BTreeMap::new(),
        }
    }

    pub fn related(&mut self, finding: &Diagnostic) -> Vec<RelatedInformation> {
        let contract = finding.contract.as_ref();
        let callback = finding
            .callback_contract
            .as_ref()
            .filter(|&origin| Some(origin) != contract);
        let direct = contract.and_then(|origin| self.location(origin, contract_note));
        let context = callback.and_then(|origin| self.location(origin, callback_contract_note));
        direct.into_iter().chain(context).collect()
    }

    fn location(
        &mut self,
        origin: &ContractOrigin,
        message: fn(&ContractOrigin) -> String,
    ) -> Option<RelatedInformation> {
        let buffers = self.buffers?;
        let span = self.session.extern_declaration_span(origin)?;
        let path = origin.path.as_deref()?;
        let snapshot = self
            .snapshots
            .iter()
            .find(|snapshot| snapshot.path == path)?;
        let lines = self
            .lines
            .entry(&snapshot.path)
            .or_insert_with(|| {
                (!buffers.is_dirty(Path::new(path), &snapshot.source))
                    .then(|| Lines::new(&snapshot.source))
            })
            .as_ref()?;
        Some(RelatedInformation::new(
            file_uri(path),
            lines.range(span)?,
            message(origin),
        ))
    }
}

pub(super) fn physical_path(path: &Path) -> PathBuf {
    path.canonicalize()
        .or_else(|error| -> std::io::Result<PathBuf> {
            let parent = path.parent().ok_or(error)?;
            let mut directory = parent.canonicalize()?;
            directory.push(path.file_name().unwrap_or_default());
            Ok(directory)
        })
        .unwrap_or_else(|_| normalize(path))
}
