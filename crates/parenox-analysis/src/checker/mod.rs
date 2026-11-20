//! Source analysis for the supported Clojure subset.
mod bindings;
mod calls;
mod case;
mod env;
mod exceptions;
mod flow;
mod function_forms;
mod maps;
mod model;
mod queries;
mod quote;
mod recur;

use env::Env;
use exceptions::Throws;
use model::{Contract, DeclaredValue, Function, VerifiedGuard};
pub(crate) use model::{
    Declarations, DocumentReferences, EncodeError, ExternCache, Globals, InterfaceKey, Model,
    ProjectCall, ScopedReference, SnapshotWriter, SpecCache, SpecConversion, SpecFingerprint,
    SpecSource, java_type_tag_at,
};
pub(crate) use queries::CheckedCall;
use queries::TypeQuery;
use recur::RecurTarget;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::reader::{Form, Kind, Span};
use crate::types::{
    Arity, Compatibility, IntegerRange, IntegerType, MAX_TUPLE_ITEMS, Signature, Type,
};
use crate::{ContractOrigin, Diagnostic, Suggestion};

pub(crate) struct FunctionCheck {
    pub diagnostics: Vec<Diagnostic>,
    pub checked_helpers: BTreeSet<String>,
    pub declaration_findings_sensitive: bool,
    specs: BTreeMap<String, SpecInput>,
}

struct SpecInput {
    ty: Option<Type>,
    conversion: Option<SpecConversion>,
    provider: Option<Arc<model::SpecProvider>>,
}

impl FunctionCheck {
    pub fn matches_specs(&self, globals: &Globals) -> bool {
        self.specs.iter().all(|(key, previous)| {
            globals.spec(key) == previous.ty.as_ref()
                && globals.spec_conversion(key) == previous.conversion
                && globals.spec_provider(key) == previous.provider.as_ref()
        })
    }
}

impl Model {
    pub fn check_function(
        &self,
        function: &Function,
        source: &str,
        globals: &Globals,
        has_declaration_findings: bool,
    ) -> FunctionCheck {
        let mut checker = Checker::new(self, globals, source);
        checker.has_declaration_findings = has_declaration_findings;
        let contract = function
            .contract
            .as_ref()
            .expect("checked functions have contracts");
        for signature in contract.signatures.iter() {
            let arity = &function.arities[&signature.arity()];
            let result = checker.declared_function_body(function, signature);
            checker.expect_contract(
                &result,
                &signature.ret,
                arity.body.last().map_or(arity.span, |form| form.span),
                &format!("Return value of {}", function.name),
                Some(&contract.origin),
            );
        }
        FunctionCheck {
            diagnostics: checker.diagnostics,
            checked_helpers: checker
                .specializations
                .into_keys()
                .filter_map(|(name, _, reachable)| reachable.then_some(name))
                .collect(),
            declaration_findings_sensitive: checker.has_unproven_expectation,
            specs: checker.spec_inputs,
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum LiteralContainer {
    Map(Option<Arc<BTreeMap<Type, LiteralContainer>>>),
    Vector(Option<Arc<[Option<LiteralContainer>]>>),
}

impl LiteralContainer {
    fn map_field(&self, key: &Type) -> Option<&Self> {
        let Self::Map(Some(fields)) = self else {
            return None;
        };
        fields.get(key)
    }

    fn vector_item(&self, index: usize) -> Option<&Self> {
        let Self::Vector(Some(items)) = self else {
            return None;
        };
        items.get(index).and_then(Option::as_ref)
    }

    fn matches_type(&self, ty: &Type) -> bool {
        match (self, ty) {
            (Self::Map(_), Type::Map(_)) => true,
            (Self::Vector(items), Type::Tuple(types)) => items
                .as_ref()
                .is_none_or(|items| items.len() == types.len()),
            _ => false,
        }
    }

    fn join(self, other: Self, known: &Type) -> Option<Self> {
        match (self, other) {
            (Self::Map(left), Self::Map(right)) => {
                let fields = match (left, right) {
                    (Some(left), Some(right)) if left == right => Some(left),
                    (Some(left), Some(right)) => {
                        let shared: BTreeMap<_, _> = left
                            .iter()
                            .filter(|(key, child)| right.get(*key) == Some(*child))
                            .map(|(key, child)| (key.clone(), child.clone()))
                            .collect();
                        (!shared.is_empty()).then(|| Arc::new(shared))
                    }
                    _ => None,
                };
                Some(Self::Map(fields))
            }
            (left @ Self::Vector(_), right) if left == right && left.matches_type(known) => {
                Some(left)
            }
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Value {
    // Inferred language type for ordinary use and diagnostics, e.g. String or Set<String>.
    ty: Type,
    // More precise static knowledge, e.g. a string literal or finite union of values.
    known: Type,
    notes: Vec<String>,
    contract: Option<Arc<ContractOrigin>>,
    guard: Option<Arc<VerifiedGuard>>,
    // Structural types do not prove ordinary JVM collection behavior.
    literal_container: Option<LiteralContainer>,
}

impl Value {
    fn new(known: Type) -> Self {
        Self {
            ty: known.inferred(),
            known,
            notes: Vec::new(),
            contract: None,
            guard: None,
            literal_container: None,
        }
    }

    fn with_notes(known: Type, notes: Vec<String>) -> Self {
        Self {
            notes,
            ..Self::new(known)
        }
    }

    fn is_literal_map(&self) -> bool {
        matches!(self.literal_container, Some(LiteralContainer::Map(_)))
    }

    fn join(mut self, other: Value) -> Value {
        if self.known == Type::Never {
            return other;
        }
        if other.known == Type::Never {
            return self;
        }
        for note in other.notes {
            if !self.notes.contains(&note) {
                self.notes.push(note);
            }
        }
        let contract = if self.contract == other.contract {
            self.contract
        } else {
            None
        };
        let guard = if self.guard == other.guard {
            self.guard
        } else {
            None
        };
        let known = self.known.join_values(other.known);
        let literal_container = self
            .literal_container
            .zip(other.literal_container)
            .and_then(|(left, right)| left.join(right, &known));
        Self {
            contract,
            guard,
            literal_container,
            ..Self::with_notes(known, self.notes)
        }
    }

    fn narrow(&mut self, known: Type) {
        let keep = match &self.literal_container {
            Some(LiteralContainer::Map(_)) => matches!(known, Type::Map(_)),
            Some(LiteralContainer::Vector(_)) => {
                matches!((&self.known, &known), (Type::Tuple(before), Type::Tuple(after)) if before.len() == after.len())
            }
            None => false,
        };
        if !keep {
            self.literal_container = None;
        }
        self.ty = known.inferred();
        self.known = known;
    }

    fn same_value_facts(&self, other: &Self) -> bool {
        self.known == other.known && self.literal_container == other.literal_container
    }
}

impl Value {
    fn from_contract(contract: &Contract) -> Self {
        Self {
            contract: Some(contract.origin.clone()),
            guard: contract.guard.clone(),
            ..Self::new(Type::Function(contract.signatures.clone()))
        }
    }
}

struct Checker<'a> {
    model: &'a Model,
    globals: &'a Globals,
    source: &'a str,
    partial_scope: Option<&'a BTreeSet<&'a str>>,
    type_query: Option<TypeQuery<'a>>,
    diagnostics: Vec<Diagnostic>,
    has_declaration_findings: bool,
    has_unproven_expectation: bool,
    spec_inputs: BTreeMap<String, SpecInput>,
    active_helpers: Vec<String>,
    thread_depth: usize,
    path_reachable: bool,
    specializations: BTreeMap<(String, Vec<Value>, bool), calls::Specialization>,
    recur_target: Option<RecurTarget>,
    recur_checks: usize,
    reduction_steps: usize,
    map_callback_checks: usize,
    callback_depth: usize,
    macro_scope_depth: usize,
    throws: Option<Throws>,
}

impl<'a> Checker<'a> {
    fn new(model: &'a Model, globals: &'a Globals, source: &'a str) -> Self {
        Self {
            model,
            globals,
            source,
            partial_scope: None,
            type_query: None,
            diagnostics: Vec::new(),
            has_declaration_findings: !model.diagnostics.is_empty(),
            has_unproven_expectation: false,
            spec_inputs: BTreeMap::new(),
            active_helpers: Vec::new(),
            thread_depth: 0,
            path_reachable: true,
            specializations: BTreeMap::new(),
            recur_target: None,
            recur_checks: 0,
            reduction_steps: 0,
            map_callback_checks: 0,
            callback_depth: 0,
            macro_scope_depth: 0,
            throws: None,
        }
    }
}

impl Checker<'_> {
    fn declared_function_body(&mut self, function: &Function, signature: &Signature) -> Value {
        let arity = &function.arities[&signature.arity()];
        let mut args: Vec<_> = signature.args.iter().cloned().map(Value::new).collect();
        if let Some(rest) = &signature.rest {
            let seq = Type::Seq(Box::new(rest.clone()));
            // At the required count Clojure invokes a matching fixed clause first.
            let rest = if function
                .arities
                .contains_key(&Arity::Fixed(signature.args.len()))
            {
                seq
            } else {
                seq.nilable()
            };
            args.push(Value::new(rest));
        }
        self.function_body(
            &arity.params,
            arity.conditions(),
            &arity.body,
            args,
            &Env::new(),
            arity.span,
        )
    }

    fn read_spec(&mut self, key: &str) -> (Option<&Type>, Option<SpecConversion>) {
        let ty = self.globals.spec(key);
        let conversion = self.globals.spec_conversion(key);
        self.spec_inputs
            .entry(key.into())
            .or_insert_with(|| SpecInput {
                ty: ty.cloned(),
                conversion: conversion.clone(),
                provider: self.globals.spec_provider(key).cloned(),
            });
        (ty, conversion)
    }

    fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    fn dedup_diagnostics(&mut self, start: usize) {
        if self.diagnostics.len() - start < 2 {
            return;
        }
        let mut reported = BTreeSet::new();
        let diagnostics = self.diagnostics.split_off(start);
        self.diagnostics
            .extend(diagnostics.into_iter().filter(|diagnostic| {
                reported.insert((
                    diagnostic.span.start.offset,
                    diagnostic.span.end.offset,
                    diagnostic.code.clone(),
                    diagnostic.message.clone(),
                    diagnostic.contract.clone(),
                ))
            }));
    }

    fn unsupported(&mut self, span: Span, message: impl Into<String>) -> Value {
        self.may_throw();
        self.error("PNX9001", span, message);
        Value::new(Type::Unknown)
    }

    fn unchecked_form(&mut self, span: Span, message: &str) -> Value {
        self.may_throw();
        self.diagnostics
            .push(Diagnostic::analysis_gap("PNX9001", span, message));
        Value::new(Type::Unknown)
    }

    fn expect(&mut self, actual: &Value, expected: &Type, span: Span, context: &str) {
        if self.partial_scope.is_some() && actual.known.contains_unknown() {
            return;
        }
        let compatibility = actual.known.compatibility(expected);
        if compatibility == Compatibility::Unproven {
            self.unproven_expectation(actual, expected, span, context);
            return;
        }
        if compatibility == Compatibility::Mismatch {
            let reported_type = if expected.requires_precise_type() {
                &actual.known
            } else {
                &actual.ty
            };
            let mut diagnostic = Diagnostic::type_mismatch(
                span,
                context,
                expected.to_string(),
                reported_type.to_string(),
            );
            diagnostic.notes = actual.notes.clone();
            let expression = &self.source[span.start.offset..span.end.offset];
            if *expected == Type::String
                && actual.ty.without_nil() != actual.ty
                && actual.ty.without_nil() != Type::Never
                && actual.ty.without_nil().subtype_of(expected)
            {
                diagnostic.suggestions.push(self.nil_string_fallback(span));
            } else if *expected == Type::Number
                && expression.starts_with('"')
                && let Type::Str(value) = &actual.known
                && value
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '+' || c == '-')
                && value.parse::<f64>().is_ok()
                && crate::reader::read(value)
                    .is_ok_and(|forms| forms.len() == 1 && matches!(forms[0].kind, Kind::Number))
            {
                diagnostic.suggestions.push(Suggestion {
                    message: "use a number literal if this value is intended to be numeric".into(),
                    span,
                    replacement: value.clone(),
                    applicability: "requires-review",
                });
            } else {
                diagnostic.help = Some(if context.starts_with("Return value of ") {
                    format!("Return a value of type {expected} to satisfy the declared signature.")
                } else {
                    format!(
                        "Provide a value of type {expected}; validate or convert the input before using it here."
                    )
                });
            }
            self.diagnostics.push(diagnostic);
        }
    }

    fn unproven_expectation(
        &mut self,
        actual: &Value,
        expected: impl std::fmt::Display,
        span: Span,
        context: &str,
    ) {
        // Unknown types often follow a finding already reported for the
        // function or its declarations.
        if !self.path_reachable {
            return;
        }
        self.has_unproven_expectation = true;
        if !self.diagnostics.is_empty() {
            return;
        }
        if !self.has_declaration_findings {
            self.error(
                "PNX9004",
                span,
                format!("{context}: cannot verify {} against {expected}", actual.ty),
            );
        }
    }

    fn nil_string_fallback(&self, span: Span) -> Suggestion {
        let expression = &self.source[span.start.offset..span.end.offset];
        Suggestion {
            message: "handle Nil explicitly; choose a fallback for your application".into(),
            span,
            replacement: format!("(or {expression} \"\")"),
            applicability: "requires-review",
        }
    }

    fn expect_contract(
        &mut self,
        actual: &Value,
        expected: &Type,
        span: Span,
        context: &str,
        contract: Option<&ContractOrigin>,
    ) {
        let start = self.diagnostics.len();
        self.expect(actual, expected, span, context);
        if let Some(contract) = contract
            && let Some(diagnostic) = self.diagnostics.get_mut(start)
        {
            diagnostic.contract = Some(contract.clone());
        }
    }

    fn body(&mut self, body: &[Form], env: &Env, tail: bool) -> Value {
        let mut value = Value::new(Type::Nil);
        for (index, form) in body.iter().enumerate() {
            let tail = tail && index + 1 == body.len();
            value = self.infer_path(form, env, value.known != Type::Never, tail);
        }
        value
    }

    fn infer(&mut self, form: &Form, env: &Env) -> Value {
        self.infer_position(form, env, false)
    }

    fn infer_set_members(&mut self, forms: &[Form], env: &Env) -> Vec<Type> {
        forms
            .iter()
            .map(|form| self.infer(form, env).known)
            .collect()
    }

    fn eval_args<'a>(
        &'a mut self,
        forms: &'a [Form],
        env: &'a Env,
        mut reachable: bool,
    ) -> impl Iterator<Item = (Value, Span)> + 'a {
        forms.iter().map(move |form| {
            let value = self.infer_path(form, env, reachable, false);
            reachable = value.known != Type::Never;
            (value, form.span)
        })
    }

    fn infer_position(&mut self, form: &Form, env: &Env, tail: bool) -> Value {
        match &form.kind {
            Kind::Nil => Value::new(Type::Nil),
            Kind::Bool(b) => Value::new(Type::Bool(*b)),
            Kind::Number => {
                let token = &self.source[form.span.start.offset..form.span.end.offset];
                Value::new(Type::from_numeric_token(token))
            }
            Kind::SymbolicNumber(_) => Value::new(Type::NonIntegral),
            Kind::String(s) => Value::new(Type::Str(s.clone())),
            Kind::Utf16String(_) => Value::new(Type::String),
            Kind::Regex => Value::new(Type::Pattern),
            Kind::Character(c) => Value::new(Type::Char(*c)),
            Kind::Metadata(_, target) => {
                let mut value = self.infer_position(target, env, tail);
                value.literal_container = None;
                value
            }
            Kind::Conditional { .. } => self.unsupported(form.span, "Reader conditional must be selected before type analysis"),
            Kind::Tagged(tag, _) => self.unsupported(form.span, format!(
                "Tagged literal #{} has no declared reader semantics; its reader function was not executed",
                tag.symbol().unwrap())),
            Kind::Keyword(k) => Value::new(Type::Key(self.model.key(k))),
            Kind::Symbol(s) | Kind::AnonymousArgument(s) => {
                if let Some(value) = env.get(s) {
                    self.observe_local(s, form.span, value);
                    value.clone()
                } else if self.model.local_function(s).is_some_and(Function::inferred) {
                    self.error(
                        "PNX9002",
                        form.span,
                        format!(
                            "Private function {s} needs a :t/sig annotation when used as a value"
                        ),
                    );
                    Value::new(Type::Unknown)
                } else if let Some(value) = self.model.declared_value(s, self.globals) {
                    match value {
                        DeclaredValue::Literal(ty) => Value::new(ty.clone()),
                        DeclaredValue::ExternLiteral(ty, origin) if self.partial_contract_visible(origin) => Value::with_notes(
                            ty.clone(),
                            origin.path.iter().map(|path| {
                                format!("Literal {} is declared in {path}.", origin.name)
                            }).collect(),
                        ),
                        DeclaredValue::ExternLiteral(_, _) => Value::new(Type::Unknown),
                        DeclaredValue::Function(contract) if self.partial_contract_visible(&contract.origin) => Value::from_contract(contract),
                        DeclaredValue::Function(_) => Value::new(Type::Unknown),
                        DeclaredValue::UncheckedFunction(_) => self.unchecked_function(s, form.span),
                        DeclaredValue::Macro(scope) => self.macro_gap(
                            scope, form.span, format!("Macro {} is not a function value", scope.origin().name)
                        ),
                    }
                } else if self.partial_scope.is_some() {
                    Value::new(Type::Unknown)
                } else {
                    let qualified = self.model.function_name(s);
                    let external = qualified.split_once('/').is_some_and(|(namespace, _)| {
                        namespace != self.model.namespace()
                            && !self.globals.project_namespaces.contains(namespace)
                    });
                    self.error(
                        if external { "PNX9003" } else { "PNX2304" },
                        form.span,
                        if external {
                            format!("No declaration for external value {s}")
                        } else {
                            format!("Unresolved or private value {s}")
                        },
                    );
                    Value::new(Type::Unknown)
                }
            }
            Kind::Vector(xs) => {
                let prove_origin = xs.len() <= MAX_TUPLE_ITEMS
                    && !xs.iter().any(has_evaluated_metadata);
                literal_vector_value(
                    self.eval_args(xs, env, true).map(|(value, _)| value),
                    xs.len(),
                    prove_origin,
                )
            }
            Kind::Set(xs) => {
                let types = self.infer_set_members(xs, env);
                if types.contains(&Type::Never) {
                    return Value::new(Type::Never);
                }
                Value::new(Type::Set(collection_element(types)))
            }
            Kind::Map(xs) => self.map_literal(xs, maps::MapLiteral::Expression(env)),
            Kind::AutoNamespacedMap { .. } => self.unchecked_form(form.span, "Namespaced map needs source namespace context"),
            Kind::List(xs) if xs.is_empty() => Value::new(Type::empty_list()),
            Kind::List(xs) => self.list(xs, form.span, env, tail),
            Kind::AnonymousFunction(_) => self.unsupported(
                form.span,
                "#() is currently supported in direct calls and inline callbacks",
            ),
            Kind::Quote(inner) => self.quoted_data(inner),
            Kind::SyntaxQuote(_) | Kind::Unquote { .. } => self.unchecked_form(
                form.span,
                "Syntax quote and unquote expansion is not supported",
            ),
        }
    }
}

fn vector_type(items: Vec<Type>) -> Type {
    if items.len() <= MAX_TUPLE_ITEMS {
        Type::Tuple(items.into_boxed_slice())
    } else {
        Type::Vector(collection_element(items))
    }
}

fn literal_vector_value(
    items: impl Iterator<Item = Value>,
    len: usize,
    prove_origin: bool,
) -> Value {
    let prove_origin = prove_origin && len <= MAX_TUPLE_ITEMS;
    let mut types = Vec::with_capacity(len);
    let mut container_items = None;
    for value in items {
        if prove_origin && let Some(container) = value.literal_container {
            let proofs = container_items.get_or_insert_with(|| vec![None; len]);
            proofs[types.len()] = Some(container);
        }
        types.push(value.known);
    }
    if types.last() == Some(&Type::Never) {
        return Value::new(Type::Never);
    }
    let mut value = Value::new(vector_type(types));
    if prove_origin {
        value.literal_container = Some(LiteralContainer::Vector(container_items.map(Arc::from)));
    }
    value
}

fn has_evaluated_metadata(form: &Form) -> bool {
    match &form.kind {
        Kind::Metadata(..) => true,
        Kind::List(items) if items.first().and_then(Form::symbol) == Some("quote") => false,
        Kind::List(items) | Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => {
            items.iter().any(has_evaluated_metadata)
        }
        Kind::AnonymousFunction(function) => function.body.iter().any(has_evaluated_metadata),
        _ => false,
    }
}

fn collection_element(types: Vec<Type>) -> Box<Type> {
    Box::new(
        Type::union(
            types.into_iter().map(|ty| match ty {
                Type::Integer(_)
                | Type::Integral(IntegerType {
                    range:
                        IntegerRange::ExactLong(_)
                        | IntegerRange::OutsideInt32
                        | IntegerRange::OutsideInt64,
                    ..
                })
                | Type::NonIntegral => Type::Number,
                Type::Integral(integer) => Type::Integral(integer.without_value()),
                ty => ty,
            }),
        ),
    )
}

fn nil_part(ty: &Type) -> Type {
    match ty {
        Type::Nil | Type::Any | Type::Unknown | Type::SpecValue => Type::Nil,
        Type::Parameter(parameter) => parameter.upper.as_ref().map_or(Type::Nil, nil_part),
        Type::Union(xs) if xs.iter().any(|t| nil_part(t) == Type::Nil) => Type::Nil,
        _ => Type::Never,
    }
}
