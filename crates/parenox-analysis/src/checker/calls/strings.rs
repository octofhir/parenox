//! Type checks for JVM string functions with type-dependent behavior.
use super::{
    Checker, Compatibility, Env, Span, Type, Value, arithmetic::excludes_number, call_returns,
};
use crate::Diagnostic;
use crate::reader::{Form, Kind};
use crate::types::MAX_TUPLE_ITEMS;

const MAX_CONCATENATED_STRING_BYTES: usize = 1024;

pub(in crate::checker) fn concatenate_literal_strings(
    parts: impl IntoIterator<Item: AsRef<str>>,
) -> Type {
    let mut text = String::new();
    for part in parts {
        let part = part.as_ref();
        if part.len() > MAX_CONCATENATED_STRING_BYTES - text.len() {
            return Type::String;
        }
        text.push_str(part);
    }
    Type::Str(text)
}

impl Checker<'_> {
    pub(super) fn str_result(&mut self, args: &[(Value, Span)]) -> Type {
        if !args
            .iter()
            .all(|(value, _)| matches!(value.known, Type::Str(_) | Type::Nil))
        {
            self.may_throw();
            return Type::String;
        }
        concatenate_literal_strings(args.iter().filter_map(|(value, _)| match &value.known {
            Type::Str(part) => Some(part),
            _ => None,
        }))
    }

    pub(super) fn substring(
        &mut self,
        forms: &[Form],
        span: Span,
        env: &Env,
        stock: bool,
    ) -> Value {
        const NAME: &str = "clojure.core/subs";
        let args = self.typed_args(forms, env);
        let returns = call_returns(&args);
        if stock && (!returns || !self.path_reachable) {
            return Value::new(Type::Never);
        }
        let diagnostic_start = self.diagnostics.len();
        if !(2..=3).contains(&args.len()) {
            self.error(
                "PNX2554",
                span,
                "subs requires a string, start and optional end",
            );
            self.note_builtin_contract(diagnostic_start, NAME);
            if stock {
                self.arity_error();
                return Value::new(Type::Never);
            }
            return Value::new(Type::Unknown);
        }
        if !returns {
            return Value::new(Type::Never);
        }
        let value = self.substring_value(&args);
        self.note_builtin_contract(diagnostic_start, NAME);
        value
    }

    fn substring_value(&mut self, args: &[(Value, Span)]) -> Value {
        self.may_throw();
        let start = self.diagnostics.len();
        let (input, input_span) = &args[0];
        if !input.is_rejected() {
            self.expect(input, &Type::String, *input_span, "subs text");
        }
        for (value, span) in &args[1..] {
            if !value.is_rejected() {
                self.expect(value, &Type::Number, *span, "subs index");
            }
        }
        // A union with a valid String or Number path can still return.
        // Bounds and result inference require operands that might be accepted.
        let impossible_operand = self.diagnostics[start..]
            .iter()
            .any(|d| d.code == "PNX2322")
            && (input.literal_container.is_some()
                || input.known.known_intersection(&Type::String) == Some(Type::Never)
                || args[1..].iter().any(|(index, _)| excludes_number(index)));
        if impossible_operand || args.iter().any(|(value, _)| value.stops_evaluation()) {
            return Value::rejected_runtime_call();
        }
        if args.iter().any(|(value, _)| value.is_rejected()) {
            return Value::rejected_call();
        }
        let text = match &input.known {
            Type::Str(text) => Some(text),
            _ => None,
        };
        let length = text.map(|text| text.encode_utf16().count());
        let mut invalid = false;
        for (value, span) in &args[1..] {
            if let Type::Integer(index) = value.known
                && (index < 0 || length.is_some_and(|len| index as usize > len))
            {
                self.error(
                    "PNX2801",
                    *span,
                    format!("Substring index {index} is out of bounds"),
                );
                invalid = true;
            }
        }
        if invalid {
            return Value::new(Type::Never);
        }
        let start = known_index(&args[1].0);
        let end = args.get(2).map_or(length, |(value, _)| known_index(value));
        if let (Some(start), Some(end)) = (start, end) {
            if end < start {
                self.error(
                    "PNX2801",
                    args.last().unwrap().1,
                    format!("Substring end {end} precedes start {start}"),
                );
                return Value::new(Type::Never);
            }
            if let Some(text) = text {
                let units: Vec<_> = text.encode_utf16().skip(start).take(end - start).collect();
                if let Ok(slice) = String::from_utf16(&units) {
                    return Value::new(Type::Str(slice));
                }
            }
        }
        Value::new(Type::String)
    }

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
        Value::new(result.nilable()).with_ordinary_tuple()
    }

    pub(super) fn case_string(&mut self, name: &str, (value, span): &(Value, Span)) -> Type {
        const NON_NIL: &str = "Non-nil value";
        if self.partial_scope.is_some() && value.known.contains_unknown() {
            return Type::String;
        }
        match non_nil_compatibility(&value.known) {
            Compatibility::Proven => {}
            Compatibility::Unproven => self.unproven_expectation(value, NON_NIL, *span, name),
            Compatibility::Mismatch => {
                let mut diagnostic =
                    Diagnostic::type_mismatch(*span, name, NON_NIL.into(), value.ty.to_string());
                diagnostic.notes = value.notes.clone();
                let remaining = value.ty.without_nil();
                if remaining != Type::Never && remaining.subtype_of(&Type::String) {
                    diagnostic.suggestions.push(self.nil_string_fallback(*span));
                } else {
                    diagnostic.help =
                        Some("Handle Nil before converting the value to a string.".into());
                }
                self.diagnostics.push(diagnostic);
            }
        }
        if value.known == Type::Nil {
            Type::Never
        } else {
            Type::String
        }
    }

    pub(super) fn blank_string(&mut self, (value, span): &(Value, Span)) -> Type {
        self.expect(
            value,
            &Type::union([Type::String, Type::Nil, Type::Bool(false)]),
            *span,
            "clojure.string/blank?",
        );
        match &value.known {
            input if input.truthy() == Type::Never => Type::Bool(true),
            Type::Str(text) if text.is_empty() => Type::Bool(true),
            _ => Type::Boolean,
        }
    }

    pub(super) fn join_string(&mut self, (value, span): &(Value, Span)) -> Type {
        let start = self.diagnostics.len();
        let returns = self.join_input(&value.known, *span);
        self.dedup_diagnostics(start);
        if returns { Type::String } else { Type::Never }
    }

    fn join_input(&mut self, ty: &Type, span: Span) -> bool {
        match ty {
            Type::Map(_) => true,
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.join_input(upper, span)
            }
            Type::Union(parts) => {
                let mut returns = false;
                for part in parts {
                    returns |= self.join_input(part, span);
                }
                returns
            }
            _ => self.seq_type(ty, "clojure.string/join", span) != Type::Never,
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

fn non_nil_compatibility(ty: &Type) -> Compatibility {
    match ty {
        Type::Nil => Compatibility::Mismatch,
        Type::Any | Type::Unknown | Type::SpecValue => Compatibility::Unproven,
        Type::Parameter(parameter) => parameter
            .upper
            .as_ref()
            .map_or(Compatibility::Mismatch, non_nil_compatibility),
        Type::Union(parts) => Compatibility::all(parts.iter().map(non_nil_compatibility)),
        _ => Compatibility::Proven,
    }
}

fn known_index(value: &Value) -> Option<usize> {
    match value.known {
        Type::Integer(index) => usize::try_from(index).ok(),
        _ => None,
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
