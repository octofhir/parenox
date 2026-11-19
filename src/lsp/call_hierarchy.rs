//! Map source call edges to LSP items and UTF-16 site ranges.
use std::collections::BTreeMap;

use parenox::CallHierarchyItem;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Lines, Range, TextQuery, Workspace};

#[derive(Deserialize)]
pub(in crate::lsp) struct CallHierarchyQuery {
    item: Selection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Selection {
    name: String,
    uri: String,
    selection_range: Range,
}

impl Workspace {
    pub(in crate::lsp) fn prepare_call_hierarchy(&self, query: TextQuery) -> Option<Vec<Value>> {
        let (path, offset) = self.query_offset(query)?;
        let item = self
            .session_for(&path)?
            .prepare_call_hierarchy(&path, offset)?;
        let mut lines = BTreeMap::new();
        Some(vec![self.call_hierarchy_item(&item, &mut lines)?])
    }

    pub(in crate::lsp) fn incoming_calls(&self, query: CallHierarchyQuery) -> Vec<Value> {
        self.call_hierarchy_edges(query.item, true)
            .unwrap_or_default()
    }

    pub(in crate::lsp) fn outgoing_calls(&self, query: CallHierarchyQuery) -> Vec<Value> {
        self.call_hierarchy_edges(query.item, false)
            .unwrap_or_default()
    }

    fn call_hierarchy_edges(&self, item: Selection, incoming: bool) -> Option<Vec<Value>> {
        let path = self.document_key(&item.uri)?;
        let document = self.documents.get(&path)?;
        let lines = Lines::new(&document.source);
        let selection = lines.byte_range(item.selection_range)?;
        let session = self.session_for(&path)?;
        let current = session.prepare_call_hierarchy(&path, selection.start)?;
        if current.path != path
            || current.name != item.name
            || (current.selection.start.offset..current.selection.end.offset) != selection
        {
            return None;
        }
        let groups = if incoming {
            session.incoming_calls(&path, selection.start)
        } else {
            session.outgoing_calls(&path, selection.start)
        };
        let mut lines = BTreeMap::from([(path.as_str(), lines)]);
        Some(
            groups
                .iter()
                .filter_map(|group| {
                    let item = self.call_hierarchy_item(&group.item, &mut lines)?;
                    let site_path = if incoming { &group.item.path } else { &path };
                    let site_lines = lines.get(site_path.as_str())?;
                    let ranges: Vec<_> = group
                        .sites
                        .iter()
                        .filter_map(|span| site_lines.range(*span))
                        .collect();
                    Some(if incoming {
                        json!({"from":item, "fromRanges":ranges})
                    } else {
                        json!({"to":item, "fromRanges":ranges})
                    })
                })
                .collect(),
        )
    }

    fn call_hierarchy_item<'a, 'b>(
        &'a self,
        item: &'b CallHierarchyItem,
        lines: &mut BTreeMap<&'b str, Lines<'a>>,
    ) -> Option<Value> {
        let document = self.documents.get(&item.path)?;
        let lines = lines
            .entry(&item.path)
            .or_insert_with(|| Lines::new(&document.source));
        Some(json!({
            "name": item.name,
            "kind": 12,
            "detail": item.namespace,
            "uri": document.uri(&item.path),
            "range": lines.range(item.range)?,
            "selectionRange": lines.range(item.selection)?,
        }))
    }
}
