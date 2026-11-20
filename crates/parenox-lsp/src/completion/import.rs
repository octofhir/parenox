//! Completion for Java class slots in a namespace import clause.
use parenox_analysis::syntax::{SyntaxKind, SyntaxToken};
use serde_json::{Value, json};

use super::DependencyCompletions;
use super::ns_form::{clause, forms, token_text};

pub(super) enum Target {
    Qualified,
    Grouped(String),
}

pub(super) fn target(token: &SyntaxToken) -> Option<Target> {
    if token.kind() != SyntaxKind::Symbol {
        return None;
    }
    let parent = token.parent()?;
    if clause(&parent, ":import") {
        return Some(Target::Qualified);
    }
    if !clause(&parent.parent()?, ":import") {
        return None;
    }
    let mut group = forms(&parent, SyntaxKind::OpenParen)
        .or_else(|| forms(&parent, SyntaxKind::OpenBracket))?;
    let head = group.next()?;
    let package = token_text(&head, SyntaxKind::Symbol)?;
    group
        .any(|element| element.as_token() == Some(token))
        .then(|| Target::Grouped(package.to_owned()))
}

pub(super) fn complete(
    target: Target,
    prefix: &str,
    range: &Value,
    limit: usize,
    dependencies: &DependencyCompletions,
) -> Vec<Value> {
    let (package, prefix, qualified) = match &target {
        Target::Qualified => {
            let Some((package, prefix)) = prefix.rsplit_once('.') else {
                return Vec::new();
            };
            (package, prefix, true)
        }
        Target::Grouped(package) => (package.as_str(), prefix, false),
    };
    dependencies
        .matching_java_classes(package, prefix, limit)
        .into_iter()
        .map(|name| {
            let full = format!("{package}.{name}");
            let label = if qualified { &full } else { &name };
            json!({"label":label,"kind":7,"detail":full,"textEdit":{"range":range,"newText":label}})
        })
        .collect()
}
