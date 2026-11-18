//! Type checks for JVM string functions with type-dependent behavior.
use super::{Checker, Span, Type, Value};
use crate::Diagnostic;

impl Checker<'_> {
    pub(super) fn blank_string(&mut self, (value, span): &(Value, Span)) -> Type {
        self.expect(
            value,
            &Type::String.nilable(),
            *span,
            "clojure.string/blank?",
        );
        match &value.known {
            Type::Nil => Type::Bool(true),
            Type::Str(text) if text.is_empty() => Type::Bool(true),
            _ => Type::Boolean,
        }
    }

    pub(super) fn replace_string(&mut self, args: &[(Value, Span)], span: Span) -> Type {
        let [
            (text, text_span),
            (pattern, pattern_span),
            (replacement, replacement_span),
        ] = args
        else {
            unreachable!("replace arity is checked before dispatch");
        };
        self.expect(
            text,
            &Type::String,
            *text_span,
            "Argument 1 of clojure.string/replace",
        );
        if pattern.known.subtype_of(&Type::Character) {
            self.expect(
                replacement,
                &Type::Character,
                *replacement_span,
                "Character replacement",
            );
        } else if pattern.known.subtype_of(&Type::String) {
            self.expect(
                replacement,
                &Type::String,
                *replacement_span,
                "String replacement",
            );
        } else {
            let text_patterns = Type::union([Type::String, Type::Pattern]);
            if pattern.known.subtype_of(&text_patterns) {
                if !replacement.known.subtype_of(&Type::String) {
                    self.diagnostics.push(Diagnostic::analysis_gap(
                        "PNX9001",
                        *replacement_span,
                        "Regex replacement callback types are unavailable",
                    ));
                }
            } else {
                let supported = Type::union([Type::Character, text_patterns]);
                if pattern.known.subtype_of(&supported) {
                    self.diagnostics.push(Diagnostic::analysis_gap(
                        "PNX9001",
                        span,
                        "Replacement overloads require a known match type",
                    ));
                } else {
                    self.expect(
                        pattern,
                        &supported,
                        *pattern_span,
                        "Argument 2 of clojure.string/replace",
                    );
                }
            }
        }
        Type::String
    }
}
