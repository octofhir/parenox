//! Expose a path as calls to literal keywords or symbols.
use parenox_analysis::Session;
use parenox_analysis::reader::MAX_NESTING_DEPTH;
use parenox_analysis::syntax::{SyntaxElement, SyntaxKind, SyntaxToken};
use serde_json::Value;

use super::{
    SPACE, callable_key, end, list_forms, literal_key, literal_receiver, lookup_call,
    namespace_may_change, push_trivia, replace_node, start, vector_items,
};
use crate::text::Lines;

pub(in crate::refactor) fn unwind_get_in(
    session: &Session,
    path: &str,
    lines: &Lines<'_>,
    token: &SyntaxToken,
    all: bool,
) -> Option<Vec<Value>> {
    use SyntaxKind::{Conditional, Symbol, Vector};
    let (call, root) = lookup_call(session, path, token)?;
    let mut forms = list_forms(&call, true)?;
    let head = forms.next()?;
    let receiver = forms.next()?;
    let argument = forms.next()?;
    let default = forms.next();
    if head.kind() != Symbol
        || forms.next().is_some()
        || [&receiver, &argument]
            .into_iter()
            .chain(default.as_ref())
            .any(|form| form.kind() == Conditional)
    {
        return None;
    }
    let name = session.core_lookup_at(path, start(&head))?;
    let at = usize::from(call.text_range().start());
    let close = usize::from(call.text_range().end()) - 1;
    if namespace_may_change(&root, at) {
        return None;
    }
    let source = lines.source();
    let form_text = |form: &SyntaxElement| &source[start(form)..end(form)];
    let after_head = &source[end(&head)..start(&receiver)];
    let before_key = &source[end(&receiver)..start(&argument)];
    let mut text = String::from("(");
    push_trivia(&mut text, &source[at + 1..start(&head)]);
    if name == "get" {
        if !callable_key(&argument) {
            return None;
        }
        push_callable(
            &mut text,
            source,
            &argument,
            Some(&receiver),
            default.is_some(),
        );
        push_trivia(&mut text, after_head);
        push_argument(&mut text, form_text(&receiver));
        push_trivia(&mut text, before_key);
        if let Some(default) = &default {
            push_trivia(&mut text, &source[end(&argument)..start(default)]);
            push_argument(&mut text, form_text(default));
        }
        push_trivia(
            &mut text,
            &source[end(default.as_ref().unwrap_or(&argument))..close],
        );
    } else {
        // Three-argument get-in passes a private not-found sentinel through
        // each lookup. A keyword call with the default cannot reproduce that.
        if default.is_some()
            || session.completion_archive_namespace(path, "clojure.core", at)
                != Some("clojure.core")
        {
            return None;
        }
        let vector = argument.as_node().filter(|node| node.kind() == Vector)?;
        let mut keys = vector_items(vector).peekable();
        let first = keys.next()?;
        if !callable_key(&first) {
            return None;
        }
        let path_start = &source[start(&argument) + 1..start(&first)];
        let path_end = end(&argument) - 1;
        if all {
            let keys: Vec<_> = std::iter::once(first)
                .chain(keys)
                .take(MAX_NESTING_DEPTH + 1)
                .collect();
            // Each promoted key adds at most one enclosing level, including
            // the quote wrapper next to the deepest receiver.
            let depth = MAX_NESTING_DEPTH.checked_sub(keys.len())?;
            if !keys.iter().all(callable_key)
                || call
                    .descendants()
                    .any(|node| node.ancestors().count() > depth)
            {
                return None;
            }
            for (index, key) in keys.iter().enumerate().rev() {
                // The result of a nested lookup can be any object, even
                // when the original receiver is a built-in literal.
                let seed = (index == 0).then_some(&receiver);
                push_callable(&mut text, source, key, seed, false);
                if index > 0 {
                    text.push_str(" (");
                }
            }
            push_trivia(&mut text, after_head);
            push_argument(&mut text, form_text(&receiver));
            push_trivia(&mut text, before_key);
            push_trivia(&mut text, path_start);
            for (index, key) in keys.iter().enumerate() {
                if let Some(next) = keys.get(index + 1) {
                    push_trivia(&mut text, &source[end(key)..start(next)]);
                    text.push(')');
                } else {
                    push_trivia(&mut text, &source[end(key)..path_end]);
                }
            }
        } else if let Some(second) = keys.next() {
            let vector_output = keys.peek().is_some();
            // Unlike the JVM inline expansion, ClojureScript evaluates the
            // entire path before looking up its first key.
            let fixed_path = path.ends_with(".cljc");
            if std::iter::once(second.clone()).chain(keys).any(|key| {
                key.kind() == Conditional || fixed_path && !literal_key(&key) && !callable_key(&key)
            }) {
                return None;
            }
            let operator = if vector_output {
                form_text(&head)
            } else {
                let local = session.refactor_local_binding_at(path, "get", at)?;
                if !local && session.refactor_core_visible(path, "get", at) {
                    "get"
                } else {
                    "clojure.core/get"
                }
            };
            text.push_str(operator);
            push_trivia(&mut text, after_head);
            push_argument(&mut text, "(");
            push_callable(&mut text, source, &first, Some(&receiver), false);
            push_argument(&mut text, form_text(&receiver));
            text.push(')');
            push_trivia(&mut text, before_key);
            if vector_output {
                push_argument(&mut text, "[");
            }
            let before_first = push_trivia(&mut text, path_start);
            let after_first = push_trivia(&mut text, &source[end(&first)..start(&second)]);
            if vector_output {
                let rest = &source[start(&second)..end(&argument)];
                if before_first || after_first {
                    push_argument(&mut text, rest);
                } else {
                    text.push_str(rest);
                }
            } else {
                push_argument(&mut text, form_text(&second));
                push_trivia(&mut text, &source[end(&second)..path_end]);
            }
        } else {
            push_callable(&mut text, source, &first, Some(&receiver), false);
            push_trivia(&mut text, after_head);
            push_argument(&mut text, form_text(&receiver));
            push_trivia(&mut text, before_key);
            push_trivia(&mut text, path_start);
            push_trivia(&mut text, &source[end(&first)..path_end]);
        }
        push_trivia(&mut text, &source[end(&argument)..close]);
    }
    text.push(')');
    replace_node(lines, &call, text)
}

fn push_callable(
    text: &mut String,
    source: &str,
    key: &SyntaxElement,
    receiver: Option<&SyntaxElement>,
    has_default: bool,
) {
    use SyntaxKind::*;
    let key_text = &source[start(key)..end(key)];
    // A bare keyword with one argument can invoke the JVM's IKeywordLookup
    // specialization. Quoting it keeps ordinary lookup for unknown receivers.
    if key.kind() == Keyword && !has_default && !receiver.is_some_and(literal_receiver) {
        text.push_str("(quote ");
        text.push_str(key_text);
        text.push(')');
    } else {
        text.push_str(key_text);
    }
}

fn push_argument(text: &mut String, argument: &str) {
    if !text.ends_with(SPACE) {
        text.push(' ');
    }
    text.push_str(argument);
}
