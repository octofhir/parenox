//! Source-local filtering after checking. The checker never trusts a suppression as a type fact.
use std::collections::BTreeMap;

use crate::reader::{Form, Kind, LineComment, Span};
use crate::{Diagnostic, Report, SuppressedDiagnostic};

pub(crate) const MARKER: &str = "parenox-ignore";

#[derive(Default)]
pub(crate) struct Suppressions {
    rules: Vec<Rule>,
    pub invalid: Vec<Diagnostic>,
}

struct Rule {
    code: &'static str,
    target: Span,
    directive: Span,
    reason: String,
}

fn suppressible(code: &str) -> Option<&'static str> {
    match code {
        "PNX2322" => Some("PNX2322"),
        "PNX2741" => Some("PNX2741"),
        "PNX2801" => Some("PNX2801"),
        "PNX2802" => Some("PNX2802"),
        _ => None,
    }
}

impl Suppressions {
    pub(crate) fn read(source: &str, forms: &[Form], comments: Vec<LineComment>) -> Self {
        let mut result = Self::default();
        if comments.is_empty() {
            return result;
        }
        let mut targets = comments
            .iter()
            .map(|comment| (comment.following_offset, None))
            .collect::<BTreeMap<_, _>>();
        let mut quoted = Vec::new();
        for form in forms {
            locate_targets(form, &mut targets, &mut quoted);
        }
        for comment in comments {
            let span = comment.span;
            if quoted.iter().any(|&quote| covers(quote, span)) {
                continue;
            }
            let text = source[span.start.offset..span.end.offset]
                .trim_start_matches(';')
                .trim_start()
                .strip_prefix(MARKER)
                .unwrap();
            let invalid = |message| Diagnostic::new("PNX9006", span, message);
            let Some((code, reason)) = text.split_once(':') else {
                result
                    .invalid
                    .push(invalid("Expected parenox-ignore CODE: reason"));
                continue;
            };
            let Some(code) = suppressible(code.trim()) else {
                result.invalid.push(Diagnostic::new(
                    "PNX9006",
                    span,
                    format!("Cannot suppress diagnostic {}", code.trim()),
                ));
                continue;
            };
            let reason = reason.trim();
            if reason.is_empty() {
                result
                    .invalid
                    .push(invalid("parenox-ignore requires a nonempty reason"));
                continue;
            }
            let Some(target) = targets[&comment.following_offset] else {
                result.invalid.push(invalid(
                    "parenox-ignore must precede a source form in the same collection",
                ));
                continue;
            };
            result.rules.push(Rule {
                code,
                target,
                directive: span,
                reason: reason.into(),
            });
        }
        // A narrower directive accounts for findings before its enclosing directive.
        result.rules.sort_by_key(|rule| {
            (
                rule.target.end.offset - rule.target.start.offset,
                rule.directive.start.offset,
            )
        });
        result
    }

    pub(crate) fn has_rules(&self) -> bool {
        !self.rules.is_empty()
    }

    pub(crate) fn apply(&self, report: &mut Report) -> Vec<Diagnostic> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        // A parser, resolver or checker error may have prevented a finding. Do
        // not claim the directive was unused if checking stopped or was incomplete.
        let fully_checked = report
            .diagnostics
            .iter()
            .all(|diagnostic| suppressible(&diagnostic.code).is_some());
        let mut used = vec![false; self.rules.len()];
        for diagnostic in std::mem::take(&mut report.diagnostics) {
            match self.rules.iter().enumerate().find(|(_, rule)| {
                diagnostic.code == rule.code && covers(rule.target, diagnostic.span)
            }) {
                Some((index, rule)) => {
                    used[index] = true;
                    report.suppressed.push(SuppressedDiagnostic {
                        diagnostic,
                        reason: rule.reason.clone(),
                        directive: rule.directive,
                    });
                }
                None => report.diagnostics.push(diagnostic),
            }
        }
        if !fully_checked {
            return Vec::new();
        }
        self.rules
            .iter()
            .zip(used)
            .filter(|(_, used)| !used)
            .map(|(rule, _)| {
                Diagnostic::new(
                    "PNX9007",
                    rule.directive,
                    format!("Unused suppression for {}", rule.code),
                )
            })
            .collect()
    }
}

fn covers(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset
        && inner.start.offset < outer.end.offset
        && inner.end.offset <= outer.end.offset
}

fn locate_targets(
    form: &Form,
    targets: &mut BTreeMap<usize, Option<Span>>,
    quoted: &mut Vec<Span>,
) {
    if let Some(target) = targets.get_mut(&form.span.start.offset) {
        *target = Some(form.span);
    }
    let children = match &form.kind {
        Kind::List(forms) if forms.first().and_then(Form::symbol) == Some("quote") => {
            quoted.push(form.span);
            return;
        }
        Kind::List(forms) | Kind::Vector(forms) | Kind::Map(forms) | Kind::Set(forms) => {
            forms.as_slice()
        }
        Kind::AnonymousFunction(function) => &function.body,
        Kind::Metadata(_, target) => std::slice::from_ref(target.as_ref()),
        _ => return,
    };
    for child in children {
        locate_targets(child, targets, quoted);
    }
}
