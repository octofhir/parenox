//! Source analysis for the supported Clojure subset.
mod bindings;
mod calls;
mod case;
mod env;
mod flow;
mod maps;
mod model;
mod recur;

use env::Env;
use model::Function;
pub(crate) use model::{Declarations, Globals, Model, SpecCache, SpecFingerprint, SpecSource};
use recur::RecurTarget;

use std::collections::BTreeMap;

use crate::reader::{Form, Kind, Span};
use crate::types::{ExtraFields, Field, MAX_TUPLE_ITEMS, MapType, Type};
use crate::{Diagnostic, Suggestion};

impl Model {
    pub fn check_function(
        &self,
        function: &Function,
        source: &str,
        globals: &Globals,
    ) -> Vec<Diagnostic> {
        let mut checker = Checker {
            model: self,
            globals,
            source,
            diagnostics: Vec::new(),
            active_helpers: Vec::new(),
            thread_depth: 0,
            specializations: BTreeMap::new(),
            recur_target: None,
            recur_checks: 0,
        };
        let args = function.args.iter().cloned().map(Value::new).collect();
        let result = checker.function_body(
            &function.params,
            &function.body,
            args,
            &Env::new(),
            function.span,
        );
        checker.expect(
            &result,
            &function.ret,
            function.body.last().unwrap().span,
            &format!("Return value of {}", function.name),
        );
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
}

impl Value {
    fn new(known: Type) -> Self {
        Self {
            ty: known.inferred(),
            known,
            notes: Vec::new(),
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
        Value::with_notes(self.known.join_values(other.known), self.notes)
    }

    fn narrow(&mut self, known: Type) {
        self.ty = known.inferred();
        self.known = known;
    }
}

struct Checker<'a> {
    model: &'a Model,
    globals: &'a Globals,
    source: &'a str,
    diagnostics: Vec<Diagnostic>,
    active_helpers: Vec<String>,
    thread_depth: usize,
    specializations: BTreeMap<(String, Vec<Value>), (Value, Vec<Diagnostic>)>,
    recur_target: Option<RecurTarget>,
    recur_checks: usize,
}

impl Checker<'_> {
    fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    fn unsupported(&mut self, span: Span, message: impl Into<String>) -> Value {
        self.error("PNX9001", span, message);
        Value::new(Type::Unknown)
    }

    fn expect(&mut self, actual: &Value, expected: &Type, span: Span, context: &str) {
        if actual.ty.contains_unknown() || expected.contains_unknown() {
            return; // The originating unsupported/unresolved form has its own diagnostic.
        }
        if !actual.known.subtype_of(expected) {
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
            Kind::String(s) => Value::new(Type::Str(s.clone())),
            Kind::Character(c) => Value::new(Type::Char(*c)),
            Kind::Metadata(_, target) => self.infer_position(target, env, tail),
            Kind::Conditional { .. } => self.unsupported(form.span, "Reader conditional must be selected before type analysis"),
            Kind::Tagged(tag, _) => self.unsupported(form.span, format!(
                "Tagged literal #{} has no declared reader semantics; its reader function was not executed",
                tag.symbol().unwrap())),
            Kind::Keyword(k) => Value::new(Type::Key(self.model.key(k))),
            Kind::Symbol(s) => {
                if let Some(value) = env.get(s) {
                    value.clone()
                } else if self.model.local_function(s).is_some_and(|f| f.inferred) {
                    self.error(
                        "PNX9002",
                        form.span,
                        format!(
                            "Private function {s} needs a :t/sig annotation when used as a value"
                        ),
                    );
                    Value::new(Type::Unknown)
                } else if let Some((args, ret)) = self.model.signature(s, self.globals) {
                    Value::new(Type::Function(args.to_vec(), Box::new(ret.clone())))
                } else {
                    let qualified = self.model.function_name(s);
                    let external = qualified.rsplit_once('/').is_some_and(|(namespace, _)| {
                        namespace != self.model.namespace()
                            && !self.globals.project_namespaces.contains(namespace)
                    });
                    self.error(
                        if external { "PNX9003" } else { "PNX2304" },
                        form.span,
                        if external {
                            format!("No declared signature for external value {s}")
                        } else {
                            format!("Unresolved or private value {s}")
                        },
                    );
                    Value::new(Type::Unknown)
                }
            }
            Kind::Vector(xs) if xs.len() <= MAX_TUPLE_ITEMS => Value::new(Type::Tuple(
                xs.iter().map(|x| self.infer(x, env).known).collect(),
            )),
            Kind::Vector(xs) | Kind::Set(xs) => {
                let types = xs
                    .iter()
                    .map(|x| match self.infer(x, env).known {
                        Type::Integer(_) => Type::Number,
                        ty => ty,
                    })
                    .collect::<Vec<_>>();
                let element = Box::new(Type::union(types));
                Value::new(if matches!(form.kind, Kind::Set(_)) {
                    Type::Set(element)
                } else {
                    Type::Vector(element)
                })
            }
            Kind::Map(xs) => {
                let mut fields = BTreeMap::new();
                for pair in xs.as_chunks::<2>().0 {
                    let value = self.infer(&pair[1], env);
                    if let Some(key) = self.literal_map_key(&pair[0]) {
                        if fields
                            .insert(
                                key.clone(),
                                Field {
                                    ty: value.known,
                                    required: true,
                                },
                            )
                            .is_some()
                        {
                            self.error("PNX2790", pair[0].span, format!("Duplicate map key {key}"));
                        }
                    } else {
                        self.unsupported(pair[0].span, "Map literals currently require keyword or string keys");
                    }
                }
                Value::new(Type::Map(MapType {
                    fields,
                    extra: ExtraFields::Closed,
                }))
            }
            Kind::List(xs) if xs.is_empty() => {
                self.unsupported(form.span, "Empty list values are not supported yet")
            }
            Kind::List(xs) => self.list(xs, form.span, env, tail),
            Kind::Quote(_) => self.unsupported(form.span, "Quoted data is not supported yet"),
        }
    }
}

fn nil_part(ty: &Type) -> Type {
    match ty {
        Type::Nil | Type::Unknown => Type::Nil,
        Type::Union(xs) if xs.iter().any(|t| nil_part(t) == Type::Nil) => Type::Nil,
        _ => Type::Never,
    }
}
