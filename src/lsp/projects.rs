//! Route open buffers to the configured project or a nested project.
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;

use serde_json::Value;

use super::symbols;
use super::text::{file_path, normalize};
use super::workspace::{DocumentParams, WatchedFiles, Workspace};
use crate::cli::ProjectOptions;

pub(super) struct Projects {
    root: PathBuf,
    primary: Workspace,
    nested: BTreeMap<PathBuf, Workspace>,
    owners: BTreeMap<PathBuf, PathBuf>,
    versioned_edits: bool,
    snippets: bool,
    hierarchical_symbols: bool,
}

impl Projects {
    pub fn load(
        root: PathBuf,
        options: &ProjectOptions,
        watch: bool,
        versioned_edits: bool,
        snippets: bool,
        hierarchical_symbols: bool,
    ) -> Self {
        let root = normalize(&root);
        Self {
            primary: Workspace::load(root.clone(), options, watch, versioned_edits),
            root,
            nested: BTreeMap::new(),
            owners: BTreeMap::new(),
            versioned_edits,
            snippets,
            hierarchical_symbols,
        }
    }

    pub fn snippets(&self) -> bool {
        self.snippets
    }

    pub fn hierarchical_symbols(&self) -> bool {
        self.hierarchical_symbols
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
            let mut project = Workspace::load(root.clone(), &options, false, self.versioned_edits);
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
