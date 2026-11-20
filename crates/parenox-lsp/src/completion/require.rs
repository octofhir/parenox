//! Completion for namespace and referral slots in a top-level ns form.
use parenox_analysis::Session;
use parenox_analysis::syntax::{SyntaxKind, SyntaxToken};
use serde_json::{Value, json};

use super::DependencyCompletions;
use super::ns_form::{clause, forms, token_text};

pub(super) enum Target {
    Namespace,
    Refer(String),
}

pub(super) fn target(token: &SyntaxToken) -> Option<Target> {
    if token.kind() != SyntaxKind::Symbol {
        return None;
    }
    let require_clause = |node| clause(node, ":require");
    let parent = token.parent()?;
    if require_clause(&parent) {
        return Some(Target::Namespace);
    }
    let enclosing = parent.parent()?;
    if require_clause(&enclosing) {
        let head = forms(&parent, SyntaxKind::OpenBracket)?.next()?;
        return (head.as_token() == Some(token)).then_some(Target::Namespace);
    }
    if forms(&parent, SyntaxKind::OpenBracket).is_none()
        && forms(&parent, SyntaxKind::OpenParen).is_none()
    {
        return None;
    }
    require_clause(&enclosing.parent()?).then_some(())?;
    let mut libspec = forms(&enclosing, SyntaxKind::OpenBracket)?;
    let namespace = libspec.next()?.into_token()?;
    if namespace.kind() != SyntaxKind::Symbol {
        return None;
    }
    while let (Some(key), Some(value)) = (libspec.next(), libspec.next()) {
        if value.as_node() == Some(&parent)
            && token_text(&key, SyntaxKind::Keyword) == Some(":refer")
        {
            return Some(Target::Refer(namespace.text().to_owned()));
        }
    }
    None
}

pub(super) fn complete(
    session: &Session,
    path: &str,
    target: Target,
    prefix: &str,
    range: &Value,
    limit: usize,
    dependencies: &DependencyCompletions,
) -> Vec<Value> {
    let item = |name: &str, kind, detail: &str| json!({"label":name,"kind":kind,"detail":detail,"textEdit":{"range":range,"newText":name}});
    match target {
        Target::Namespace => session
            .complete_require_namespace(path, prefix, limit)
            .into_iter()
            .map(|name| item(name, 9, "Project namespace"))
            .collect(),
        Target::Refer(namespace) => {
            if let Some(names) = session.complete_require_refer(&namespace, prefix, limit) {
                names
                    .into_iter()
                    .map(|name| item(name, 6, &namespace))
                    .collect()
            } else {
                dependencies
                    .matching(&namespace, prefix, limit)
                    .into_iter()
                    .map(|export| {
                        item(
                            &export.name,
                            if export.callable { 3 } else { 6 },
                            &namespace,
                        )
                    })
                    .collect()
            }
        }
    }
}
