//! Route open buffers to the configured project or a nested project.
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use parenox_lsp::{project_tree, structural::FoldingOptions};
use serde_json::Value;

use super::symbols;
use super::text::{file_path, normalize};
use super::workspace::file_rename::FileRename;
use super::workspace::{DocumentParams, WatchedFiles, Workspace};
use crate::cli::ProjectOptions;
use crate::workspace::is_clojure_source;

pub(super) struct Projects {
    root: PathBuf,
    primary: Workspace,
    nested: BTreeMap<PathBuf, Workspace>,
    owners: BTreeMap<PathBuf, PathBuf>,
    client: ClientFeatures,
}

pub(super) struct ClientFeatures {
    pub apply_edit: bool,
    pub versioned_edits: bool,
    pub snippets: bool,
    pub markdown_hover: bool,
    pub hierarchical_symbols: bool,
    pub folding: FoldingOptions,
    pub test_tree: bool,
}

impl Projects {
    pub fn load(
        root: PathBuf,
        options: &ProjectOptions,
        watch: bool,
        client: ClientFeatures,
    ) -> Self {
        let root = normalize(&root);
        Self {
            primary: Workspace::load(
                root.clone(),
                options,
                watch,
                client.versioned_edits,
                client.test_tree,
            ),
            root,
            nested: BTreeMap::new(),
            owners: BTreeMap::new(),
            client,
        }
    }

    pub fn snippets(&self) -> bool {
        self.client.snippets
    }

    pub fn apply_edit(&self) -> bool {
        self.client.apply_edit
    }

    pub fn markdown_hover(&self) -> bool {
        self.client.markdown_hover
    }

    pub fn hierarchical_symbols(&self) -> bool {
        self.client.hierarchical_symbols
    }

    pub fn folding(&self) -> &FoldingOptions {
        &self.client.folding
    }

    pub fn workspace_symbols(
        &mut self,
        query: &str,
        output: &mut impl Write,
    ) -> io::Result<Vec<Value>> {
        for workspace in std::iter::once(&mut self.primary).chain(self.nested.values_mut()) {
            workspace.ensure_index(output)?;
        }
        let candidates = std::iter::once(&self.primary)
            .chain(self.nested.values())
            .flat_map(Workspace::symbol_candidates);
        Ok(symbols::search(query, candidates))
    }

    pub fn initialized(&mut self, output: &mut impl Write) -> io::Result<()> {
        self.primary.initialized(output)
    }

    pub fn project_tree(
        &mut self,
        query: Option<project_tree::Leaf>,
        output: &mut impl Write,
    ) -> io::Result<Option<Value>> {
        if query.as_ref().is_some_and(project_tree::Leaf::needs_index) {
            self.primary.ensure_index(output)?;
        }
        Ok(self.primary.project_tree(query))
    }

    pub fn dependency_contents(&self, uri: &str) -> Option<String> {
        std::iter::once(&self.primary)
            .chain(self.nested.values())
            .find_map(|workspace| workspace.dependency_contents(uri))
    }

    pub fn will_rename_files(
        &mut self,
        files: &[FileRename],
        output: &mut impl Write,
    ) -> io::Result<Result<Value, String>> {
        let mut project = None;
        for file in files {
            let (Some(old), Some(new)) = (file_path(&file.old_uri), file_path(&file.new_uri))
            else {
                continue;
            };
            if !is_clojure_source(&old) || old == new {
                continue;
            }
            let owner = self.source_owner(&old);
            if owner != self.source_owner(&new) || project.is_some_and(|root| root != owner) {
                return Ok(Err("File rename edits cannot span multiple projects".into()));
            }
            project = Some(owner);
        }
        let Some(project) = project else {
            return Ok(Ok(Value::Null));
        };
        let workspace = match project.cloned() {
            Some(root) => self.nested.get_mut(&root).unwrap(),
            None => &mut self.primary,
        };
        workspace.ensure_index(output)?;
        Ok(workspace.will_rename_files(files))
    }

    pub fn did_rename_files(
        &mut self,
        files: &[FileRename],
        output: &mut impl Write,
    ) -> io::Result<()> {
        for workspace in std::iter::once(&mut self.primary).chain(self.nested.values_mut()) {
            workspace.did_rename_files(files, output)?;
        }
        for file in files {
            if let (Some(old), Some(new)) = (file_path(&file.old_uri), file_path(&file.new_uri))
                && !old.exists()
                && let Some(root) = self.owners.remove(&old)
            {
                if new.is_file() && self.nested[&root].contains_project_source(&new) {
                    self.owners.entry(new).or_insert(root);
                } else if !self.owners.values().any(|owner| *owner == root) {
                    self.nested.remove(&root);
                }
            }
        }
        Ok(())
    }

    fn source_owner(&self, path: &Path) -> Option<&PathBuf> {
        self.owners.get(path).or_else(|| {
            self.nested
                .iter()
                .filter(|(_, workspace)| workspace.contains_project_source(path))
                .max_by_key(|(root, _)| root.components().count())
                .map(|(root, _)| root)
        })
    }

    pub fn for_document(&self, uri: Option<&str>) -> &Workspace {
        self.owner(uri)
            .map_or(&self.primary, |root| &self.nested[root])
    }

    pub fn for_document_mut(&mut self, uri: Option<&str>) -> &mut Workspace {
        match self.owner(uri).cloned() {
            Some(root) => self.nested.get_mut(&root).unwrap(),
            None => &mut self.primary,
        }
    }

    pub fn track_open(&mut self, uri: Option<&str>, output: &mut impl Write) -> io::Result<()> {
        let Some(path) = uri.and_then(file_path) else {
            return Ok(());
        };
        if !path.starts_with(&self.root)
            || self.owners.contains_key(&path)
            || self.primary.contains_project_source(&path)
        {
            return Ok(());
        }
        let options = ProjectOptions::default();
        // Only look within the IDE workspace. A neighboring checkout never
        // receives the primary project's CLI flags or contributes its specs.
        let Some(root) = path
            .ancestors()
            .skip(1)
            .take_while(|dir| *dir != self.root)
            .find(|dir| options.manifests(dir).iter().any(|path| path.is_file()))
            .map(PathBuf::from)
        else {
            return Ok(());
        };
        if !self.nested.contains_key(&root) {
            let mut project = Workspace::load(
                root.clone(),
                &options,
                false,
                self.client.versioned_edits,
                self.client.test_tree,
            );
            project.initialized(output)?;
            self.nested.insert(root.clone(), project);
        }
        self.owners.insert(path, root);
        Ok(())
    }

    pub fn track_close(&mut self, uri: Option<&str>) {
        if let Some(root) = uri
            .and_then(file_path)
            .and_then(|path| self.owners.remove(&path))
            && !self.owners.values().any(|owner| *owner == root)
        {
            self.nested.remove(&root);
        }
    }

    pub fn save(&mut self, params: DocumentParams, output: &mut impl Write) -> io::Result<()> {
        // A project input can be saved without a corresponding open buffer,
        // and two projects may intentionally share an input on disk.
        for workspace in std::iter::once(&mut self.primary).chain(self.nested.values_mut()) {
            workspace.save(&params, output)?;
        }
        Ok(())
    }

    pub fn changed_files(
        &mut self,
        params: WatchedFiles,
        output: &mut impl Write,
    ) -> io::Result<()> {
        for workspace in std::iter::once(&mut self.primary).chain(self.nested.values_mut()) {
            workspace.changed_files(&params, output)?;
        }
        Ok(())
    }

    fn owner(&self, uri: Option<&str>) -> Option<&PathBuf> {
        self.owners.get(&file_path(uri?)?)
    }
}
