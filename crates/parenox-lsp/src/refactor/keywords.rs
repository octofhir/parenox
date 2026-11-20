//! Change the spelling of a keyword without changing its resolved namespace.
use parenox_analysis::Session;
use parenox_analysis::reader::Span;
use parenox_analysis::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use serde_json::{Value, json};

use crate::text::{Lines, Range};

pub(super) struct Cycle {
    token: SyntaxToken,
    replacement: String,
    title: &'static str,
}

impl Cycle {
    pub(super) fn title(&self) -> &'static str {
        self.title
    }

    pub(super) fn edit(self, lines: &Lines<'_>) -> Option<Value> {
        let bytes = self.token.text_range();
        let range = Range {
            start: lines.position(bytes.start().into())?,
            end: lines.position(bytes.end().into())?,
        };
        Some(json!({"range":range,"newText":self.replacement}))
    }
}

pub(super) fn cycle(
    session: &Session,
    path: &str,
    token: &SyntaxToken,
    offset: usize,
) -> Option<Cycle> {
    let token = if token.kind() == SyntaxKind::Keyword {
        token.clone()
    } else {
        token.prev_token().filter(|previous| {
            previous.kind() == SyntaxKind::Keyword
                && usize::from(previous.text_range().end()) == offset
        })?
    };
    let text = token.text();
    let (explicit, name) = if let Some(name) = text.strip_prefix("::") {
        (None, name)
    } else {
        let (namespace, name) = text.strip_prefix(':')?.split_once('/')?;
        (Some(namespace), name)
    };
    // A slash in the local name cannot be expressed as an unaliased :: token.
    if name.is_empty()
        || name.contains('/')
        || token.parent_ancestors().any(|parent| {
            matches!(
                parent.kind(),
                SyntaxKind::Discard
                    | SyntaxKind::Conditional
                    | SyntaxKind::Tagged
                    | SyntaxKind::Error
            )
        })
    {
        return None;
    }
    let start = usize::from(token.text_range().start());
    let (namespace, site) = session.reader_namespace_before(path, start)?;
    if explicit.is_some_and(|written| written != namespace) {
        return None;
    }
    let root = token.parent_ancestors().last()?;
    let declaration = namespace_form(&root, site)?;
    // A form is read before it runs. Inspect preceding forms for namespace
    // changes, including the branches of portable reader conditionals.
    if renames_namespace_operation(&declaration)
        || root
            .children()
            .take_while(|form| usize::from(form.text_range().end()) <= start)
            .any(|form| form != declaration && changes_namespace(&form))
    {
        return None;
    }

    let (replacement, title) = if explicit.is_some() {
        (
            format!("::{name}"),
            "Change namespaced keyword to auto-resolved",
        )
    } else {
        (
            format!(":{namespace}/{name}"),
            "Change auto-resolved keyword to namespaced",
        )
    };
    Some(Cycle {
        token,
        replacement,
        title,
    })
}

fn namespace_form(root: &SyntaxNode, site: Span) -> Option<SyntaxNode> {
    let start = u32::try_from(site.start.offset).ok()?;
    let token = root.token_at_offset(start.into()).right_biased()?;
    if token.kind() != SyntaxKind::Symbol
        || token
            .parent_ancestors()
            .any(|parent| parent.kind() == SyntaxKind::Conditional)
    {
        return None;
    }
    token
        .parent_ancestors()
        .find(|form| form.parent().as_ref() == Some(root))
}

fn renames_namespace_operation(declaration: &SyntaxNode) -> bool {
    declaration
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Keyword && token.text() == ":rename")
        .any(|token| {
            std::iter::successors(token.next_sibling_or_token(), |part| {
                part.next_sibling_or_token()
            })
            .find(|part| !part.kind().is_trivia() && part.kind() != SyntaxKind::Discard)
            .and_then(|part| part.into_node())
            .is_some_and(|form| changes_namespace(&form))
        })
}

fn changes_namespace(form: &SyntaxNode) -> bool {
    form.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Symbol)
        .any(|token| {
            let name = token.text().rsplit('/').next().unwrap_or_default();
            matches!(
                name,
                "ns" | "in-ns" | "*ns*" | "eval" | "load" | "load-file" | "load-string"
            ) && !token
                .parent_ancestors()
                .any(|parent| matches!(parent.kind(), SyntaxKind::Quote | SyntaxKind::Discard))
        })
}
