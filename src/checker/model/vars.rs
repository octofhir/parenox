//! Literal values declared as source vars.
use super::super::{Checker, Env};
use super::{Globals, Model};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LiteralVar {
    pub ty: Type,
    pub private: bool,
}

impl Model {
    pub(super) fn definition_name(&mut self, symbol: &str, span: Span, op: &str) -> Option<String> {
        if symbol.contains('/') && symbol != "/" {
            self.error(
                "PNX1001",
                span,
                format!("A {op} name must be an unqualified symbol"),
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

    pub(super) fn read_def(&mut self, xs: &[Form], span: Span, source: &str, globals: &Globals) {
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
        let initializer = match &xs[2..] {
            [] => {
                self.error(
                    "PNX9001",
                    name_form.span,
                    "An unbound def has no inferred value",
                );
                None
            }
            [value] => Some(value),
            [doc, value] if matches!(doc.kind, Kind::String(_)) => Some(value),
            _ => {
                self.error(
                    "PNX1001",
                    span,
                    "def accepts one initializer and an optional docstring",
                );
                None
            }
        };
        let dynamic = name_form.metadata(":dynamic").is_some_and(|value| {
            !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
        });
        let private = name_form.metadata(":private").is_some_and(|value| {
            !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
        });
        let ty = if dynamic {
            self.error(
                "PNX9001",
                name_form.span,
                "Dynamic vars are not supported yet",
            );
            Type::Unknown
        } else if let Some(value) = initializer {
            if literal(value) {
                let mut checker = Checker::new(self, globals, source);
                let result = checker.infer(value, &Env::new()).known;
                self.diagnostics.extend(checker.diagnostics);
                result
            } else {
                self.error(
                    "PNX9001",
                    value.span,
                    "def analysis currently requires a literal initializer",
                );
                Type::Unknown
            }
        } else {
            Type::Unknown
        };
        self.names.refers.remove(symbol);
        if self
            .literals
            .insert(name.clone(), LiteralVar { ty, private })
            .is_some()
        {
            self.error("PNX2790", name_form.span, format!("Duplicate var {name}"));
        }
    }
}

fn literal(form: &Form) -> bool {
    match &form.without_metadata().kind {
        Kind::Nil
        | Kind::Bool(_)
        | Kind::Number
        | Kind::String(_)
        | Kind::Character(_)
        | Kind::Keyword(_) => true,
        Kind::Vector(forms) | Kind::Map(forms) | Kind::Set(forms) => forms.iter().all(literal),
        _ => false,
    }
}
