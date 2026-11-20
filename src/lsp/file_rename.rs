//! Namespace edits around client-owned filesystem moves.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use parenox_lsp::text::{Lines, file_path};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Workspace, publish, visible_under};
use crate::workspace::is_clojure_source;

#[derive(Deserialize)]
pub(in crate::lsp) struct RenameFiles {
    pub files: Vec<FileRename>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::lsp) struct FileRename {
    pub old_uri: String,
    pub new_uri: String,
}

impl Workspace {
    pub(in crate::lsp) fn will_rename_files(&self, files: &[FileRename]) -> Result<Value, String> {
        let mut destinations = BTreeSet::new();
        let mut by_file: BTreeMap<String, Vec<_>> = BTreeMap::new();
        for file in files {
            let (Some(old), Some(new)) = (file_path(&file.old_uri), file_path(&file.new_uri))
            else {
                continue;
            };
            if !is_clojure_source(&old) || old == new {
                continue;
            }
            let old_path = self
                .document_key(&file.old_uri)
                .ok_or("The source file is unavailable")?;
            if !self.documents.contains_key(&old_path) {
                continue;
            }
            let new = self.logical_source_path(&new);
            if !is_clojure_source(&new) || old.extension() != new.extension() {
                return Err("Namespace moves require the same Clojure source extension".into());
            }
            if fs::symlink_metadata(&new).is_ok() {
                return Err(format!("The destination already exists: {}", new.display()));
            }
            let old_namespace = namespace_for_path(Path::new(&old_path), &self.editable_roots)?;
            let new_namespace = namespace_for_path(&new, &self.editable_roots)?;
            if !destinations.insert(new_namespace.clone()) {
                return Err(format!(
                    "The destination namespace {new_namespace} is repeated"
                ));
            }
            let edits = self
                .session
                .namespace_move(&old_path, &old_namespace, &new_namespace)?;
            for edit in edits {
                by_file
                    .entry(edit.path)
                    .or_default()
                    .push((edit.span, new_namespace.clone()));
            }
        }
        let mut changes = BTreeMap::new();
        let mut document_changes = Vec::new();
        for (path, mut replacements) in by_file {
            let document = self
                .rename_document(&path)
                .ok_or_else(|| format!("Namespace move needs an editable source file: {path}"))?;
            let lines = Lines::new(document.source);
            replacements.sort_by_key(|(span, _)| span.start.offset);
            if replacements
                .windows(2)
                .any(|pair| pair[0].0.end.offset > pair[1].0.start.offset)
            {
                return Err("Namespace move edits overlap".into());
            }
            let edits = replacements
                .into_iter()
                .map(|(span, text)| {
                    let range = lines.range(span).ok_or("A namespace move span is stale")?;
                    Ok(json!({"range":range,"newText":text}))
                })
                .collect::<Result<Vec<_>, String>>()?;
            if self.versioned_edits {
                document_changes.push(json!({
                    "textDocument":{"uri":document.uri,"version":document.version},"edits":edits
                }));
            } else {
                changes.insert(document.uri, edits);
            }
        }
        Ok(if self.versioned_edits {
            json!({"documentChanges":document_changes})
        } else {
            json!({"changes":changes})
        })
    }

    pub(in crate::lsp) fn did_rename_files(
        &mut self,
        files: &[FileRename],
        output: &mut impl Write,
    ) -> io::Result<()> {
        let mut refresh = false;
        for file in files {
            let (Some(old), Some(new)) = (file_path(&file.old_uri), file_path(&file.new_uri))
            else {
                continue;
            };
            if old == new || old.exists() {
                continue;
            }
            let old_key = self.document_key(&file.old_uri);
            let new_is_indexed = is_clojure_source(&new) && self.contains_project_source(&new);
            if new_is_indexed || (is_clojure_source(&old) && self.contains_project_source(&old)) {
                refresh = true;
            }
            let Some(old_key) = old_key else { continue };
            let Some(mut document) = self.documents.remove(&old_key) else {
                continue;
            };
            self.session.forget_file(&old_key);
            refresh |= document.indexed;
            let Some(editor) = &mut document.editor else {
                continue;
            };
            publish(output, json!({"uri":editor.uri,"diagnostics":[]}))?;
            let new = self.logical_source_path(&new);
            if new_is_indexed
                && new.is_file()
                && let Some(new_key) = new.to_str()
                && self
                    .documents
                    .get(new_key)
                    .is_none_or(|new| new.editor.is_none())
            {
                editor.uri.clone_from(&file.new_uri);
                editor.published = None;
                editor.test_tree = None;
                self.documents.insert(new_key.into(), document);
            }
        }
        if refresh {
            self.refresh(output)?;
        }
        Ok(())
    }

    fn logical_source_path(&self, path: &Path) -> PathBuf {
        self.source_aliases
            .iter()
            .find_map(|(physical, logical)| {
                path.strip_prefix(physical)
                    .ok()
                    .map(|tail| logical.join(tail))
            })
            .unwrap_or_else(|| path.to_owned())
    }
}

fn namespace_for_path(path: &Path, roots: &[PathBuf]) -> Result<String, String> {
    let mut namespaces = BTreeSet::new();
    for root in roots {
        if !visible_under(path, root) {
            continue;
        }
        // The destination can be absent, but its nearest existing ancestor
        // must stay under the physical source root.
        let physical_root = root
            .canonicalize()
            .map_err(|_| "The configured source root does not exist")?;
        let ancestor = path
            .ancestors()
            .find(|p| p.exists())
            .and_then(|p| p.canonicalize().ok());
        if !ancestor.is_some_and(|ancestor| ancestor.starts_with(physical_root)) {
            return Err("Namespace moves cannot escape the source root through a symlink".into());
        }
        let tail = path.strip_prefix(root).unwrap().with_extension("");
        let mut parts = Vec::new();
        for component in tail.components() {
            let part = component
                .as_os_str()
                .to_str()
                .ok_or("The namespace path is not UTF-8")?;
            if part.is_empty()
                || !part.starts_with(|ch: char| ch.is_alphabetic() || ch == '_')
                || !part.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
            {
                return Err(
                    "The file name does not follow Clojure namespace path conventions".into(),
                );
            }
            parts.push(part.replace('_', "-"));
        }
        if !parts.is_empty() {
            namespaces.insert(parts.join("."));
        }
    }
    if namespaces.len() != 1 {
        return Err(
            "The file must map to one namespace under the configured editable source roots".into(),
        );
    }
    Ok(namespaces.pop_first().unwrap())
}
