//! Group ordinary source calls by their enclosing function or target.
use std::collections::{BTreeMap, btree_map::Entry};

use crate::checker::{Declarations, ScopedReference};
use crate::project::{SourceSymbol, SourceSymbolKind};
use crate::reader::Span;

use super::{ResolvedVar, Session};

#[derive(Debug)]
pub struct CallHierarchyItem {
    pub path: String,
    pub namespace: String,
    pub name: String,
    pub range: Span,
    pub selection: Span,
}

#[derive(Debug)]
pub struct CallHierarchyCalls {
    pub item: CallHierarchyItem,
    pub sites: Vec<Span>,
}

impl Session {
    /// Resolve a source function declaration or reference at a UTF-8 byte offset.
    pub fn prepare_call_hierarchy(&self, path: &str, offset: usize) -> Option<CallHierarchyItem> {
        if let Some(item) = self.call_hierarchy_definition(path, offset) {
            return Some(item);
        }
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, _) = file.navigation_source()?;
        let references = self.collect_file_references(
            path,
            file,
            self.has_standard_test(),
            false,
            &mut BTreeMap::new(),
        );
        let &(span, reference) = references
            .uses
            .iter()
            .find(|(span, _)| contains(*span, offset))?;
        hierarchy_item(self.resolve_call_reference(path, declarations, span, reference)?)
    }

    /// Find calls to the declaration at this offset. Each site's span is in its caller.
    pub fn incoming_calls(&self, path: &str, offset: usize) -> Vec<CallHierarchyCalls> {
        let Some(target) = self.call_hierarchy_definition(path, offset) else {
            return Vec::new();
        };
        let Some((provider, provider_forms)) = self
            .current_file(path)
            .and_then(|file| file.navigation_source())
        else {
            return Vec::new();
        };
        let public = provider
            .source_var(provider_forms, &target.name, true)
            .is_some();
        let standard_test = self.has_standard_test();
        let mut providers = BTreeMap::new();
        let mut groups = BTreeMap::<_, CallHierarchyCalls>::new();
        for (caller_path, file) in &self.files {
            if file.snapshot != self.snapshot
                || (!public && caller_path != path)
                || !file.source.contains(&target.name)
            {
                continue;
            }
            let Some((caller, forms)) = file.navigation_source() else {
                continue;
            };
            let references = self.collect_file_references(
                caller_path,
                file,
                standard_test,
                true,
                &mut providers,
            );
            let symbols = caller.outline(forms);
            for (span, reference) in references.calls.into_iter().flatten() {
                let matches = match reference {
                    ScopedReference::Referred(namespace, name) => {
                        namespace == target.namespace && name == target.name
                    }
                    ScopedReference::InNamespace(name) => {
                        caller_path == path
                            && name == target.name
                            && provider
                                .source_var_before(provider_forms, name, span.start.offset)
                                .is_some()
                    }
                    ScopedReference::Local(_) => false,
                };
                if matches
                    && let Some(symbol) = symbols.iter().find(|symbol| {
                        symbol.kind == SourceSymbolKind::Function
                            && contains(symbol.range, span.start.offset)
                    })
                {
                    groups
                        .entry((caller_path.as_str(), symbol.selection.start.offset))
                        .or_insert_with(|| CallHierarchyCalls {
                            item: CallHierarchyItem::new(caller_path, caller.namespace(), *symbol),
                            sites: Vec::new(),
                        })
                        .sites
                        .push(span);
                }
            }
        }
        finish_groups(groups)
    }

    /// Find calls from the declaration at this offset. All sites are in `path`.
    pub fn outgoing_calls(&self, path: &str, offset: usize) -> Vec<CallHierarchyCalls> {
        let Some(caller) = self.call_hierarchy_definition(path, offset) else {
            return Vec::new();
        };
        let Some(file) = self.current_file(path) else {
            return Vec::new();
        };
        let Some((declarations, _)) = file.navigation_source() else {
            return Vec::new();
        };
        let references = self.collect_file_references(
            path,
            file,
            self.has_standard_test(),
            true,
            &mut BTreeMap::new(),
        );
        let mut groups = BTreeMap::<_, CallHierarchyCalls>::new();
        for (span, reference) in references.calls.into_iter().flatten() {
            if !contains(caller.range, span.start.offset) {
                continue;
            }
            let Some(target) = self.resolve_call_reference(path, declarations, span, reference)
            else {
                continue;
            };
            let key = (target.path, target.span.start.offset);
            match groups.entry(key) {
                Entry::Occupied(mut entry) => entry.get_mut().sites.push(span),
                Entry::Vacant(entry) => {
                    if let Some(item) = hierarchy_item(target) {
                        entry.insert(CallHierarchyCalls {
                            item,
                            sites: vec![span],
                        });
                    }
                }
            }
        }
        finish_groups(groups)
    }

    fn call_hierarchy_definition(&self, path: &str, offset: usize) -> Option<CallHierarchyItem> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        let symbol = declarations.outline(forms).into_iter().find(|symbol| {
            symbol.kind == SourceSymbolKind::Function && contains(symbol.selection, offset)
        })?;
        let target = self.resolve_source_var(path, declarations.namespace(), symbol.name, None)?;
        (target.path == path && target.span == symbol.selection)
            .then(|| CallHierarchyItem::new(path, target.namespace, symbol))
    }

    fn resolve_call_reference<'a>(
        &'a self,
        path: &str,
        declarations: &'a Declarations,
        span: Span,
        reference: ScopedReference<'a>,
    ) -> Option<ResolvedVar<'a>> {
        match reference {
            ScopedReference::Referred(namespace, name) => {
                self.resolve_source_var(path, namespace, name, None)
            }
            ScopedReference::InNamespace(name) => self.resolve_source_var(
                path,
                declarations.namespace(),
                name,
                Some(span.start.offset),
            ),
            ScopedReference::Local(_) => None,
        }
    }
}

impl CallHierarchyItem {
    fn new(path: &str, namespace: &str, symbol: SourceSymbol<'_>) -> Self {
        Self {
            path: path.to_owned(),
            namespace: namespace.to_owned(),
            name: symbol.name.to_owned(),
            range: symbol.range,
            selection: symbol.selection,
        }
    }
}

fn hierarchy_item(target: ResolvedVar<'_>) -> Option<CallHierarchyItem> {
    let (declarations, forms) = target.source.navigation_source()?;
    let symbol = declarations.outline(forms).into_iter().find(|symbol| {
        symbol.kind == SourceSymbolKind::Function && symbol.selection == target.span
    })?;
    Some(CallHierarchyItem::new(
        target.path,
        target.namespace,
        symbol,
    ))
}

fn finish_groups(groups: BTreeMap<(&str, usize), CallHierarchyCalls>) -> Vec<CallHierarchyCalls> {
    groups
        .into_values()
        .map(|mut group| {
            group.sites.sort_unstable_by_key(|span| span.start.offset);
            group.sites.dedup();
            group
        })
        .collect()
}

fn contains(span: Span, offset: usize) -> bool {
    (span.start.offset..span.end.offset).contains(&offset)
}
