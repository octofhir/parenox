//! Reorganize lookups while retaining their receiver, keys and trivia.
mod inverse;

pub(super) use inverse::unwind_get_in;

use parenox_analysis::Session;
use parenox_analysis::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
use serde_json::Value;

use super::threading::{SPACE, active_call, end, list_forms, start};
use super::{push_trivia, replace_node};
use crate::text::Lines;

struct Lookup<'a> {
    receiver: SyntaxElement,
    operator: &'a str,
    core_name: Option<&'static str>,
    keys: &'a str,
    leading: [&'a str; 3],
    trailing: &'a str,
    dispatch: Dispatch,
}

#[derive(Clone, Copy)]
enum Dispatch {
    Ordinary,
    UnprovenKeyword,
    QuotedOperator,
}

pub(super) fn get_in_all(
    session: &Session,
    path: &str,
    lines: &Lines<'_>,
    token: &SyntaxToken,
) -> Option<Vec<Value>> {
    let (call, root) = lookup_call(session, path, token)?;
    let at = usize::from(call.text_range().start());
    let source = lines.source();
    let mut lookup = lookup_at(session, path, source, &call)?;
    let mut dispatch = lookup.dispatch;
    let mut chain = Vec::new();
    while let Some(inner) = lookup
        .receiver
        .as_node()
        .and_then(|node| lookup_at(session, path, source, node))
    {
        use Dispatch::*;
        match (dispatch, inner.dispatch) {
            // A quoted-key chain uses ordinary lookup. Carry a deeper bare
            // keyword call as the receiver when its dispatch is unknown.
            (QuotedOperator, UnprovenKeyword) | (UnprovenKeyword, QuotedOperator) => break,
            (Ordinary, next) => dispatch = next,
            _ => {}
        }
        chain.push(lookup);
        lookup = inner;
    }
    let receiver = &source[start(&lookup.receiver)..end(&lookup.receiver)];
    chain.push(lookup);
    let outer = chain.first()?;
    let multiple = chain.len() > 1;
    if !multiple && outer.core_name.is_some() {
        return None;
    }
    let name = if multiple { "get-in" } else { "get" };
    let local = session.refactor_local_binding_at(path, name, at)?;
    if namespace_may_change(&root, at) {
        return None;
    }
    let core =
        session.completion_archive_namespace(path, "clojure.core", at) == Some("clojure.core");
    // The JVM inlines a two-argument get-in with a vector using clojure.core.
    if multiple && !core {
        return None;
    }
    let operator = if outer.core_name == Some(name) {
        outer.operator.to_owned()
    } else if !local && session.refactor_core_visible(path, name, at) {
        name.to_owned()
    } else if core {
        format!("clojure.core/{name}")
    } else {
        return None;
    };
    let mut text = format!("({operator} {receiver} ");
    if multiple {
        text.push('[');
    }
    for (index, step) in chain.into_iter().rev().enumerate() {
        if index > 0 && !text.ends_with(SPACE) {
            text.push(' ');
        }
        let mut trivia = false;
        for gap in step.leading {
            trivia |= push_trivia(&mut text, gap);
        }
        if trivia && !text.ends_with(SPACE) {
            text.push(' ');
        }
        text.push_str(step.keys);
        push_trivia(&mut text, step.trailing);
    }
    if multiple {
        text.push(']');
    }
    text.push(')');
    replace_node(lines, &call, text)
}

fn lookup_call(
    session: &Session,
    path: &str,
    token: &SyntaxToken,
) -> Option<(SyntaxNode, SyntaxNode)> {
    if token.kind() != SyntaxKind::OpenParen {
        return None;
    }
    let call = token.parent()?;
    if !active_call(&call)
        || call
            .ancestors()
            .any(|node| node.kind() == SyntaxKind::Conditional)
    {
        return None;
    }
    let at = usize::from(call.text_range().start());
    let root = call.ancestors().last()?;
    if path.ends_with(".cljc")
        && (session.completion_namespace_at(path, "cljs.core", at) != Some("cljs.core")
            || root
                .descendants()
                .take_while(|node| usize::from(node.text_range().start()) < at)
                .any(|node| node.kind() == SyntaxKind::Conditional))
    {
        return None;
    }
    Some((call, root))
}

fn namespace_may_change(root: &SyntaxNode, offset: usize) -> bool {
    root.descendants_with_tokens()
        .take_while(|part| usize::from(part.text_range().start()) < offset)
        .filter_map(SyntaxElement::into_token)
        .any(|token| {
            token.kind() == SyntaxKind::Symbol
                && matches!(
                    token.text().rsplit('/').next(),
                    Some(
                        "in-ns"
                            | "alias"
                            | "ns-unalias"
                            | "ns-unmap"
                            | "require"
                            | "use"
                            | "refer"
                            | "eval"
                            | "load"
                            | "load-file"
                            | "load-string"
                    )
                )
                && !token
                    .parent_ancestors()
                    .any(|node| matches!(node.kind(), SyntaxKind::Quote | SyntaxKind::Discard))
        })
}

fn lookup_at<'a>(
    session: &Session,
    path: &str,
    source: &'a str,
    call: &SyntaxNode,
) -> Option<Lookup<'a>> {
    use SyntaxKind::*;
    let mut parts = list_forms(call, true)?;
    let head = parts.next()?;
    let receiver = parts.next()?;
    let argument = parts.next();
    if parts.next().is_some() || receiver.kind() == Conditional {
        return None;
    }
    let from = usize::from(head.text_range().start());
    let through = usize::from(head.text_range().end());
    let portable = path.ends_with(".cljc");
    let operator = &source[from..through];
    let (core_name, keys, between, last) = match (head.kind(), argument) {
        (_, None) if callable_key(&head) => (None, operator, "", end(&receiver)),
        (Symbol, Some(argument)) if argument.kind() != Conditional => {
            let (name, keys) = match session.core_lookup_at(path, from)? {
                "get" if !portable || literal_key(&argument) => {
                    ("get", &source[start(&argument)..end(&argument)])
                }
                "get-in" => {
                    let vector = argument.as_node().filter(|node| node.kind() == Vector)?;
                    let mut keys = vector_items(vector).peekable();
                    if keys.peek().is_none()
                        || keys.any(|item| {
                            item.kind() == Conditional || portable && !literal_key(&item)
                        })
                    {
                        return None;
                    }
                    ("get-in", &source[start(&argument) + 1..end(&argument) - 1])
                }
                _ => return None,
            };
            (
                Some(name),
                keys,
                &source[end(&receiver)..start(&argument)],
                end(&argument),
            )
        }
        _ => return None,
    };
    let open = usize::from(call.text_range().start());
    let close = usize::from(call.text_range().end()) - 1;
    let leading = [
        &source[open + 1..from],
        &source[through..start(&receiver)],
        between,
    ];
    let dispatch = match (core_name, head.kind()) {
        (None, Keyword) if !literal_receiver(&receiver) => Dispatch::UnprovenKeyword,
        (None, List | Quote) => Dispatch::QuotedOperator,
        _ => Dispatch::Ordinary,
    };
    Some(Lookup {
        receiver,
        operator,
        core_name,
        keys,
        leading,
        trailing: &source[last..close],
        dispatch,
    })
}

fn vector_items(node: &SyntaxNode) -> impl Iterator<Item = SyntaxElement> + use<> {
    use SyntaxKind::*;
    node.children_with_tokens().filter(|part| {
        !part.kind().is_trivia() && !matches!(part.kind(), OpenBracket | CloseBracket | Discard)
    })
}

fn literal_key(form: &SyntaxElement) -> bool {
    use SyntaxKind::*;
    match form.kind() {
        Keyword | String | Number | Character | Boolean | Nil | Quote => true,
        Vector => form
            .as_node()
            .is_some_and(|node| vector_items(node).all(|item| literal_key(&item))),
        _ => false,
    }
}

fn literal_receiver(form: &SyntaxElement) -> bool {
    use SyntaxKind::*;
    matches!(form.kind(), Map | NamespacedMap | Vector | Set | Nil)
}

fn callable_key(key: &SyntaxElement) -> bool {
    use SyntaxKind::*;
    match key.kind() {
        Keyword => true,
        Quote => key.as_node().is_some_and(|node| {
            node.children_with_tokens()
                .find(|part| {
                    !part.kind().is_trivia() && !matches!(part.kind(), QuoteMark | Discard)
                })
                .is_some_and(|target| matches!(target.kind(), Keyword | Symbol))
        }),
        List => key
            .as_node()
            .and_then(|node| list_forms(node, true))
            .is_some_and(|mut forms| {
                let head = forms.next().and_then(SyntaxElement::into_token);
                head.is_some_and(|token| token.kind() == Symbol && token.text() == "quote")
                    && forms
                        .next()
                        .is_some_and(|target| matches!(target.kind(), Keyword | Symbol))
                    && forms.next().is_none()
            }),
        _ => false,
    }
}
