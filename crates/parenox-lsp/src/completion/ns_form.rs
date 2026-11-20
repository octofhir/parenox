//! Recognize top-level namespace clauses in the lossless syntax tree.
use parenox_analysis::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

pub(super) fn clause(node: &SyntaxNode, keyword: &str) -> bool {
    let Some(head) = forms(node, SyntaxKind::OpenParen).and_then(|mut forms| forms.next()) else {
        return false;
    };
    if token_text(&head, SyntaxKind::Keyword) != Some(keyword) {
        return false;
    }
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent
        .parent()
        .is_none_or(|root| root.kind() != SyntaxKind::Root)
    {
        return false;
    }
    let Some(mut ns) = forms(&parent, SyntaxKind::OpenParen) else {
        return false;
    };
    ns.next().is_some_and(|head| {
        matches!(
            token_text(&head, SyntaxKind::Symbol),
            Some("ns" | "clojure.core/ns")
        )
    }) && ns
        .next()
        .is_some_and(|name| matches!(name.kind(), SyntaxKind::Symbol | SyntaxKind::Metadata))
}

pub(super) fn forms(
    node: &SyntaxNode,
    opener: SyntaxKind,
) -> Option<impl Iterator<Item = SyntaxElement>> {
    use SyntaxKind::*;
    if !matches!(node.kind(), List | Vector | Error) {
        return None;
    }
    let mut elements = node
        .children_with_tokens()
        .filter(|element| !element.kind().is_trivia() && element.kind() != Discard);
    (elements.next()?.kind() == opener).then(|| {
        elements.take_while(|element| !matches!(element.kind(), CloseParen | CloseBracket))
    })
}

pub(super) fn token_text(element: &SyntaxElement, kind: SyntaxKind) -> Option<&str> {
    element
        .as_token()
        .filter(|token| token.kind() == kind)
        .map(SyntaxToken::text)
}
