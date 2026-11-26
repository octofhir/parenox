//! Literal values declared as source vars.
mod initializers;
mod sets;
mod strings;

use std::sync::Arc;

pub(super) use initializers::{adjacent_literal_def, inert, prior_initializer};
pub(super) use strings::LiteralInitializerScope;

use super::super::{Checker, Env, LiteralContainer, PatternCaptures, Value};
use super::{Declarations, Globals, Model};
use crate::reader::{Form, Kind, Span};
use crate::types::{IntegerClass, Type};

#[derive(Clone)]
pub(super) enum CapturedInitializer {
    Form(Arc<Form>),
    FixedLong(i64),
}

impl CapturedInitializer {
    pub(super) fn from_form(form: Form) -> Self {
        Self::Form(Arc::new(form))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::checker) struct LiteralVar {
    pub ty: Type,
    pub private: bool,
    pub(in crate::checker) container: Option<LiteralContainer>,
    pub(in crate::checker) pattern_captures: Option<PatternCaptures>,
}

impl LiteralVar {
    pub(super) fn unknown(private: bool) -> Self {
        Self {
            ty: Type::Unknown,
            private,
            container: None,
            pattern_captures: None,
        }
    }
}

impl Model {
    pub(super) fn withhold_literals_before_last_tag(
        &mut self,
        declarations: &Declarations,
        forms: &[Form],
    ) {
        let end = declarations.completed_before_last_tag();
        for &index in declarations
            .literal_vars
            .iter()
            .take_while(|&&index| index < end)
        {
            let Kind::List(items) = &forms[index].without_metadata().kind else {
                unreachable!("literal definitions are lists");
            };
            if let Some(name) = items.get(1).and_then(Form::symbol)
                && let Some(value) = self
                    .literals
                    .get_mut(&format!("{}/{name}", self.namespace()))
            {
                value.ty = Type::Unknown;
                value.container = None;
                value.pattern_captures = None;
            }
        }
    }

    pub(crate) fn literal_root_type(&self, name: &str) -> Option<&Type> {
        let ty = &self.literals.get(name)?.ty;
        (!ty.contains_unknown()).then_some(ty)
    }

    pub(super) fn capture_literal_root(
        &self,
        declarations: &Declarations,
        forms: &[Form],
        source: &str,
        index: usize,
        initializer: &Form,
    ) -> Option<CapturedInitializer> {
        if !matches!(
            initializer.kind,
            Kind::Symbol(_) | Kind::Vector(_) | Kind::Map(_)
        ) {
            return None;
        }
        let known_root = |name: &str| {
            self.literals
                .get(&format!("{}/{name}", self.names.namespace))
                .is_some_and(|root| !root.ty.contains_unknown())
        };
        let adjacent = (|| {
            let (previous, name, root) = initializers::adjacent_root(
                declarations,
                forms,
                source,
                index,
                &self.captured_initializers,
            )?;
            if !known_root(name) {
                return None;
            }
            if let Some(captured) = self.captured_initializers.get(&previous) {
                return matches!(&initializer.kind, Kind::Symbol(symbol) if symbol == name)
                    .then(|| captured.clone());
            }
            initializers::substitute(initializer, name, root).map(CapturedInitializer::from_form)
        })();
        adjacent.or_else(|| {
            initializers::capture_literal_run(declarations, forms, index, known_root)
                .map(CapturedInitializer::from_form)
        })
    }

    pub(super) fn definition_name(&mut self, symbol: &str, span: Span, op: &str) -> Option<String> {
        if symbol.contains('/') && symbol != "/" {
            self.error(
                "PNX1001",
                span,
                format!("A {op} name must be an unqualified symbol"),
            );
            return None;
        }
        if let Some(class) = self.names.imports.get(symbol) {
            self.error(
                "PNX2790",
                span,
                format!("{symbol} already refers to class {class}"),
            );
            return None;
        }
        if let Some(referred) = self.names.refers.get(symbol)
            && !referred.starts_with("clojure.core/")
        {
            self.error(
                "PNX2790",
                span,
                format!("{symbol} already refers to {referred}"),
            );
            return None;
        }
        Some(format!("{}/{symbol}", self.names.namespace))
    }

    pub(super) fn read_def(
        &mut self,
        xs: &[Form],
        span: Span,
        source: &str,
        captured: Option<&CapturedInitializer>,
        globals: &Globals,
    ) {
        let Some(name_form) = xs.get(1) else {
            self.error("PNX1001", span, "def requires a var name");
            return;
        };
        let Some(symbol) = name_form.symbol() else {
            self.error(
                "PNX1001",
                name_form.span,
                "def requires a symbol as its var name",
            );
            return;
        };
        let Some(name) = self.definition_name(symbol, name_form.span, "def") else {
            return;
        };
        let initializer = def_initializer(&xs[2..]);
        if initializer.is_none() {
            if xs.len() == 2 {
                self.unsupported_var(name_form.span, "An unbound def has no inferred value");
            } else {
                self.error(
                    "PNX1001",
                    span,
                    "def accepts one initializer and an optional docstring",
                );
            }
        }
        let dynamic = name_form.metadata(":dynamic").is_some_and(|value| {
            !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
        });
        let private = name_form.metadata(":private").is_some_and(|value| {
            !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
        });
        let expanded = match captured {
            Some(CapturedInitializer::Form(form)) => Some(form.as_ref()),
            _ => initializer,
        };
        let value = if dynamic {
            self.unsupported_var(name_form.span, "Dynamic vars are not supported yet");
            Value::new(Type::Unknown)
        } else if self.root_writes.contains(symbol) {
            self.unsupported_var(
                name_form.span,
                "Var root can change through alter-var-root; its initializer type is unchecked",
            );
            Value::new(Type::Unknown)
        } else if let Some(CapturedInitializer::FixedLong(value)) = captured {
            // A bounded fold has the same type as its decimal integer literal.
            Value::new(Type::exact_long(*value, IntegerClass::IntOrLong))
        } else if let Some(value) = expanded {
            if literal(value) {
                let mut checker = Checker::new(self, globals, source);
                let result = checker.infer(value, &Env::new());
                self.diagnostics.extend(checker.diagnostics);
                result
            } else {
                self.unsupported_var(
                    value.span,
                    "def analysis currently requires a literal initializer",
                );
                Value::new(Type::Unknown)
            }
        } else {
            Value::new(Type::Unknown)
        };
        let literal = LiteralVar {
            ty: value.known,
            private,
            container: value.literal_container,
            pattern_captures: value.pattern_captures,
        };
        self.insert_literal(symbol, name, literal, name_form.span);
    }

    pub(super) fn read_unchecked_var(&mut self, name_form: &Form, op: &str, private: bool) {
        let symbol = name_form.symbol().unwrap();
        if let Some(name) = self.definition_name(symbol, name_form.span, op) {
            self.insert_literal(symbol, name, LiteralVar::unknown(private), name_form.span);
        }
    }

    fn insert_literal(&mut self, symbol: &str, name: String, literal: LiteralVar, span: Span) {
        self.names.refers.remove(symbol);
        if self.literals.insert(name.clone(), literal).is_some() {
            self.error("PNX2790", span, format!("Duplicate var {name}"));
        }
    }

    fn unsupported_var(&mut self, span: Span, message: &str) {
        self.diagnostics
            .push(crate::Diagnostic::analysis_gap("PNX9001", span, message));
    }
}

pub(super) fn def_initializer(tail: &[Form]) -> Option<&Form> {
    match tail {
        [value] => Some(value),
        [doc, value] if doc.is_string() => Some(value),
        _ => None,
    }
}

pub(super) fn literal(form: &Form) -> bool {
    match &form.without_metadata().kind {
        Kind::Nil
        | Kind::Bool(_)
        | Kind::Number
        | Kind::SymbolicNumber(_)
        | Kind::String(_)
        | Kind::Utf16String(_)
        | Kind::Character(_)
        | Kind::Keyword(_)
        | Kind::Regex { .. }
        | Kind::Quote(_) => true,
        Kind::Vector(forms) | Kind::Map(forms) | Kind::Set(forms) => forms.iter().all(literal),
        Kind::List(forms) => match forms.as_slice() {
            [] => true,
            [head, _] => head.symbol() == Some("quote"),
            _ => false,
        },
        _ => false,
    }
}
