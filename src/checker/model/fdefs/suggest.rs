//! Reviewable s/fdef edits from the current source, without executing it.

use std::collections::BTreeSet;
use std::fmt::Write;

use crate::checker::bindings::binding_names;
use crate::checker::function_forms::function_clauses;
use crate::project::{FdefSuggestion, SourceEdit};
use crate::reader::{Form, Kind};
use crate::types::Arity;

use super::super::{Declarations, Names, contracts::function_parts};

const SPEC: &str = "clojure.spec.alpha";
const UNKNOWN: &str = "clojure.core/any?";

impl Declarations {
    pub(crate) fn suggest_fdef<'a>(
        &'a self,
        forms: &'a [Form],
        source: &str,
        offset: usize,
    ) -> Option<FdefSuggestion> {
        let ns = forms.first()?;
        let Kind::List(ns_items) = &ns.without_metadata().kind else {
            return None;
        };
        if !matches!(ns_items.first()?.symbol(), Some("ns" | "clojure.core/ns"))
            || ns_items.get(1)?.symbol()? != self.namespace()
            || !self.all_refers.is_empty()
        {
            return None;
        }
        let index = forms.partition_point(|form| form.span.end.offset <= offset);
        let form = forms.get(index)?;
        if offset < form.span.start.offset || self.functions.binary_search(&index).is_err() {
            return None;
        }
        let Kind::List(items) = &form.without_metadata().kind else {
            return None;
        };
        let name = items.get(1)?.symbol().filter(|name| !name.contains('/'))?;
        self.source_var(forms, name, false)?;
        let (arms, attrs) = function_parts(&items[2..]);
        if [":t/sig", ":t/guard", ":dynamic", ":inline", ":macro"]
            .iter()
            .any(|key| {
                items[1].metadata(key).is_some()
                    || attrs.clone().any(|[k, _]| k.keyword() == Some(key))
            })
            || self.fdefs.iter().any(|&index| {
                let Kind::List(fdef) = &forms[index].without_metadata().kind else {
                    return false;
                };
                fdef.get(1).and_then(Form::symbol).is_some_and(|target| {
                    target == name || self.names.resolve_parts(target) == (self.namespace(), name)
                })
            })
        {
            return None;
        }

        let clauses = function_clauses(arms, form.span).ok()?;
        let (Arity::Fixed(_), clause) = clauses.first_key_value()? else {
            return None;
        };
        if clauses.len() != 1 {
            return None;
        }
        let params = clause.bindings();
        let mut locals = Vec::new();
        for binding in params.iter() {
            binding_names(binding, &mut locals);
        }
        let operation = |head: &'a Form| {
            let symbol = head.symbol()?;
            if !symbol.contains('/')
                && (locals.iter().any(|&(name, _)| name == symbol)
                    || self.source_declarations(forms, symbol).next().is_some()
                    || self.forward_vars.contains_key(symbol))
            {
                return None;
            }
            Some(self.names.resolve_parts(symbol))
        };
        let result = clause
            .body
            .last()
            .map_or(Some("nil?"), |tail| result_predicate(tail, &operation));
        let mut args = vec![None; params.len()];
        // A direct numeric operation on the sole parameter gives a candidate
        // input constraint. Observed call sites alone give no such constraint.
        if let [param] = params.as_ref()
            && param.metadata(":tag").is_none()
            && let [body] = clause.body
            && let Kind::List(call) = &body.without_metadata().kind
            && let [head, arg] = call.as_slice()
            && param
                .symbol()
                .is_some_and(|name| arg.symbol() == Some(name))
            && matches!(operation(head), Some(("clojure.core", "inc" | "dec")))
        {
            args[0] = Some("number?");
        }
        let draft = result.is_none() || args.iter().any(Option::is_none);

        // Keep a same-line comment on the definition and leave the next form alone.
        let end = form.span.end.offset;
        let line_end = source[end..]
            .find(['\r', '\n'])
            .map_or(source.len(), |i| end + i);
        let tail = source[end..line_end].trim();
        if !tail.is_empty() && !tail.starts_with(';') {
            return None;
        }
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let (qualifier, require) = spec_require(ns, ns_items, &self.names, newline);
        let mut edits = Vec::new();
        let mut new_text = format!("{newline}{newline}");
        let header = format!(";; Draft s/fdef for {name}:");
        if draft {
            if source[line_end..].trim_start().starts_with(&header) {
                return None;
            }
            write!(
                new_text,
                "{header} replace unknown predicates before enabling.{newline}"
            )
            .unwrap();
            if require.is_some() {
                write!(new_text, ";; Require {SPEC} before enabling.{newline}").unwrap();
            }
        } else if let Some(require) = require {
            edits.push(require);
        }
        let comment = if draft { ";; " } else { "" };
        write!(
            new_text,
            "{comment}({qualifier}/fdef {name}{newline}{comment}  :args ({qualifier}/cat"
        )
        .unwrap();
        let mut used = BTreeSet::new();
        for (index, (param, predicate)) in params.iter().zip(args).enumerate() {
            let mut tag = param
                .symbol()
                .filter(|name| {
                    name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                        && name
                            .bytes()
                            .all(|ch| ch.is_ascii_alphanumeric() || b"_-?!".contains(&ch))
                })
                .map_or_else(|| format!("arg{}", index + 1), str::to_owned);
            while !used.insert(tag.clone()) {
                tag.push('_');
            }
            write!(new_text, " :{tag} {}", qualified_predicate(predicate)).unwrap();
        }
        write!(
            new_text,
            "){newline}{comment}  :ret {})",
            qualified_predicate(result)
        )
        .unwrap();
        if line_end == source.len() {
            new_text.push_str(newline);
        }
        edits.push(SourceEdit {
            offset: line_end,
            new_text,
        });
        Some(FdefSuggestion { draft, edits })
    }
}

fn qualified_predicate(predicate: Option<&str>) -> String {
    predicate.map_or_else(|| UNKNOWN.into(), |name| format!("clojure.core/{name}"))
}

fn result_predicate<'a>(
    form: &'a Form,
    operation: &impl Fn(&'a Form) -> Option<(&'a str, &'a str)>,
) -> Option<&'static str> {
    match &form.without_metadata().kind {
        Kind::String(_) | Kind::Utf16String(_) => Some("string?"),
        Kind::Number | Kind::SymbolicNumber(_) => Some("number?"),
        Kind::Keyword(_) => Some("keyword?"),
        Kind::Character(_) => Some("char?"),
        Kind::Bool(_) => Some("boolean?"),
        Kind::Nil => Some("nil?"),
        Kind::Map(_) | Kind::AutoNamespacedMap { .. } => Some("map?"),
        Kind::List(items) => {
            let [head, args @ ..] = items.as_slice() else {
                return None;
            };
            if let (Some("if"), [_, yes, no]) = (head.symbol(), args) {
                let predicate = result_predicate(yes, operation)?;
                return (result_predicate(no, operation) == Some(predicate)).then_some(predicate);
            }
            match operation(head)? {
                ("clojure.core", "str") => Some("string?"),
                ("clojure.string", "trim" | "lower-case" | "upper-case") if args.len() == 1 => {
                    Some("string?")
                }
                ("clojure.core", "boolean" | "not" | "nil?" | "some?") if args.len() == 1 => {
                    Some("boolean?")
                }
                ("clojure.core", "inc" | "dec") if args.len() == 1 => Some("number?"),
                _ => None,
            }
        }
        _ => None,
    }
}

fn spec_require<'a>(
    ns: &Form,
    items: &'a [Form],
    names: &Names,
    newline: &str,
) -> (&'a str, Option<SourceEdit>) {
    let mut require = None;
    let mut loaded = false;
    for clause in &items[2..] {
        let Kind::List(forms) = &clause.without_metadata().kind else {
            continue;
        };
        if forms.first().and_then(Form::keyword) != Some(":require") {
            continue;
        }
        require.get_or_insert(clause);
        for lib in &forms[1..] {
            if lib.symbol() == Some(SPEC) {
                loaded = true;
            }
            if let Some([head, options @ ..]) = lib.vector()
                && head.symbol() == Some(SPEC)
            {
                let (pairs, _) = options.as_chunks::<2>();
                let alias = pairs.iter().find_map(|[key, value]| {
                    (key.keyword() == Some(":as"))
                        .then_some(value.symbol())
                        .flatten()
                });
                if let Some(alias) = alias
                    && names.resolve_parts(&format!("{alias}/fdef")).0 == SPEC
                {
                    return (alias, None);
                }
                loaded |= !options
                    .iter()
                    .any(|option| option.keyword() == Some(":as-alias"));
            }
        }
    }
    if loaded {
        return (SPEC, None);
    }
    let edit = match require {
        Some(clause) => SourceEdit {
            offset: clause.without_metadata().span.end.offset - 1,
            new_text: format!("{newline}    [{SPEC}]"),
        },
        None => SourceEdit {
            offset: ns.without_metadata().span.end.offset - 1,
            new_text: format!("{newline}  (:require [{SPEC}])"),
        },
    };
    (SPEC, Some(edit))
}
