use std::fmt::Write;

use parenox_analysis::Session;
use parenox_analysis::syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
use serde_json::Value;

use crate::text::Lines;

use super::replace_node;

pub(super) const SPACE: [char; 5] = [' ', '\t', '\n', '\r', ','];

#[derive(Clone, Copy)]
pub(super) enum Direction {
    First,
    Last,
}

impl Direction {
    pub(super) fn from_arrow(arrow: &str) -> Option<Self> {
        match raw_name(arrow) {
            "->" => Some(Self::First),
            "->>" => Some(Self::Last),
            _ => None,
        }
    }

    fn arrow(self) -> &'static str {
        match self {
            Self::First => "->",
            Self::Last => "->>",
        }
    }
}

pub(super) fn thread(
    session: &Session,
    path: &str,
    lines: &Lines<'_>,
    token: &SyntaxToken,
    direction: Direction,
    all: bool,
) -> Option<Vec<Value>> {
    use SyntaxKind::*;
    let source = lines.source();
    let on_call = token.kind() == OpenParen
        || matches!(direction, Direction::Last) && token.kind() == CloseParen
        || all
            && matches!(
                token.kind(),
                CloseParen | Symbol | Keyword | Number | String | Character | Boolean | Nil | Regex
            );
    if !on_call {
        return None;
    }
    let call = token.parent()?;
    if !active_call(&call) {
        return None;
    }
    let (mut input, step) = extract(&call, source, direction, all)?;
    let mut steps = vec![step];
    if all {
        while let Some((arg, step)) = input
            .as_node()
            .and_then(|node| extract(node, source, direction, all))
        {
            input = arg;
            steps.push(step);
        }
    }

    let start_offset = usize::from(call.text_range().start());
    let arrow = direction.arrow();
    let qualified = if session.core_completion_visible(path, arrow, start_offset)
        && symbol_mentions(&call, arrow).next().is_none()
    {
        arrow.to_owned()
    } else {
        format!("clojure.core/{arrow}")
    };
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let indent = lines.position(start_offset)?.character + qualified.len() + 2;
    let input = &source[start(&input)..end(&input)];
    let mut new_text = format!("({qualified} {input}");
    for step in steps.iter().rev() {
        write!(new_text, "{newline}{:indent$}{step}", "").ok()?;
    }
    new_text.push(')');
    replace_node(lines, &call, new_text)
}

fn extract(
    call: &SyntaxNode,
    source: &str,
    direction: Direction,
    all: bool,
) -> Option<(SyntaxElement, String)> {
    use SyntaxKind::*;
    let preserve_trivia = all || matches!(direction, Direction::Last);
    let mut parts = list_forms(call, preserve_trivia)?;
    let head = parts.next()?.into_token()?;
    if !matches!(head.kind(), Symbol | Keyword) || blocked_head(head.text()) {
        return None;
    }
    let first = parts.next()?;
    if matches!(first.kind(), Conditional | Discard)
        || parts
            .clone()
            .any(|form| matches!(form.kind(), Conditional | Discard))
    {
        return None;
    }
    let (input, has_other_args) = match direction {
        Direction::First => (first, parts.next().is_some()),
        Direction::Last => match parts.last() {
            Some(last) => (last, true),
            None => (first, false),
        },
    };
    if !has_other_args {
        if !call
            .children_with_tokens()
            .any(|part| matches!(part.kind(), Comment | Discard))
        {
            return Some((input, head.text().to_owned()));
        }
        if !preserve_trivia {
            return None;
        }
    }

    let prefix = &source[usize::from(call.text_range().start())..start(&input)];
    let suffix =
        source[end(&input)..usize::from(call.text_range().end())].trim_start_matches(SPACE);
    let prefix = match direction {
        Direction::First => prefix,
        Direction::Last => trim_before_last(prefix, &input),
    };
    let needs_space = !prefix.ends_with(SPACE)
        && (suffix.starts_with("#_")
            || matches!(direction, Direction::First)
                && has_other_args
                && usize::from(head.text_range().end()) == start(&input));
    let separator = if needs_space { " " } else { "" };
    Some((input, format!("{prefix}{separator}{suffix}")))
}

fn trim_before_last<'a>(prefix: &'a str, input: &SyntaxElement) -> &'a str {
    let previous = std::iter::successors(input.prev_sibling_or_token(), |part| {
        part.prev_sibling_or_token()
    })
    .find(|part| !matches!(part.kind(), SyntaxKind::Whitespace | SyntaxKind::Comma));
    let trimmed = prefix.trim_end_matches(SPACE);
    // Keep the line break that terminates the comment after removing the next form.
    if previous.is_some_and(|part| part.kind() == SyntaxKind::Comment)
        && let Some(newline) = prefix[trimmed.len()..].find(['\r', '\n'])
    {
        let offset = trimmed.len() + newline;
        let line_break = if prefix[offset..].starts_with("\r\n") {
            2
        } else {
            1
        };
        return &prefix[..offset + line_break];
    }
    trimmed
}

pub(super) fn list_forms(
    call: &SyntaxNode,
    skip_discards: bool,
) -> Option<impl Iterator<Item = SyntaxElement> + Clone + use<>> {
    use SyntaxKind::*;
    if call.kind() != List || call.last_child_or_token()?.kind() != CloseParen {
        return None;
    }
    Some(call.children_with_tokens().skip(1).filter(move |part| {
        !(part.kind().is_trivia()
            || part.kind() == CloseParen
            || skip_discards && part.kind() == Discard)
    }))
}

pub(super) fn start(form: &SyntaxElement) -> usize {
    usize::from(form.text_range().start())
}

pub(super) fn end(form: &SyntaxElement) -> usize {
    usize::from(form.text_range().end())
}

pub(super) fn active_call(call: &SyntaxNode) -> bool {
    use SyntaxKind::*;
    call.kind() == List
        && !call.ancestors().skip(1).any(inactive_context)
        && !call
            .descendants_with_tokens()
            .any(|part| matches!(part.kind(), Error | ErrorToken))
}

fn raw_name(name: &str) -> &str {
    name.strip_prefix("clojure.core/").unwrap_or(name)
}

fn blocked_head(name: &str) -> bool {
    matches!(
        raw_name(name),
        "def"
            | "defn"
            | "defn-"
            | "defmacro"
            | "deftest"
            | "testing"
            | "if"
            | "when"
            | "fn"
            | "fn*"
            | "let"
            | "let*"
            | "loop"
            | "loop*"
            | ":require"
            | ":import"
    ) || inert_or_threading_head(name)
}

fn inert_or_threading_head(name: &str) -> bool {
    matches!(
        raw_name(name),
        "quote"
            | "comment"
            | "ns"
            | "case"
            | "letfn"
            | "defprotocol"
            | "defrecord"
            | "deftype"
            | "reify"
            | "proxy"
            | "extend-type"
            | "extend-protocol"
            | "->"
            | "->>"
            | "some->"
            | "some->>"
            | "cond->"
            | "cond->>"
            | "as->"
            | "doto"
    )
}

fn inactive_context(node: SyntaxNode) -> bool {
    use SyntaxKind::*;
    match node.kind() {
        Quote | SyntaxQuote | VarQuote | Discard | Metadata | Tagged | Error => true,
        List => node
            .children_with_tokens()
            .skip(1)
            .find(|part| !part.kind().is_trivia() && part.kind() != Discard)
            .and_then(SyntaxElement::into_token)
            .is_some_and(|head| inert_or_threading_head(head.text())),
        _ => false,
    }
}

pub(super) fn symbol_mentions<'a>(
    call: &SyntaxNode,
    name: &'a str,
) -> impl Iterator<Item = SyntaxToken> + use<'a> {
    call.ancestors()
        .take_while(|node| node.kind() != SyntaxKind::Root)
        .last()
        .into_iter()
        .flat_map(|top| top.descendants_with_tokens())
        .filter_map(SyntaxElement::into_token)
        .filter(move |token| {
            token.kind() == SyntaxKind::Symbol && token.text().rsplit('/').next() == Some(name)
        })
}
