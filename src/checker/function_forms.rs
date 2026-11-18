//! Shared parsing of function clauses.
use std::borrow::Cow;
use std::collections::BTreeMap;

use super::Value;
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::{Arity, MAX_TUPLE_ITEMS, Type};

pub(super) const MAX_FIXED_PARAMETERS: usize = 20;

pub(super) struct Clause<'a> {
    params: &'a [Form],
    rest: Option<&'a Form>,
    pub body: &'a [Form],
    pub span: Span,
}

impl<'a> Clause<'a> {
    fn arity(&self) -> Arity {
        Arity::new(self.params.len(), self.rest.is_some())
    }

    pub fn bindings(&self) -> Cow<'a, [Form]> {
        if let Some(rest) = self.rest {
            Cow::Owned(self.params.iter().chain([rest]).cloned().collect())
        } else {
            Cow::Borrowed(self.params)
        }
    }
}

pub(super) fn function_clauses(
    forms: &[Form],
    span: Span,
) -> Result<BTreeMap<Arity, Clause<'_>>, Box<Diagnostic>> {
    if forms.first().and_then(Form::vector).is_some() {
        let clause = function_clause(forms, span)?;
        return Ok(BTreeMap::from([(clause.arity(), clause)]));
    }
    let mut clauses = BTreeMap::new();
    for arm in forms {
        let Kind::List(forms) = &arm.without_metadata().kind else {
            return Err(error(
                "PNX1001",
                arm.span,
                "Expected a function arity declaration",
            ));
        };
        let clause = function_clause(forms, arm.span)?;
        let count = clause.arity();
        if clauses.insert(count, clause).is_some() {
            return Err(error(
                "PNX2554",
                arm.span,
                format!("Duplicate function arity {count}"),
            ));
        }
    }
    if clauses.is_empty() {
        return Err(error(
            "PNX1001",
            span,
            "Expected a parameter vector or arity declarations",
        ));
    }
    if !Arity::valid_set(clauses.keys().copied()) {
        return Err(error(
            "PNX2554",
            span,
            "A variadic clause must be unique and require at least as many arguments as every fixed clause",
        ));
    }
    Ok(clauses)
}

pub(super) fn split_rest(forms: &[Form]) -> Result<(&[Form], Option<&Form>), Span> {
    let Some(index) = forms.iter().position(|form| form.symbol() == Some("&")) else {
        return Ok((forms, None));
    };
    let [rest] = &forms[index + 1..] else {
        return Err(forms[index].span);
    };
    if rest.symbol() == Some("&") {
        return Err(rest.span);
    }
    Ok((&forms[..index], Some(rest)))
}

pub(super) fn pack_arguments(arity: Arity, mut values: Vec<Value>) -> Vec<Value> {
    if let Arity::Variadic(count) = arity {
        let rest: Vec<_> = values.drain(count..).map(|value| value.known).collect();
        let ty = if rest.contains(&Type::Never) {
            Type::Never
        } else if rest.is_empty() {
            Type::Nil
        } else if rest.len() <= MAX_TUPLE_ITEMS {
            Type::SeqTuple(rest.into_boxed_slice())
        } else {
            Type::Seq(Box::new(Type::union(rest)))
        };
        values.push(Value::new(ty));
    }
    values
}

fn function_clause(forms: &[Form], span: Span) -> Result<Clause<'_>, Box<Diagnostic>> {
    let [parameters, body @ ..] = forms else {
        return Err(error(
            "PNX1001",
            span,
            "Expected a function parameter vector",
        ));
    };
    let Some(params) = parameters.vector() else {
        return Err(error(
            "PNX1001",
            parameters.span,
            "Expected a function parameter vector",
        ));
    };
    let (params, rest) = split_rest(params)
        .map_err(|span| error("PNX1001", span, "& requires exactly one rest binding"))?;
    if let Some(rest) = rest
        && rest.map().is_some()
    {
        return Err(Box::new(Diagnostic::analysis_gap(
            "PNX9001",
            rest.span,
            "Map destructuring of variadic arguments is not supported yet",
        )));
    }
    for param in params.iter().chain(rest) {
        match &param.without_metadata().kind {
            Kind::Symbol(name) | Kind::AnonymousArgument(name)
                if !name.contains('/') || name == "/" => {}
            Kind::Map(_) | Kind::Vector(_) => {}
            _ => {
                return Err(error(
                    "PNX1001",
                    param.span,
                    "Invalid function parameter binding",
                ));
            }
        }
    }
    if params.len() > MAX_FIXED_PARAMETERS {
        return Err(error(
            "PNX2554",
            parameters.span,
            "Clojure/JVM supports at most 20 fixed parameters",
        ));
    }
    let (condition_span, body) = if let [head, rest @ ..] = body
        && !rest.is_empty()
        && let Some(entries) = head.map()
    {
        let span = entries
            .as_chunks::<2>()
            .0
            .iter()
            .any(|[key, _]| matches!(key.keyword(), Some(":pre" | ":post")))
            .then_some(head.span);
        (span, rest)
    } else {
        let span = [":pre", ":post"]
            .iter()
            .find_map(|key| parameters.metadata(key).map(|form| form.span));
        (span, body)
    };
    if let Some(span) = condition_span {
        return Err(Box::new(Diagnostic::analysis_gap(
            "PNX9001",
            span,
            "Function preconditions and postconditions are not supported yet",
        )));
    }
    Ok(Clause {
        params,
        rest,
        body,
        span,
    })
}

fn error(code: &str, span: Span, message: impl Into<String>) -> Box<Diagnostic> {
    Box::new(Diagnostic::new(code, span, message))
}
