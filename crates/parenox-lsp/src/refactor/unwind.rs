use parenox_analysis::Session;
use parenox_analysis::syntax::{SyntaxElement, SyntaxKind, SyntaxToken};
use serde_json::Value;

use crate::text::Lines;

use super::replace_node;
use super::threading::{Direction, SPACE, active_call, end, list_forms, start, symbol_mentions};

pub(super) fn unwind(
    session: &Session,
    path: &str,
    lines: &Lines<'_>,
    token: &SyntaxToken,
    all: bool,
) -> Option<Vec<Value>> {
    let (call, direction) = token.parent()?.ancestors().find_map(|node| {
        let head = list_forms(&node, true)?.next()?.into_token()?;
        let direction = Direction::from_arrow(head.text())?;
        Some((node, direction))
    })?;
    if !active_call(&call) {
        return None;
    }

    let mut forms = list_forms(&call, true)?;
    let head = forms.next()?;
    let arrow = head.as_token()?.text();
    if !arrow.contains('/')
        && (!session.core_completion_visible(path, arrow, start(&head))
            || session.definition(path, start(&head)).is_some()
            || symbol_mentions(&call, arrow).any(|other| {
                other
                    .parent()
                    .and_then(|parent| list_forms(&parent, true)?.next())
                    != Some(SyntaxElement::Token(other))
            }))
    {
        return None;
    }
    let seed = forms.next()?;
    if seed.kind() == SyntaxKind::Conditional
        || forms
            .clone()
            .any(|form| form.kind() == SyntaxKind::Conditional)
    {
        return None;
    }
    let source = lines.source();
    let value = &source[start(&seed)..end(&seed)];
    let step = forms.next()?;
    let first = Step::of(&step, source, direction)?;
    let range = call.text_range();
    let opening = usize::from(range.start());
    let closing = usize::from(range.end()) - 1;

    let mut new_text = String::new();
    if !all && forms.next().is_some() {
        new_text.push_str(&source[opening..start(&seed)]);
        push_trivia(
            &mut new_text,
            trivia(&source[end(&seed)..start(&step)]).trim_start_matches(SPACE),
        );
        first.prefix(&mut new_text);
        new_text.push_str(value);
        first.suffix(&mut new_text, "", "");
        new_text.push_str(&source[end(&step)..=closing]);
    } else {
        let mut steps = vec![(first, trivia(&source[end(&seed)..start(&step)]))];
        let mut last = end(&step);
        for next in forms {
            steps.push((
                Step::of(&next, source, direction)?,
                trivia(&source[last..start(&next)]),
            ));
            last = end(&next);
        }

        for (step, _) in steps.iter().rev() {
            step.prefix(&mut new_text);
        }
        new_text.push_str(trivia(&source[end(&head)..start(&seed)]));
        new_text.push_str(value);
        let mut steps = steps.into_iter();
        let (final_step, final_gap) = steps.next_back()?;
        for (step, gap) in steps {
            step.suffix(&mut new_text, gap, "");
        }
        final_step.suffix(&mut new_text, final_gap, trivia(&source[last..closing]));
    }
    replace_node(lines, &call, new_text)
}

fn trivia(gap: &str) -> &str {
    if gap.trim_matches(SPACE).is_empty() {
        ""
    } else {
        gap
    }
}

fn push_trivia(text: &mut String, gap: &str) {
    if gap.starts_with("#_") && !text.ends_with(SPACE) {
        text.push(' ');
    }
    text.push_str(gap);
}

struct Step<'a> {
    before: &'a str,
    after: &'a str,
    bare: bool,
}

impl<'a> Step<'a> {
    fn of(form: &SyntaxElement, source: &'a str, direction: Direction) -> Option<Self> {
        use SyntaxKind::*;
        if matches!(form.kind(), Symbol | Keyword) {
            return Some(Self {
                before: &source[start(form)..end(form)],
                after: ")",
                bare: true,
            });
        }
        let mut parts = list_forms(form.as_node()?, true)?;
        let head = parts.next()?;
        if head.kind() == Conditional || parts.clone().any(|part| part.kind() == Conditional) {
            return None;
        }
        let anchor = match direction {
            Direction::First => head,
            Direction::Last => parts.last().unwrap_or(head),
        };
        let insertion = end(&anchor);
        Some(Self {
            before: &source[start(form)..insertion],
            after: &source[insertion..end(form)],
            bare: false,
        })
    }

    fn prefix(&self, text: &mut String) {
        if self.bare {
            text.push('(');
        }
        text.push_str(self.before);
        text.push(' ');
    }

    fn suffix(&self, text: &mut String, gap: &str, tail: &str) {
        push_trivia(text, gap);
        if gap.is_empty()
            && self
                .after
                .starts_with(|ch: char| !ch.is_whitespace() && !"()[]{};,\"".contains(ch))
        {
            text.push(' ');
        }
        text.push_str(&self.after[..self.after.len() - 1]);
        push_trivia(text, tail);
        text.push(')');
    }
}
