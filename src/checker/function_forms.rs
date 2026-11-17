//! Shared parsing of fixed-arity function clauses.
use std::collections::BTreeMap;

use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};

pub(super) struct Clause<'a> {
    pub params: &'a [Form],
    pub body: &'a [Form],
    pub span: Span,
}

pub(super) fn fixed_clauses(
    forms: &[Form],
    span: Span,
) -> Result<BTreeMap<usize, Clause<'_>>, Box<Diagnostic>> {
    if forms.first().and_then(Form::vector).is_some() {
        let clause = fixed_clause(forms, span)?;
        return Ok(BTreeMap::from([(clause.params.len(), clause)]));
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
        let clause = fixed_clause(forms, arm.span)?;
        let count = clause.params.len();
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
    Ok(clauses)
}

fn fixed_clause(forms: &[Form], span: Span) -> Result<Clause<'_>, Box<Diagnostic>> {
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
    for param in params {
        match &param.without_metadata().kind {
            Kind::Symbol(name) if name == "&" => {
                return Err(error(
                    "PNX9001",
                    param.span,
                    "Variadic functions are not supported yet",
                ));
            }
            Kind::Symbol(name) if !name.contains('/') || name == "/" => {}
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
    if params.len() > 20 {
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
        return Err(error(
            "PNX9001",
            span,
            "Function preconditions and postconditions are not supported yet",
        ));
    }
    Ok(Clause { params, body, span })
}

fn error(code: &str, span: Span, message: impl Into<String>) -> Box<Diagnostic> {
    Box::new(Diagnostic::new(code, span, message))
}
