//! Project snapshots for displaying locations from saved externs.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::text::normalize;

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
