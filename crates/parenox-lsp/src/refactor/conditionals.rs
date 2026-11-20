use std::fmt::Write;

use parenox_analysis::Session;
use parenox_analysis::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
use serde_json::Value;

use crate::text::Lines;

use super::threading::{SPACE, active_call, end, list_forms, start, symbol_mentions};
use super::{push_trivia, replace_node};

pub(super) fn if_to_cond(
    session: &Session,
    path: &str,
    lines: &Lines<'_>,
    token: &SyntaxToken,
) -> Option<Vec<Value>> {
    let root = nearby_call(token, "if")?;
    if !active_call(&root) {
        return None;
    }
    let opening = usize::from(root.text_range().start());
    let cond = if session.core_completion_visible(path, "cond", opening)
        && symbol_mentions(&root, "cond").next().is_none()
    {
        "cond"
    } else if session.completion_namespace_at(path, "clojure.core", opening) == Some("clojure.core")
    {
        "clojure.core/cond"
    } else {
        return None;
    };
    let source = lines.source();
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let indent = lines.position(opening)?.character + 2;
    let mut text = format!("({cond}");
    let mut call = root.clone();
    let mut before_nested_if = "";
    let mut blank_before_pair = false;
    let mut tails = Vec::new();
    loop {
        let mut forms = list_forms(&call, true)?;
        let head = forms.next()?;
        let test = forms.next()?;
        let yes = forms.next()?;
        let no = forms.next();
        if forms.next().is_some()
            || [&test, &yes]
                .into_iter()
                .chain(no.as_ref())
                .any(conditional_part)
        {
            return None;
        }
        let open = usize::from(call.text_range().start());
        let leading = [
            before_nested_if,
            &source[open + 1..start(&head)],
            &source[end(&head)..start(&test)],
        ];
        let mut has_trivia = false;
        for gap in leading {
            has_trivia |= push_trivia(&mut text, gap);
        }
        if !has_trivia {
            if blank_before_pair {
                text.push_str(newline);
            }
            push_line(&mut text, newline, indent);
        }
        text.push_str(&source[start(&test)..end(&test)]);

        let before_yes = &source[end(&test)..start(&yes)];
        if !push_trivia(&mut text, before_yes) {
            push_line(&mut text, newline, indent);
        }
        text.push_str(&source[start(&yes)..end(&yes)]);

        let last = no.as_ref().unwrap_or(&yes);
        let closing = usize::from(call.text_range().end()) - 1;
        tails.push(&source[end(last)..closing]);
        let Some(no) = no else { break };
        let before_no = &source[end(&yes)..start(&no)];
        if let Some(nested) = no.as_node().filter(|node| head_is(node, "if")) {
            call = nested.clone();
            before_nested_if = before_no;
            blank_before_pair = true;
            continue;
        }
        if !push_trivia(&mut text, before_no) {
            text.push_str(newline);
            push_line(&mut text, newline, indent);
        }
        text.push_str(":else");
        push_line(&mut text, newline, indent);
        text.push_str(&source[start(&no)..end(&no)]);
        break;
    }
    for tail in tails.into_iter().rev() {
        push_trivia(&mut text, tail);
    }
    text.push(')');
    replace_node(lines, &root, text)
}

pub(super) fn cond_to_if(
    session: &Session,
    path: &str,
    lines: &Lines<'_>,
    token: &SyntaxToken,
) -> Option<Vec<Value>> {
    let root = nearby_call(token, "cond").or_else(|| nearby_call(token, "clojure.core/cond"))?;
    if !active_call(&root) {
        return None;
    }
    let mut forms = list_forms(&root, true)?;
    let head = forms.next()?;
    let head_start = start(&head);
    if head.as_token()?.text() == "cond" {
        if !session.core_completion_visible(path, "cond", head_start)
            || session.definition(path, head_start).is_some()
            || symbol_mentions(&root, "cond").any(|symbol| {
                symbol
                    .parent()
                    .and_then(|parent| list_forms(&parent, true)?.next())
                    != Some(SyntaxElement::Token(symbol))
            })
        {
            return None;
        }
    } else if session.completion_namespace_at(path, "clojure.core", head_start)
        != Some("clojure.core")
    {
        return None;
    }

    let args: Vec<_> = forms.collect();
    let (branches, unmatched) = args.as_chunks::<2>();
    if !unmatched.is_empty() || args.iter().any(conditional_part) {
        return None;
    }
    let source = lines.source();
    let opening = usize::from(root.text_range().start());
    let closing = usize::from(root.text_range().end()) - 1;
    let indent = lines.position(opening)?.character;
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let before_head = &source[opening + 1..head_start];
    let mut text = String::new();
    let mut previous_end = end(&head);
    let mut open_ifs = 0;
    if branches.is_empty() {
        if source[..opening]
            .chars()
            .next_back()
            .is_some_and(needs_separator)
        {
            text.push(' ');
        }
        text.push_str("nil");
        push_trivia(&mut text, before_head);
    }
    for (index, [test, result]) in branches.iter().enumerate() {
        let before_test = &source[previous_end..start(test)];
        let before_result = &source[end(test)..start(result)];
        // Keep auto-resolved keywords so the reader still checks their aliases.
        let plain_keyword = test.as_token().is_some_and(|token| {
            token.kind() == SyntaxKind::Keyword && !token.text().starts_with("::")
        });
        let final_else = plain_keyword && index > 0 && index + 1 == branches.len();
        let branch_indent = indent + 2 * open_ifs;
        if final_else {
            let test_trivia = push_trivia(&mut text, before_test);
            if !push_trivia(&mut text, before_result) && !test_trivia {
                push_line(&mut text, newline, branch_indent);
            }
        } else {
            if index > 0 && !push_trivia(&mut text, before_test) {
                push_line(&mut text, newline, branch_indent);
            }
            text.push_str("(if");
            let mut first_test_trivia = false;
            if index == 0 {
                first_test_trivia |= push_trivia(&mut text, before_head);
                first_test_trivia |= push_trivia(&mut text, before_test);
            }
            if !first_test_trivia {
                text.push(' ');
            }
            if plain_keyword {
                text.push_str("true");
            } else {
                text.push_str(&source[start(test)..end(test)]);
            }
            open_ifs += 1;
            if !push_trivia(&mut text, before_result) {
                push_line(&mut text, newline, branch_indent + 2);
            }
        }
        text.push_str(&source[start(result)..end(result)]);
        previous_end = end(result);
    }
    push_trivia(&mut text, &source[previous_end..closing]);
    if branches.is_empty()
        && !text.ends_with(SPACE)
        && source[closing + 1..]
            .chars()
            .next()
            .is_some_and(needs_separator)
    {
        text.push(' ');
    }
    text.extend(std::iter::repeat_n(')', open_ifs));
    replace_node(lines, &root, text)
}

fn nearby_call(token: &SyntaxToken, name: &str) -> Option<SyntaxNode> {
    let parent = token.parent()?;
    if head_is(&parent, name) {
        let mut forms = list_forms(&parent, true)?;
        let head = forms.next()?;
        if usize::from(token.text_range().start()) <= end(&head) {
            return Some(parent);
        }
    }
    if !matches!(token.kind(), SyntaxKind::Whitespace | SyntaxKind::Comma) {
        return None;
    }
    token
        .next_sibling_or_token()
        .and_then(SyntaxElement::into_node)
        .filter(|node| head_is(node, name))
}

fn head_is(call: &SyntaxNode, name: &str) -> bool {
    list_forms(call, true)
        .and_then(|mut forms| forms.next())
        .and_then(SyntaxElement::into_token)
        .is_some_and(|head| head.kind() == SyntaxKind::Symbol && head.text() == name)
}

fn conditional_part(part: &SyntaxElement) -> bool {
    part.kind() == SyntaxKind::Conditional
}

fn needs_separator(character: char) -> bool {
    !character.is_whitespace() && !"()[]{}\";,`~@^\\".contains(character)
}

fn push_line(text: &mut String, newline: &str, indent: usize) {
    write!(text, "{newline}{:indent$}", "").unwrap();
}
