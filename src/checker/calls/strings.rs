//! Type checks for JVM string functions with type-dependent behavior.
use super::{Checker, Env, Span, Type, Value, call_returns};
use crate::Diagnostic;
use crate::reader::{Form, Kind};
use crate::types::MAX_TUPLE_ITEMS;

impl Checker<'_> {
    pub(super) fn regex_match(
        &mut self,
        forms: &[Form],
        span: Span,
        env: &Env,
        name: &str,
    ) -> Value {
        if name == "clojure.core/re-find" && forms.len() == 1 {
            return self.unchecked_form(
                span,
                "The stateful Matcher overload of re-find is unchecked",
            );
        }
        let args = self.typed_args(forms, env);
        let start = self.diagnostics.len();
        let [(pattern, pattern_span), (text, text_span)] = args.as_slice() else {
            self.error(
                "PNX2554",
                span,
                format!("{name} requires a pattern and text"),
            );
            self.note_builtin_contract(start, name);
            return Value::new(Type::Unknown);
        };
        if !call_returns(&args) {
            return Value::new(Type::Never);
        }
        self.may_throw();
        self.expect(
            pattern,
            &Type::Pattern,
            *pattern_span,
            &format!("Argument 1 of {name}"),
        );
        self.expect(
            text,
            &Type::String,
            *text_span,
            &format!("Argument 2 of {name}"),
        );
        self.note_builtin_contract(start, name);

        let literal = forms[0].without_metadata();
        let captures = matches!(literal.kind, Kind::Regex)
            .then(|| &self.source[literal.span.start.offset..literal.span.end.offset])
            .and_then(literal_capture_count);
        let groups = || Type::Vector(Box::new(Type::String.nilable()));
        let result = match captures {
            Some(0) => Type::String,
            Some(count) if count < MAX_TUPLE_ITEMS => Type::Tuple(
                std::iter::once(Type::String)
                    .chain(std::iter::repeat_n(Type::String.nilable(), count))
                    .collect(),
            ),
            Some(_) => groups(),
            None => Type::union([Type::String, groups()]),
        };
        Value::new(result.nilable())
    }

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

fn literal_capture_count(pattern: &str) -> Option<usize> {
    if !pattern.contains('(') {
        return Some(0);
    }
    // Classes, quoted regex segments, and inline group syntax need the JVM dialect.
    if pattern.contains('[') || pattern.contains("\\Q") || pattern.contains("(?") {
        return None;
    }
    let mut chars = pattern.chars();
    let mut count = 0;
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                chars.next();
            }
            '(' => count += 1,
            _ => {}
        }
    }
    Some(count)
}
