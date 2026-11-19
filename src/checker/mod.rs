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
mod quote;
mod recur;

use env::Env;
use exceptions::Throws;
use model::{Contract, DeclaredValue, Function, VerifiedGuard};
pub(crate) use model::{
    Declarations, EncodeError, ExternCache, Globals, InterfaceKey, Model, ProjectCall,
    ScopedReference, SnapshotWriter, SpecCache, SpecFingerprint, SpecSource,
};
use recur::RecurTarget;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::reader::{Form, Kind, Span};
use crate::types::{Arity, Compatibility, MAX_TUPLE_ITEMS, Type};
use crate::{ContractOrigin, Diagnostic, Suggestion};

impl Model {
    pub fn check_function(
        &self,
        function: &Function,
        source: &str,
        globals: &Globals,
    ) -> Vec<Diagnostic> {
        let mut checker = Checker::new(self, globals, source);
        let contract = function
            .contract
            .as_ref()
            .expect("checked functions have contracts");
        for signature in contract.signatures.iter() {
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
            let result =
                checker.function_body(&arity.params, &arity.body, args, &Env::new(), arity.span);
            checker.expect_contract(
                &result,
                &signature.ret,
                arity.body.last().map_or(arity.span, |form| form.span),
                &format!("Return value of {}", function.name),
                Some(&contract.origin),
            );
        }
        checker.diagnostics
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
}

impl Value {
    fn new(known: Type) -> Self {
        Self {
            ty: known.inferred(),
            known,
            notes: Vec::new(),
            contract: None,
            guard: None,
        }
    }

    fn with_notes(known: Type, notes: Vec<String>) -> Self {
        Self {
            notes,
            ..Self::new(known)
        }
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
        Self {
            contract,
            guard,
            ..Self::with_notes(self.known.join_values(other.known), self.notes)
        }
    }

    fn narrow(&mut self, known: Type) {
        self.ty = known.inferred();
        self.known = known;
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
    diagnostics: Vec<Diagnostic>,
    active_helpers: Vec<String>,
    thread_depth: usize,
    specializations: BTreeMap<(String, Vec<Value>), calls::Specialization>,
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
            diagnostics: Vec::new(),
            active_helpers: Vec::new(),
            thread_depth: 0,
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
            // Unknown types often result from an error already reported for this function or
            // its declarations. If there is no local diagnostic, surface the unproven use.
            if self.diagnostics.is_empty() && self.model.diagnostics.is_empty() {
                self.error(
                    "PNX9004",
                    span,
                    format!("{context}: cannot verify {} against {expected}", actual.ty),
                );
            }
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
                diagnostic.suggestions.push(Suggestion {
                    message: "handle Nil explicitly; choose a fallback for your application".into(),
                    span,
                    replacement: format!("(or {expression} \"\")"),
                    applicability: "requires-review",
                });
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
                let digits = token.trim_start_matches(['+', '-']);
                let ty = if digits.len() > 1 && digits.starts_with('0') {
                    // A leading zero can denote octal syntax in Clojure.
                    Type::Number
                } else {
                    token.parse::<i32>().map_or(Type::Number, Type::Integer)
                };
                Value::new(ty)
            }
            Kind::SymbolicNumber(_) => Value::new(Type::Number),
            Kind::String(s) => Value::new(Type::Str(s.clone())),
            Kind::Utf16String(_) => Value::new(Type::String),
            Kind::Regex => Value::new(Type::Pattern),
            Kind::Character(c) => Value::new(Type::Char(*c)),
            Kind::Metadata(_, target) => self.infer_position(target, env, tail),
            Kind::Conditional { .. } => self.unsupported(form.span, "Reader conditional must be selected before type analysis"),
            Kind::Tagged(tag, _) => self.unsupported(form.span, format!(
                "Tagged literal #{} has no declared reader semantics; its reader function was not executed",
                tag.symbol().unwrap())),
            Kind::Keyword(k) => Value::new(Type::Key(self.model.key(k))),
            Kind::Symbol(s) | Kind::AnonymousArgument(s) => {
                if let Some(value) = env.get(s) {
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
                        DeclaredValue::ExternLiteral(ty, origin) => Value::with_notes(
                            ty.clone(),
                            origin.path.iter().map(|path| {
                                format!("Literal {} is declared in {path}.", origin.name)
                            }).collect(),
                        ),
                        DeclaredValue::Function(contract) => Value::from_contract(contract),
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
                let types: Vec<_> = self.eval_args(xs, env, true).map(|(value, _)| value.known).collect();
                if types.last() == Some(&Type::Never) {
                    return Value::new(Type::Never);
                }
                Value::new(vector_type(types))
            }
            Kind::Set(xs) => {
                let types: Vec<_> = xs.iter().map(|x| self.infer(x, env).known).collect();
                if types.contains(&Type::Never) {
                    return Value::new(Type::Never);
                }
                Value::new(Type::Set(collection_element(types)))
            }
            Kind::Map(xs) => self.map_literal(xs, maps::MapLiteral::Expression(env)),
            Kind::AutoNamespacedMap { .. } => self.unchecked_form(form.span, "Namespaced map needs source namespace context"),
            Kind::List(xs) if xs.is_empty() => Value::new(Type::empty_seq()),
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

fn collection_element(types: Vec<Type>) -> Box<Type> {
    Box::new(Type::union(types.into_iter().map(|ty| match ty {
        Type::Integer(_) => Type::Number,
        ty => ty,
    })))
}

fn nil_part(ty: &Type) -> Type {
    match ty {
        Type::Nil | Type::Unknown => Type::Nil,
        Type::Parameter(parameter) => parameter.upper.as_ref().map_or(Type::Nil, nil_part),
        Type::Union(xs) if xs.iter().any(|t| nil_part(t) == Type::Nil) => Type::Nil,
        _ => Type::Never,
    }
}
