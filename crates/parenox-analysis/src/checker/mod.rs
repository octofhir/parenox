//! Source analysis for the supported Clojure subset.
mod bindings;
mod calls;
mod case;
mod env;
mod exceptions;
mod flow;
mod function_forms;
mod maps;
mod metadata;
mod model;
mod queries;
mod quote;
mod recur;

use env::Env;
use exceptions::Throws;
use model::{Contract, DeclaredValue, Function, VerifiedGuard};
pub(crate) use model::{
    Declarations, DocumentReferences, EncodeError, ExternCache, Globals, InterfaceKey, Model,
    ProjectCall, RuntimeLoads, ScopedReference, SnapshotWriter, SpecCache, SpecConversion,
    SpecFingerprint, SpecSource, bare_dotted_name, java_type_tag_at,
};
pub(crate) use queries::CheckedCall;
use queries::TypeQuery;
use recur::RecurTarget;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::reader::{Form, Kind, Span};
use crate::types::{
    Arity, Compatibility, ExtraFields, IntegerRange, IntegerType, MAX_TUPLE_ITEMS, Signature, Type,
};
use crate::{ContractOrigin, Diagnostic, Suggestion};

pub(crate) struct FunctionCheck {
    pub diagnostics: Vec<Diagnostic>,
    pub dotted_callable_sites: Box<[Span]>,
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
            let (result, return_span) = checker.declared_function_body(function, signature);
            checker.expect_contract(
                &result,
                &signature.ret,
                return_span,
                &format!("Return value of {}", function.name),
                Some(&contract.origin),
            );
        }
        FunctionCheck {
            diagnostics: checker.diagnostics,
            dotted_callable_sites: checker.dotted_callable_sites.into_boxed_slice(),
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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LiteralContainer {
    Map(Option<Arc<BTreeMap<Type, LiteralContainer>>>),
    Set(Option<Arc<[Type]>>),
    Vector(Option<Arc<[Option<LiteralContainer>]>>),
    List(Option<Arc<[Option<LiteralContainer>]>>),
    Positions(Arc<[Option<LiteralContainer>]>),
}

impl LiteralContainer {
    fn map_field(&self, key: &Type) -> Option<&Self> {
        let Self::Map(Some(fields)) = self else {
            return None;
        };
        fields.get(key)
    }

    fn position_child(&self, index: usize) -> Option<&Self> {
        let items = self.child_positions()?;
        items.get(index).and_then(Option::as_ref)
    }

    fn child_positions(&self) -> Option<&Arc<[Option<Self>]>> {
        match self {
            Self::Vector(Some(items)) | Self::List(Some(items)) | Self::Positions(items) => {
                Some(items)
            }
            _ => None,
        }
    }

    fn matches_type(&self, ty: &Type) -> bool {
        match (self, ty) {
            (Self::Map(_), Type::Map(_)) => true,
            (Self::Set(_), Type::Set(_)) => true,
            (Self::Vector(items), Type::Tuple(types))
            | (Self::List(items), Type::ListTuple(types)) => items
                .as_ref()
                .is_none_or(|items| items.len() == types.len()),
            (Self::Positions(items), Type::SeqTuple(types) | Type::ListTuple(types)) => {
                items.len() == types.len()
            }
            (Self::Positions(items), Type::MapEntry(_)) => items.len() == 2,
            (
                Self::Vector(_) | Self::List(_) | Self::Positions(_) | Self::Set(_),
                Type::Union(parts),
            ) => parts
                .iter()
                .all(|part| matches!(part, Type::Nil) || self.matches_type(part)),
            _ => false,
        }
    }

    fn join(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Set(left), Self::Set(right)) => {
                Some(Self::Set(if left == right { left } else { None }))
            }
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
            (left @ Self::Vector(_), right @ Self::Vector(_)) => Some(if left == right {
                left
            } else {
                Self::Vector(None)
            }),
            (left @ Self::List(_), right @ Self::List(_)) => Some(if left == right {
                left
            } else {
                Self::List(None)
            }),
            (left @ Self::Positions(_), right) if left == right => Some(left),
            _ => None,
        }
    }
}

enum SetMembership<'a> {
    Present(&'a Type),
    Absent,
    Unproven,
}

impl SetMembership<'_> {
    fn present(self) -> Option<bool> {
        match self {
            Self::Present(_) => Some(true),
            Self::Absent => Some(false),
            Self::Unproven => None,
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
    pattern_captures: Option<u16>,
    rejection: Option<CallRejection>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CallRejection {
    // A declared contract failed, leaving the runtime result unproved.
    Contract,
    // The operation has no normal return for these arguments.
    Runtime,
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
            pattern_captures: None,
            rejection: None,
        }
    }

    fn rejected_call() -> Self {
        Self {
            rejection: Some(CallRejection::Contract),
            ..Self::new(Type::Unknown)
        }
    }

    fn rejected_runtime_call() -> Self {
        Self {
            rejection: Some(CallRejection::Runtime),
            ..Self::new(Type::Unknown)
        }
    }

    fn is_rejected(&self) -> bool {
        self.rejection.is_some()
    }

    fn stops_evaluation(&self) -> bool {
        self.known == Type::Never || self.rejection == Some(CallRejection::Runtime)
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

    fn literal_set(members: Vec<Type>) -> Self {
        let exact = (members.len() <= MAX_TUPLE_ITEMS
            && members.iter().all(|ty| exact_set_member(ty).is_some()))
        .then(|| {
            let mut proof: Arc<[Type]> = Arc::from(members.as_slice());
            // Exact membership carries no information about iteration order.
            Arc::make_mut(&mut proof).sort_unstable();
            proof
        });
        Self {
            literal_container: Some(LiteralContainer::Set(exact)),
            ..Self::new(Type::Set(collection_element(members)))
        }
    }

    fn exact_set_members(&self) -> Option<&[Type]> {
        match (&self.known, &self.literal_container) {
            (Type::Set(_), Some(LiteralContainer::Set(Some(members)))) => Some(members),
            _ => None,
        }
    }

    fn set_membership(&self, key: &Type) -> SetMembership<'_> {
        let Some(members) = self.exact_set_members() else {
            return SetMembership::Unproven;
        };
        let mut distinct = true;
        for member in members.iter() {
            match calls::literal_equality(member, key) {
                Some(true) => return SetMembership::Present(member),
                Some(false) => {}
                None => distinct = false,
            }
        }
        if distinct {
            SetMembership::Absent
        } else {
            SetMembership::Unproven
        }
    }

    fn set_lookup_type(&self, element: &Type, key: &Type, default: Type) -> Type {
        if self.exact_set_members().is_some()
            && let Type::Union(keys) = key
            && keys.len() <= MAX_TUPLE_ITEMS
        {
            return Type::union(
                keys.iter()
                    .map(|key| self.set_lookup_type(element, key, default.clone())),
            );
        }
        match self.set_membership(key) {
            SetMembership::Present(member) => member.clone(),
            SetMembership::Absent => default,
            SetMembership::Unproven => {
                let stored = if self.is_literal_set() {
                    element.hash_set_member_type(key)
                } else {
                    // Arbitrary sets may use a custom comparator.
                    element.clone()
                };
                Type::union([stored, default])
            }
        }
    }

    fn set_presence(&self, key: &Type) -> Option<bool> {
        if let Type::Union(keys) = key
            && keys.len() <= MAX_TUPLE_ITEMS
        {
            let mut outcomes = keys.iter().map(|key| self.set_membership(key).present());
            let first = outcomes.next()??;
            return outcomes
                .all(|outcome| outcome == Some(first))
                .then_some(first);
        }
        self.set_membership(key).present()
    }

    fn is_literal_set(&self) -> bool {
        matches!(self.literal_container, Some(LiteralContainer::Set(_)))
    }

    fn with_ordinary_tuple(mut self) -> Self {
        let proof = LiteralContainer::Vector(None);
        if proof.matches_type(&self.known) {
            self.literal_container = Some(proof);
        }
        self
    }

    fn position_value(&self, known: Type, index: usize) -> Self {
        let mut value = Self::with_notes(known, self.notes.clone());
        value.literal_container = self
            .literal_container
            .as_ref()
            .and_then(|container| container.position_child(index))
            .filter(|proof| proof.matches_type(&value.known))
            .cloned();
        value
    }

    fn traversal_element(&self, known: Type) -> Self {
        fn shared<'a>(
            mut origins: impl Iterator<Item = Option<&'a LiteralContainer>>,
        ) -> Option<LiteralContainer> {
            let first = origins.next()??.clone();
            origins.try_fold(first, |shared, proof| shared.join(proof?.clone()))
        }

        let origin = match (&self.known, &self.literal_container) {
            (Type::Map(map), Some(LiteralContainer::Map(Some(origins))))
                if matches!(map.extra, ExtraFields::Closed) =>
            {
                let payload = shared(
                    map.fields
                        .iter()
                        .filter(|(_, field)| field.ty != Type::Never)
                        .map(|(key, _)| origins.get(key)),
                );
                payload.map(|proof| LiteralContainer::Positions(Arc::from([None, Some(proof)])))
            }
            (_, proof) => proof
                .as_ref()
                .and_then(LiteralContainer::child_positions)
                .and_then(|children| shared(children.iter().map(Option::as_ref))),
        };
        Self {
            literal_container: origin.filter(|proof| proof.matches_type(&known)),
            ..Self::with_notes(known, self.notes.clone())
        }
    }

    fn unproven_tuple_traversal(&self) -> bool {
        fn contains_tuple(ty: &Type) -> bool {
            match ty {
                Type::Tuple(_) => true,
                Type::Union(parts) => parts.iter().any(contains_tuple),
                Type::Parameter(parameter) => parameter.upper.as_ref().is_some_and(contains_tuple),
                _ => false,
            }
        }
        !matches!(self.literal_container, Some(LiteralContainer::Vector(_)))
            && contains_tuple(&self.known)
    }

    fn merge_container_origins(&mut self, other: &mut Self) -> Option<LiteralContainer> {
        match (
            self.literal_container.take(),
            other.literal_container.take(),
        ) {
            (Some(left), Some(right)) => left.join(right),
            (proof, None) if matches!(other.known, Type::Nil | Type::Never) => proof,
            (None, proof) if matches!(self.known, Type::Nil | Type::Never) => proof,
            _ => None,
        }
    }

    fn union_branches(branches: impl IntoIterator<Item = Self>) -> Self {
        let mut types = Vec::new();
        let mut notes = Vec::new();
        let mut origin: Option<LiteralContainer> = None;
        let mut have_value = false;
        for branch in branches {
            if branch.stops_evaluation() {
                notes.extend(branch.notes);
                continue;
            }
            if branch.known != Type::Nil {
                origin = if have_value {
                    origin
                        .zip(branch.literal_container)
                        .and_then(|(left, right)| left.join(right))
                } else {
                    branch.literal_container
                };
                have_value = true;
            }
            types.push(branch.known);
            notes.extend(branch.notes);
        }
        let mut result = Self::with_notes(Type::union(types), notes);
        result.literal_container = origin.filter(|proof| proof.matches_type(&result.known));
        result
    }

    fn join(mut self, mut other: Value) -> Value {
        if self.known == Type::Never {
            return other;
        }
        if other.known == Type::Never {
            return self;
        }
        let literal_container = self.merge_container_origins(&mut other);
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
        let pattern_captures = if self.pattern_captures == other.pattern_captures {
            self.pattern_captures
        } else {
            None
        };
        let known = self.known.join_values(other.known);
        Self {
            contract,
            guard,
            literal_container: literal_container.filter(|proof| proof.matches_type(&known)),
            pattern_captures,
            ..Self::with_notes(known, self.notes)
        }
    }

    fn narrow(&mut self, known: Type) {
        let keep = self
            .literal_container
            .as_ref()
            .is_some_and(|proof| proof.matches_type(&known));
        if !keep {
            self.literal_container = None;
        }
        if known != Type::Pattern {
            self.pattern_captures = None;
        }
        self.ty = known.inferred();
        self.known = known;
    }

    fn same_value_facts(&self, other: &Self) -> bool {
        self.known == other.known
            && self.literal_container == other.literal_container
            && self.pattern_captures == other.pattern_captures
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
    partial_scope: Option<Cow<'a, BTreeSet<&'a str>>>,
    partial_method_body: bool,
    partial_method_opaque: bool,
    type_query: Option<TypeQuery<'a>>,
    untyped_binding_budget: Option<usize>,
    literal_binding_query: bool,
    diagnostics: Vec<Diagnostic>,
    dotted_callable_sites: Vec<Span>,
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
    search_steps: usize,
    vector_transform_steps: usize,
    map_callback_checks: usize,
    callback_depth: usize,
    local_inline_steps: usize,
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
            partial_method_body: false,
            partial_method_opaque: false,
            type_query: None,
            untyped_binding_budget: None,
            literal_binding_query: false,
            diagnostics: Vec::new(),
            dotted_callable_sites: Vec::new(),
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
            search_steps: 0,
            vector_transform_steps: 0,
            map_callback_checks: 0,
            callback_depth: 0,
            local_inline_steps: 0,
            macro_scope_depth: 0,
            throws: None,
        }
    }
}

impl Checker<'_> {
    fn declared_function_body(
        &mut self,
        function: &Function,
        signature: &Signature,
    ) -> (Value, Span) {
        let clause = self.model.source_clause(function, signature.arity());
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
        let result = self.function_body(
            &clause.bindings(),
            clause.conditions,
            clause.body,
            args,
            &Env::new(),
            clause.span,
        );
        let return_span = clause.body.last().map_or(clause.span, |form| form.span);
        (result, return_span)
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
        self.dedup_diagnostics_with_prior(start, None);
    }

    fn dedup_diagnostics_with_prior(&mut self, start: usize, compiled: Option<Span>) {
        let count = self.diagnostics.len() - start;
        if count == 0 || (count == 1 && compiled.is_none()) {
            return;
        }
        let key = |diagnostic: &Diagnostic| {
            (
                diagnostic.span.start.offset,
                diagnostic.span.end.offset,
                diagnostic.code.clone(),
                diagnostic.message.clone(),
                diagnostic.contract.clone(),
            )
        };
        let mut reported = BTreeSet::new();
        if let Some(span) = compiled {
            reported.extend(
                self.diagnostics[..start]
                    .iter()
                    .filter(|diagnostic| {
                        diagnostic.span.start.offset >= span.start.offset
                            && diagnostic.span.end.offset <= span.end.offset
                    })
                    .map(key),
            );
        }
        let diagnostics = self.diagnostics.split_off(start);
        self.diagnostics.extend(
            diagnostics
                .into_iter()
                .filter(|diagnostic| reported.insert(key(diagnostic))),
        );
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

    fn unchecked_alias_value(&mut self, symbol: &str, span: Span, env: &Env) -> Option<Value> {
        let namespace = self.model.runtime_alias_gap(symbol)?;
        self.partial_method_opaque |= self.partial_method_body;
        self.withhold_captured_facts(env.inline_functions());
        self.withhold_literals_after_opaque_form();
        self.diagnostics
            .push(Diagnostic::runtime_alias_gap(symbol, namespace, span));
        self.may_throw();
        Some(Value::new(Type::Unknown))
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
        let Some((last, prefix)) = body.split_last() else {
            return Value::new(if env.reachable() {
                Type::Nil
            } else {
                Type::Never
            });
        };
        let reachable = self.body_prefix_returns(prefix, env, true);
        let mut value = self.infer_path(last, env, reachable, tail);
        if value.known != Type::Never && value.stops_evaluation() {
            value.narrow(Type::Never);
        }
        value
    }

    fn body_prefix_returns(&mut self, prefix: &[Form], env: &Env, reachable: bool) -> bool {
        let mut reachable = reachable && env.reachable();
        for form in prefix {
            reachable = !self
                .infer_path(form, env, reachable, false)
                .stops_evaluation();
        }
        reachable
    }

    fn infer(&mut self, form: &Form, env: &Env) -> Value {
        self.infer_position(form, env, false)
    }

    fn infer_set_members(&mut self, forms: &[Form], env: &Env) -> Option<Vec<Type>> {
        // Runtime visits the reader's hash-set order. A failing member prevents
        // construction, but another member may have been evaluated first.
        let mut stops = false;
        let inputs: Vec<_> = forms
            .iter()
            .map(|form| {
                let value = self.infer(form, env);
                stops |= value.stops_evaluation();
                value.known
            })
            .collect();
        if stops {
            return None;
        }
        if forms.len() >= 2
            && forms.iter().any(|form| {
                matches!(
                    form.without_metadata().kind,
                    Kind::List(_) | Kind::Symbol(_) | Kind::AnonymousArgument(_)
                )
            })
        {
            let mut members = BTreeSet::new();
            for (form, ty) in forms.iter().zip(&inputs) {
                if let Some(identity) = exact_set_member(ty)
                    && !members.insert(identity)
                {
                    if self.path_reachable && env.reachable() {
                        self.error("PNX2790", form.span, format!("Duplicate set element {ty}"));
                        self.illegal_argument();
                    }
                    return None;
                }
            }
        }
        Some(inputs)
    }

    fn eval_args<'a>(
        &'a mut self,
        forms: &'a [Form],
        env: &'a Env,
        reachable: bool,
    ) -> impl Iterator<Item = (Value, Span)> + 'a {
        self.eval_argument_forms(forms.iter(), env, reachable)
    }

    fn eval_argument_forms<'a>(
        &'a mut self,
        forms: impl Iterator<Item = &'a Form> + 'a,
        env: &'a Env,
        mut reachable: bool,
    ) -> impl Iterator<Item = (Value, Span)> + 'a {
        // Remaining expressions still need compiler checks after a proved stop.
        forms.map(move |form| {
            let mut value = self.infer_path(form, env, reachable, false);
            reachable = !value.stops_evaluation();
            if !reachable {
                value.narrow(Type::Never);
            }
            (value, form.span)
        })
    }

    fn infer_position(&mut self, form: &Form, env: &Env, tail: bool) -> Value {
        if let Some(remaining) = self.untyped_binding_budget {
            if self.type_query.is_none()
                || remaining == 0
                || (self.path_reachable
                    && !self.clean_binding_check()
                    && !self.can_keep_literal_bindings())
            {
                self.type_query = None;
                return Value::new(Type::Unknown);
            }
            self.untyped_binding_budget = Some(remaining - 1);
        }
        if self.partial_method_body
            && matches!(
                &form.kind,
                Kind::Metadata(..)
                    | Kind::Tagged(..)
                    | Kind::AutoNamespacedMap { .. }
                    | Kind::SyntaxQuote(_)
                    | Kind::Unquote { .. }
            )
        {
            self.partial_method_opaque = true;
            return Value::new(Type::Unknown);
        }
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
            Kind::Regex { captures } => Value {
                pattern_captures: *captures,
                ..Value::new(Type::Pattern)
            },
            Kind::Character(c) => Value::new(Type::Char(*c)),
            Kind::Metadata(_, target) => {
                let bare = target.without_metadata();
                let metadata_target = match &bare.kind {
                    Kind::Vector(_) => Some(metadata::MetadataTarget::Vector),
                    Kind::Map(_) => Some(metadata::MetadataTarget::Map),
                    _ => None,
                };
                if let Some(target) = metadata_target {
                    let value = self.infer_position(bare, env, tail);
                    return self
                        .literal_metadata_outcome(
                            form,
                            env,
                            !value.stops_evaluation(),
                            target,
                        )
                        .unwrap_or(value);
                }
                let mut value = self.infer_position(target, env, tail);
                value.literal_container = None;
                value
            }
            Kind::Conditional { .. } => self.unsupported(form.span, "Reader conditional must be selected before type analysis"),
            Kind::Tagged(tag, _) => self.unsupported(form.span, format!(
                "Tagged literal #{} has no declared reader semantics; its reader function was not executed",
                tag.symbol().unwrap())),
            Kind::Keyword(k) => Value::new(Type::Key(self.model.key(k))),
            Kind::Symbol(name) | Kind::AnonymousArgument(name) => self.symbol_value(name, form.span, env),
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
                self.infer_set_members(xs, env)
                    .map_or_else(|| Value::new(Type::Never), Value::literal_set)
            }
            Kind::Map(xs) => self.map_literal(xs, maps::MapLiteral::Expression(env)),
            Kind::AutoNamespacedMap { .. } => self.unchecked_form(form.span, "Namespaced map needs source namespace context"),
            Kind::List(xs) if xs.is_empty() => Value {
                literal_container: Some(LiteralContainer::List(None)),
                ..Value::new(Type::empty_list())
            },
            Kind::List(xs) => self.list(xs, form.span, env, tail),
            Kind::AnonymousFunction(function) => self.anonymous_fn_value(function, form.span, env),
            Kind::Quote(inner) => self.quoted_data(inner),
            Kind::SyntaxQuote(_) | Kind::Unquote { .. } => {
                self.withhold_literals_after_opaque_form();
                self.unchecked_form(form.span, "Syntax quote and unquote expansion is not supported")
            }
        }
    }

    // Keep symbol resolution's temporaries out of the recursive expression frame.
    fn symbol_value(&mut self, name: &str, span: Span, env: &Env) -> Value {
        self.observe_completion(span, env);
        if let Some(value) = env.get(name) {
            if self.path_reachable
                && let Some(function) = env.inline_function(name)
            {
                self.withhold_captured_facts(Some(function));
                self.diagnostics.push(Diagnostic::analysis_gap(
                    "PNX9001",
                    span,
                    "The type of a local fn is unchecked outside supported calls",
                ));
            }
            self.observe_local(name, span, value);
            value.clone()
        } else if self.partial_method_body {
            Value::new(Type::Unknown)
        } else if name.contains('.')
            && let Some(value) = self.dotted_function_class_use(name, span, false)
        {
            value
        } else if self
            .model
            .local_function_at(name, span.start.offset)
            .is_some_and(Function::inferred)
        {
            if !self.path_reachable {
                return Value::new(Type::Never);
            }
            self.error(
                "PNX9002",
                span,
                format!("Private function {name} needs a :t/sig annotation when used as a value"),
            );
            Value::new(Type::Unknown)
        } else if let Some(value) = self.unchecked_alias_value(name, span, env) {
            value
        } else if let Some(value) = self
            .model
            .declared_value(name, self.globals, span.start.offset)
        {
            match value {
                DeclaredValue::Literal(literal) => Value {
                    literal_container: literal.container.clone(),
                    pattern_captures: literal.pattern_captures,
                    ..Value::new(literal.ty.clone())
                },
                DeclaredValue::ExternLiteral(literal)
                    if self.partial_contract_visible(&literal.origin) =>
                {
                    Value {
                        literal_container: literal.container.clone(),
                        ..Value::with_notes(
                            literal.ty.clone(),
                            literal
                                .origin
                                .path
                                .iter()
                                .map(|path| {
                                    format!(
                                        "Literal {} is declared in {path}.",
                                        literal.origin.name
                                    )
                                })
                                .collect(),
                        )
                    }
                }
                DeclaredValue::ExternLiteral(_) => Value::new(Type::Unknown),
                DeclaredValue::Function(contract)
                    if self.partial_contract_visible(&contract.origin) =>
                {
                    Value::from_contract(contract)
                }
                DeclaredValue::Function(_) => Value::new(Type::Unknown),
                DeclaredValue::UncheckedFunction(_) => self.unchecked_function(name, span),
                DeclaredValue::Macro(scope) => self.macro_gap(
                    scope,
                    span,
                    format!("Macro {} is not a function value", scope.origin().name),
                ),
                DeclaredValue::OpaqueMacro => {
                    self.unchecked_form(span, &format!("Macro {name} has no function value"))
                }
                DeclaredValue::PrivateMacro => {
                    self.error(
                        "PNX2304",
                        span,
                        format!("Unresolved or private value {name}"),
                    );
                    Value::new(Type::Unknown)
                }
            }
        } else if self.partial_scope.is_some() {
            Value::new(Type::Unknown)
        } else {
            let resolved = self.model.resolve(name);
            match self
                .model
                .standard_core_binding(name, &resolved, self.globals)
            {
                Some(crate::core_names::CoreBinding::Var) => {
                    return self.unchecked_core_function(&resolved, span);
                }
                Some(crate::core_names::CoreBinding::Macro) => {
                    self.error(
                        "PNX2304",
                        span,
                        format!("Macro {resolved} has no function value"),
                    );
                    return Value::new(Type::Unknown);
                }
                None => {}
            }
            if self.model.known_java_class(name)
                || self.model.known_java_array_class(name, self.globals)
            {
                self.diagnostics.push(Diagnostic::analysis_gap(
                    "PNX9001",
                    span,
                    "Type analysis for JVM class values is unavailable",
                ));
                return Value::new(Type::Unknown);
            }
            if self.model.known_java_member(name, self.globals) {
                return self.unchecked_form(span, "JVM member values are not checked");
            }
            let external = self.model.has_unavailable_value_scope(name, self.globals);
            self.error(
                if external { "PNX9003" } else { "PNX2304" },
                span,
                if external {
                    format!("No declaration for external value {name}")
                } else {
                    format!("Unresolved or private value {name}")
                },
            );
            Value::new(Type::Unknown)
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

fn exact_set_member(ty: &Type) -> Option<Cow<'_, Type>> {
    match ty {
        Type::Nil | Type::Bool(_) | Type::Key(_) | Type::Str(_) | Type::Char(_) | Type::Sym(_) => {
            Some(Cow::Borrowed(ty))
        }
        _ => ty
            .exact_long_value()
            .map(|value| Cow::Owned(Type::exact_long(value, crate::types::IntegerClass::Fixed))),
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
