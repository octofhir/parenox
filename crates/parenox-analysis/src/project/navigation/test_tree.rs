//! Static test outlines using the same source resolution as references.
use std::collections::{BTreeMap, BTreeSet};

use crate::checker::ScopedReference;
use crate::project::SourceSymbolKind;
use crate::reader::{Form, Kind, Span};

use super::Session;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SourceTestKind {
    Namespace,
    Test,
    Testing,
}

pub struct SourceTestNode<'a> {
    pub name: &'a str,
    pub kind: SourceTestKind,
    pub range: Span,
    pub name_range: Span,
    pub children: Vec<SourceTestNode<'a>>,
}

impl Session {
    pub fn source_test_tree(&self, path: &str) -> Option<SourceTestNode<'_>> {
        let file = self.current_file(path)?;
        let (declarations, forms) = file.navigation_source()?;
        let mut tests = declarations.test_declarations(forms).peekable();
        tests.peek()?;
        if !self.has_standard_test() {
            return None;
        }
        let namespace = declarations
            .outline(forms)
            .into_iter()
            .find(|symbol| symbol.kind == SourceSymbolKind::Namespace)?;
        let references =
            self.collect_file_references(path, file, true, false, &mut BTreeMap::new());
        let testing_heads = references
            .uses
            .into_iter()
            .filter_map(|(span, reference)| {
                matches!(
                    reference,
                    ScopedReference::Referred("clojure.test", "testing")
                )
                .then_some(span.start.offset)
            })
            .collect();
        let children = tests
            .map(|(form, name, body)| {
                let mut children = Vec::new();
                testing_nodes(&file.source, body, &testing_heads, &mut children);
                SourceTestNode {
                    name: name.symbol().unwrap(),
                    kind: SourceTestKind::Test,
                    range: form.span,
                    name_range: name.without_metadata().span,
                    children,
                }
            })
            .collect();
        Some(SourceTestNode {
            name: namespace.name,
            kind: SourceTestKind::Namespace,
            range: namespace.range,
            name_range: namespace.selection,
            children,
        })
    }
}

fn testing_nodes<'a>(
    source: &'a str,
    forms: &'a [Form],
    heads: &BTreeSet<usize>,
    nodes: &mut Vec<SourceTestNode<'a>>,
) {
    for form in forms {
        match &form.without_metadata().kind {
            Kind::List(items) => {
                if let [head, label, body @ ..] = items.as_slice()
                    && heads.contains(&head.without_metadata().span.start.offset)
                {
                    let mut children = Vec::new();
                    testing_nodes(source, body, heads, &mut children);
                    let name = match &label.without_metadata().kind {
                        Kind::String(text) => text,
                        Kind::Nil => "",
                        _ => &source[label.span.start.offset..label.span.end.offset],
                    };
                    nodes.push(SourceTestNode {
                        name,
                        kind: SourceTestKind::Testing,
                        range: form.span,
                        name_range: head.without_metadata().span,
                        children,
                    });
                } else {
                    testing_nodes(source, items, heads, nodes);
                }
            }
            Kind::Vector(items)
            | Kind::Map(items)
            | Kind::Set(items)
            | Kind::AutoNamespacedMap { entries: items, .. } => {
                testing_nodes(source, items, heads, nodes)
            }
            Kind::AnonymousFunction(function) => {
                testing_nodes(source, &function.body, heads, nodes)
            }
            _ => {}
        }
    }
}
