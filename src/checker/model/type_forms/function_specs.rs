//! Function specs shared by s/fspec type forms and top-level s/fdef declarations.
use std::collections::BTreeSet;

use super::{Names, TypeReader};
use crate::Diagnostic;
use crate::checker::function_forms::MAX_FIXED_PARAMETERS;
use crate::reader::{Form, Kind, Span};
use crate::types::{Signature, Type};

pub(in crate::checker::model) fn read_function_spec(
    names: &Names,
    options: &[Form],
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
    lookup: impl FnMut(&str, Span) -> Option<Type>,
) -> Option<Signature> {
    TypeReader {
        names,
        parameters: None,
        literal_shapes: false,
        diagnostics,
        lookup,
    }
    .function_spec(options, span, "s/fdef")
}

impl<F: FnMut(&str, Span) -> Option<Type>> TypeReader<'_, F> {
    pub(super) fn function_spec(
        &mut self,
        options: &[Form],
        span: Span,
        operator: &str,
    ) -> Option<Signature> {
        let (pairs, remainder) = options.as_chunks::<2>();
        if let [key] = remainder {
            self.error(
                "PNX9001",
                key.span,
                format!("{operator} option requires a value"),
            );
            return None;
        }
        let (mut args, mut ret) = (None, None);
        for [key, value] in pairs {
            let slot = match key.keyword() {
                Some(":args") => &mut args,
                Some(":ret") => &mut ret,
                _ => {
                    self.error(
                        "PNX9001",
                        key.span,
                        format!("Only :args and :ret are supported in {operator}"),
                    );
                    return None;
                }
            };
            if slot.replace(value).is_some() {
                self.error("PNX2790", key.span, format!("Duplicate {operator} option"));
                return None;
            }
        }
        let (Some(args), Some(ret)) = (args, ret) else {
            self.error(
                "PNX9001",
                span,
                format!("{operator} requires both :args and :ret"),
            );
            return None;
        };
        let items = match &args.without_metadata().kind {
            Kind::List(items)
                if items.len() % 2 == 1
                    && items
                        .first()
                        .and_then(Form::symbol)
                        .is_some_and(|op| self.names.resolve(op) == "clojure.spec.alpha/cat") =>
            {
                items
            }
            _ => {
                self.error(
                    "PNX9001",
                    args.span,
                    format!("{operator} :args requires (s/cat :tag Type ...)"),
                );
                return None;
            }
        };
        let pairs = items[1..].as_chunks::<2>().0;
        let mut tags = BTreeSet::new();
        let mut fixed = Vec::new();
        let mut rest = None;
        for (index, [tag, ty]) in pairs.iter().enumerate() {
            if !tag
                .keyword()
                .is_some_and(|tag| tags.insert(self.names.key(tag)))
            {
                self.error("PNX9001", tag.span, "s/cat requires distinct keyword tags");
                return None;
            }
            if let Kind::List(repeat) = &ty.without_metadata().kind
                && repeat
                    .first()
                    .and_then(Form::symbol)
                    .is_some_and(|op| self.names.resolve(op) == "clojure.spec.alpha/*")
            {
                if index + 1 != pairs.len() || repeat.len() != 2 {
                    self.error(
                        "PNX9001",
                        ty.span,
                        "s/* requires one type and must be the last s/cat entry",
                    );
                    return None;
                }
                rest = Some(self.parse(&repeat[1]));
            } else {
                fixed.push(self.parse(ty));
            }
        }
        if fixed.len() > MAX_FIXED_PARAMETERS {
            self.error(
                "PNX2554",
                args.span,
                "Clojure/JVM supports at most 20 fixed parameters",
            );
            return None;
        }
        Some(Signature {
            args: fixed,
            rest,
            ret: self.parse(ret),
            parameters: None,
        })
    }
}
