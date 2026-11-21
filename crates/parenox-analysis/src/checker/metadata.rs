//! Metadata attached to runtime literals.
use super::{Checker, Env, Value, calls::is_builtin, model::Model};
use crate::Diagnostic;
use crate::core_names::known_core_function;
use crate::reader::{Form, Kind, Span};
use crate::types::Type;
use std::collections::BTreeSet;

const MAX_ORDERED_METADATA_KEYS: usize = 8;

#[derive(Clone, Copy)]
pub(super) enum MetadataTarget {
    Vector,
    Function,
}

enum MetadataError {
    Unsupported(Span),
    DuplicateKey { key: String, span: Span },
}

impl Checker<'_> {
    /// Evaluate metadata after the literal itself. `None` preserves its result.
    pub(super) fn literal_metadata_outcome(
        &mut self,
        form: &Form,
        env: &Env,
        mut reachable: bool,
        target: MetadataTarget,
    ) -> Option<Value> {
        if !matches!(&form.kind, Kind::Metadata(..)) {
            return None;
        }
        let (literal, unsupported) = match target {
            MetadataTarget::Vector => (
                "vector",
                "Vector metadata analysis requires up to eight literal keyword keys",
            ),
            MetadataTarget::Function => (
                "function literal",
                "Function literal metadata analysis requires up to eight literal keyword keys",
            ),
        };
        let values = match self.literal_metadata_values(form) {
            Ok(values) => values,
            Err(error) => {
                return Some(self.check_path(
                    reachable && env.reachable(),
                    |checker| match error {
                        MetadataError::Unsupported(span) => {
                            checker.withhold_captured_facts(env.inline_functions());
                            checker.withhold_literals_after_opaque_form();
                            checker.unchecked_form(span, unsupported)
                        }
                        MetadataError::DuplicateKey { key, span } => {
                            checker.error("PNX2790", span, format!("Duplicate map key {key}"));
                            Value::new(Type::Unknown)
                        }
                    },
                ));
            }
        };

        let mut unavailable = None;
        for form in values {
            let Some(mut value) = self.literal_metadata_value(form, env, reachable, literal) else {
                continue;
            };
            if value.stops_evaluation() {
                reachable = false;
                value.narrow(Type::Never);
            }
            if value.known == Type::Never || (unavailable.is_none() && value.known == Type::Unknown)
            {
                unavailable = Some(value);
            }
        }
        unavailable
    }

    fn literal_metadata_value(
        &mut self,
        form: &Form,
        env: &Env,
        reachable: bool,
        literal: &str,
    ) -> Option<Value> {
        if let Kind::Symbol(name) = &form.kind
            && !env.contains_key(name)
            && self
                .model
                .local_function_at(name, form.span.start.offset)
                .is_none()
            && self
                .model
                .declared_value(name, self.globals, form.span.start.offset)
                .is_none()
        {
            if self.model.known_java_class(name)
                || self.standard_core_name(form, env).is_some_and(|name| {
                    is_builtin(&name)
                        || name
                            .strip_prefix("clojure.core/")
                            .is_some_and(known_core_function)
                })
            {
                return None;
            }
            if Model::possible_java_class(name) {
                return Some(self.check_path(reachable && env.reachable(), |checker| {
                    checker.diagnostics.push(Diagnostic::analysis_gap(
                        "PNX9001",
                        form.span,
                        format!(
                            "JVM class resolution for {name} in {literal} metadata is unchecked"
                        ),
                    ));
                    Value::new(Type::Unknown)
                }));
            }
        }
        Some(self.infer_path(form, env, reachable, false))
    }

    fn literal_metadata_values<'form>(
        &self,
        mut form: &'form Form,
    ) -> Result<Vec<&'form Form>, MetadataError> {
        let mut wrappers = Vec::new();
        while let Kind::Metadata(metadata, target) = &form.kind {
            wrappers.push(metadata.as_ref());
            form = target;
        }

        // The reader merges the wrappers before the compiler sees the vector.
        // An outer entry replaces an earlier key in its existing position.
        let mut entries: Vec<(String, Option<&Form>)> = Vec::new();
        for metadata in wrappers.into_iter().rev() {
            let mut insert = |key, value| {
                if let Some((_, current)) = entries.iter_mut().find(|(name, _)| *name == key) {
                    *current = value;
                } else {
                    entries.push((key, value));
                }
            };
            match &metadata.kind {
                Kind::Keyword(key) => insert(self.model.key(key), None),
                Kind::String(_) => insert(":tag".into(), None),
                Kind::Symbol(_) => insert(":tag".into(), Some(metadata)),
                Kind::Map(items) => {
                    let (pairs, remainder) = items.as_chunks::<2>();
                    if !remainder.is_empty() {
                        return Err(MetadataError::Unsupported(metadata.span));
                    }
                    let mut keys = BTreeSet::new();
                    for [key, value] in pairs {
                        let Kind::Keyword(spelling) = &key.kind else {
                            return Err(MetadataError::Unsupported(key.span));
                        };
                        let resolved = self.model.key(spelling);
                        if !keys.insert(resolved.clone()) {
                            return Err(MetadataError::DuplicateKey {
                                key: resolved,
                                span: key.span,
                            });
                        }
                        insert(resolved, Some(value));
                    }
                }
                _ => return Err(MetadataError::Unsupported(metadata.span)),
            }
            if entries.len() > MAX_ORDERED_METADATA_KEYS {
                return Err(MetadataError::Unsupported(metadata.span));
            }
        }
        Ok(entries.into_iter().filter_map(|(_, value)| value).collect())
    }
}
