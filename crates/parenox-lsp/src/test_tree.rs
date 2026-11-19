//! Clojure test tree notification payload.
use parenox_analysis::{Session, SourceTestKind, SourceTestNode};
use serde_json::{Value, json};

use crate::text::Lines;

pub fn params(session: &Session, path: &str, uri: &str, source: &str) -> Option<Value> {
    let tree = session.source_test_tree(path)?;
    Some(json!({"uri":uri,"tree":node(&Lines::new(source), tree)?}))
}

fn node(lines: &Lines<'_>, tree: SourceTestNode<'_>) -> Option<Value> {
    let kind = match tree.kind {
        SourceTestKind::Namespace => 1,
        SourceTestKind::Test => 2,
        SourceTestKind::Testing => 3,
    };
    let mut value = json!({
        "name":tree.name,
        "range":lines.range(tree.range)?,
        "nameRange":lines.range(tree.name_range)?,
        "kind":kind,
    });
    if tree.kind != SourceTestKind::Testing || !tree.children.is_empty() {
        let children = tree
            .children
            .into_iter()
            .map(|child| node(lines, child))
            .collect::<Option<Vec<_>>>()?;
        value["children"] = json!(children);
    }
    Some(value)
}
