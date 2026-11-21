//! Literal values declared as source vars.
mod initializers;
mod strings;

pub(super) use initializers::adjacent_literal_def;
pub(super) use strings::LiteralStrScope;

use super::super::{Checker, Env};
use super::{Declarations, Globals, Model};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LiteralVar {
    pub ty: Type,
    pub private: bool,
}

impl Model {
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
    ) -> Option<Form> {
        if !matches!(
            initializer.kind,
            Kind::Symbol(_) | Kind::Vector(_) | Kind::Map(_)
        ) {
            return None;
        }
        let (name, root) = initializers::adjacent_root(declarations, forms, source, index)?;
        let literal = self
            .literals
            .get(&format!("{}/{name}", self.names.namespace))?;
        if literal.ty.contains_unknown() {
            return None;
        }
        initializers::substitute(initializer, name, root)
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
        expanded: Option<&Form>,
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
        let ty = if dynamic {
            self.unsupported_var(name_form.span, "Dynamic vars are not supported yet");
            Type::Unknown
        } else if let Some(value) = expanded.or(initializer) {
            if literal(value) {
                let mut checker = Checker::new(self, globals, source);
                let result = checker.infer(value, &Env::new()).known;
                self.diagnostics.extend(checker.diagnostics);
                result
            } else {
                self.unsupported_var(
                    value.span,
                    "def analysis currently requires a literal initializer",
                );
                Type::Unknown
            }
        } else {
            Type::Unknown
        };
        self.insert_literal(symbol, name, ty, private, name_form.span);
    }

    pub(super) fn read_unchecked_var(&mut self, name_form: &Form, op: &str, private: bool) {
        let symbol = name_form.symbol().unwrap();
        if let Some(name) = self.definition_name(symbol, name_form.span, op) {
            self.insert_literal(symbol, name, Type::Unknown, private, name_form.span);
        }
    }

    fn insert_literal(&mut self, symbol: &str, name: String, ty: Type, private: bool, span: Span) {
        self.names.refers.remove(symbol);
        if self
            .literals
            .insert(name.clone(), LiteralVar { ty, private })
            .is_some()
        {
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
