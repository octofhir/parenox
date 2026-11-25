//! Shared parsing of function clauses.
use std::borrow::Cow;
use std::collections::BTreeMap;

use super::Value;
use super::bindings::{dotted_pattern_error, raw_local_symbol};
use super::jvm_primitives::MAX_NATIVE_ARGS;
use super::metadata::java_type_hint;
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::{Arity, MAX_TUPLE_ITEMS, Type};

pub(super) const MAX_FIXED_PARAMETERS: usize = 20;

pub(super) fn letfn_bindings(form: &Form) -> Option<&[Form]> {
    match &form.without_metadata().kind {
        Kind::List(forms) | Kind::Vector(forms) => Some(forms),
        Kind::Nil => Some(&[]),
        Kind::Map(forms) | Kind::Set(forms) if forms.is_empty() => Some(&[]),
        Kind::String(text) if text.is_empty() => Some(&[]),
        _ => None,
    }
}

pub(super) fn letfn_definition(form: &Form) -> Option<&[Form]> {
    match &form.without_metadata().kind {
        Kind::List(forms) | Kind::Vector(forms) => Some(forms),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(super) struct FunctionConditions<'a> {
    pub pre: &'a [Form],
    pub post: Option<&'a [Form]>,
}

pub(super) struct Clause<'a> {
    pub parameter_vector: &'a Form,
    params: &'a [Form],
    rest: Option<&'a Form>,
    pub conditions: FunctionConditions<'a>,
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

    pub fn parameters(&self) -> impl Iterator<Item = &'a Form> {
        self.params.iter().chain(self.rest)
    }
}

pub(super) fn function_clauses(
    forms: &[Form],
    span: Span,
) -> Result<BTreeMap<Arity, Clause<'_>>, Box<Diagnostic>> {
    read_function_clauses(forms, span, ClauseMode::Checked)
}

pub(super) fn lexical_function_clauses(
    forms: &[Form],
    span: Span,
) -> Result<BTreeMap<Arity, Clause<'_>>, Box<Diagnostic>> {
    read_function_clauses(forms, span, ClauseMode::Lexical)
}

pub(super) fn lexical_fn_star_clauses(
    forms: &[Form],
    span: Span,
) -> Result<BTreeMap<Arity, Clause<'_>>, Box<Diagnostic>> {
    read_function_clauses(forms, span, ClauseMode::LexicalFnStar)
}

pub(super) fn fn_star_clauses(
    forms: &[Form],
    span: Span,
) -> Result<BTreeMap<Arity, Clause<'_>>, Box<Diagnostic>> {
    read_function_clauses(forms, span, ClauseMode::CheckedFnStar)
}

pub(super) fn local_fn_name(name: &str, raw: bool) -> &str {
    if raw && name != "/" {
        return name.split_once('/').map_or(name, |(_, local)| local);
    }
    name
}

pub(super) fn split_method_name(tail: &[Form]) -> (Option<&Form>, &[Form]) {
    match tail.split_first() {
        Some((name, arms)) if raw_local_symbol(name).is_some() => (Some(name), arms),
        _ => (None, tail),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ClauseMode {
    Checked,
    Lexical,
    CheckedFnStar,
    LexicalFnStar,
}

impl ClauseMode {
    fn raw(self) -> bool {
        matches!(self, Self::CheckedFnStar | Self::LexicalFnStar)
    }

    fn checked(self) -> bool {
        matches!(self, Self::Checked | Self::CheckedFnStar)
    }
}

fn read_function_clauses(
    forms: &[Form],
    span: Span,
    mode: ClauseMode,
) -> Result<BTreeMap<Arity, Clause<'_>>, Box<Diagnostic>> {
    if forms.first().and_then(Form::vector).is_some() {
        let clause = function_clause(forms, span, mode)?;
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
        let clause = function_clause(forms, arm.span, mode)?;
        let count = clause.arity();
        if clauses.insert(count, clause).is_some() {
            return Err(error(
                "PNX2554",
                arm.span,
                format!("Duplicate function arity {count}"),
            ));
        }
    }
    if clauses.is_empty() && !mode.raw() {
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
        let rest = values.split_off(count);
        let rest = if rest.iter().any(Value::stops_evaluation) {
            Value::new(Type::Never)
        } else if rest.is_empty() {
            Value::new(Type::Nil)
        } else if rest.len() <= MAX_TUPLE_ITEMS {
            Value::positional_sequence(rest)
        } else {
            Value::new(Type::Seq(Box::new(Type::union(
                rest.into_iter().map(|value| value.known),
            ))))
        };
        values.push(rest);
    }
    values
}

pub(super) fn function_clause(
    forms: &[Form],
    span: Span,
    mode: ClauseMode,
) -> Result<Clause<'_>, Box<Diagnostic>> {
    let check_types = mode.checked();
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
    if check_types {
        validate_function_hints(parameters, params, rest)?;
    }
    for param in params.iter().chain(rest) {
        if mode.raw() && raw_local_symbol(param).is_none() {
            return Err(error(
                "PNX1001",
                param.without_metadata().span,
                "fn* requires an unqualified symbol for each parameter",
            ));
        }
        if check_types && let Some(diagnostic) = dotted_pattern_error(param) {
            return Err(Box::new(diagnostic));
        }
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
    let (conditions, body) = if !mode.raw()
        && let [head, rest @ ..] = body
        && !rest.is_empty()
        && let Some(entries) = head.map()
    {
        (Some(entries), rest)
    } else {
        (None, body)
    };
    let condition = |name| {
        if mode.raw() {
            return None;
        }
        conditions
            .map_or_else(
                || parameters.metadata(name),
                |entries| {
                    entries
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .find_map(|[key, value]| (key.keyword() == Some(name)).then_some(value))
                },
            )
            .filter(|form| !matches!(form.kind, Kind::Nil | Kind::Bool(false)))
    };
    let conditions = |key, description| match condition(key) {
        Some(Form {
            kind: Kind::Vector(items),
            ..
        }) => Ok(Some(items.as_slice())),
        Some(form) if check_types => Err(Box::new(Diagnostic::analysis_gap(
            "PNX9001",
            form.span,
            format!("Function {description} checking requires a vector of expressions"),
        ))),
        _ => Ok(None),
    };
    let pre = conditions(":pre", "precondition")?.unwrap_or_default();
    let post = conditions(":post", "postcondition")?;
    Ok(Clause {
        parameter_vector: parameters,
        params,
        rest,
        conditions: FunctionConditions { pre, post },
        body,
        span,
    })
}

fn validate_function_hints(
    parameters: &Form,
    params: &[Form],
    rest: Option<&Form>,
) -> Result<(), Box<Diagnostic>> {
    if let Some(binding) = rest
        && binding.symbol().is_some()
        && java_type_hint(binding).is_some()
    {
        return Err(error(
            "PNX1001",
            binding.metadata(":tag").unwrap().span,
            "& arg cannot have type hint",
        ));
    }
    let mut primitive = None;
    for form in
        std::iter::once(parameters).chain(params.iter().filter(|form| form.symbol().is_some()))
    {
        match java_type_hint(form).and_then(|hint| hint.name()) {
            Some("long" | "double") => {
                primitive.get_or_insert_with(|| form.metadata(":tag").unwrap().span);
            }
            Some("boolean" | "byte" | "char" | "short" | "int" | "float" | "void") => {
                return Err(error(
                    "PNX1001",
                    form.metadata(":tag").unwrap().span,
                    "Only long and double primitives are supported in JVM functions",
                ));
            }
            _ => {}
        }
    }
    if let Some(span) = primitive {
        if rest.is_some() {
            return Err(error(
                "PNX1001",
                span,
                "fns taking primitives cannot be variadic",
            ));
        }
        if params.len() > MAX_NATIVE_ARGS {
            return Err(error(
                "PNX1001",
                span,
                format!("fns taking primitives support only {MAX_NATIVE_ARGS} or fewer args"),
            ));
        }
    }
    Ok(())
}

fn error(code: &str, span: Span, message: impl Into<String>) -> Box<Diagnostic> {
    Box::new(Diagnostic::new(code, span, message))
}
