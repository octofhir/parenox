//! Eager contextual calls that transform map keys or values.
use std::collections::{BTreeMap, btree_map::Entry};

use super::super::{Checker, Env, Value, calls::PreparedCallable};
use super::MAX_MAP_CALLBACK_CHECKS;
use crate::reader::{Form, Span};
use crate::types::{ExtraFields, Field, MapType, Type};

const MAX_RENAMED_KEYS: usize = 128;

struct MapUpdate<'a, 'form, 'model> {
    callable: &'a PreparedCallable<'form, 'model>,
    field_spans: BTreeMap<Type, Span>,
    target: Span,
    updater: Span,
    span: Span,
    name: &'a str,
    env: &'a Env,
    notes: &'a [String],
}

impl Checker<'_> {
    pub(in crate::checker) fn map_transform(
        &mut self,
        forms: &[Form],
        span: Span,
        env: &Env,
        name: &str,
    ) -> Value {
        let name = name
            .strip_prefix("clojure.core/")
            .expect("resolved core name");
        let [target, updater] = forms else {
            self.error(
                "PNX2554",
                span,
                format!("{name} expects a map and an updater"),
            );
            return Value::new(Type::Unknown);
        };
        let input = self.infer(target, env);
        let target_returns = input.known != Type::Never;
        let callable = self.inspect_path(target_returns, |checker| {
            checker.prepare_callable(updater, env)
        });
        if !target_returns || !callable.returns() {
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }

        let field_spans = target
            .map()
            .unwrap_or_default()
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[key, value]| {
                let span = if name == "update-keys" {
                    key.span
                } else {
                    value.span
                };
                Some((self.literal_map_key(key)?, span))
            })
            .collect();
        let update = MapUpdate {
            callable: &callable,
            field_spans,
            target: target.span,
            updater: updater.span,
            span,
            name,
            env,
            notes: &input.notes,
        };
        let start = self.diagnostics.len();
        let result = self.transform_map_type(input.known, &update);
        self.dedup_diagnostics(start);
        Value::with_notes(result, input.notes)
    }

    fn transform_map_type(&mut self, ty: Type, update: &MapUpdate<'_, '_, '_>) -> Type {
        match ty {
            Type::Nil => {
                self.call_prepared(update.callable, Vec::new(), update.span, update.env, false);
                Type::Map(MapType {
                    fields: BTreeMap::new(),
                    extra: ExtraFields::Closed,
                })
            }
            Type::Map(ref map) if map.extra == ExtraFields::Open => {
                self.unsupported(
                    update.target,
                    format!(
                        "{} cannot infer {} types for an open map",
                        update.name,
                        if update.name == "update-keys" {
                            "key"
                        } else {
                            "value"
                        },
                    ),
                )
                .known
            }
            Type::Map(map) if update.name == "update-keys" => self.update_map_keys(map, update),
            Type::Map(map) => self.update_map_values(map, update),
            Type::Union(parts) => Type::union(
                parts
                    .into_iter()
                    .map(|part| self.transform_map_type(part, update)),
            ),
            Type::Parameter(ref parameter) if let Some(upper) = &parameter.upper => {
                self.transform_map_type(upper.clone(), update)
            }
            Type::Unknown => {
                self.may_throw();
                Type::Unknown
            }
            _ => {
                self.unsupported(
                    update.target,
                    format!("{} analysis currently supports Map or Nil", update.name),
                )
                .known
            }
        }
    }

    fn update_map_values(&mut self, mut map: MapType, update: &MapUpdate<'_, '_, '_>) -> Type {
        let mut invoked = false;
        let mut returns = true;
        for (key, field) in &mut map.fields {
            if field.ty == Type::Never {
                continue;
            }
            let old = std::mem::replace(&mut field.ty, Type::Never);
            let Some(value) = self.call_map_updater(update, Some(key), old) else {
                return Type::Unknown;
            };
            invoked = true;
            returns &= !field.required || value != Type::Never;
            field.ty = value;
        }
        if let ExtraFields::Typed(pair) = &mut map.extra {
            let old = std::mem::replace(&mut pair[1], Type::Never);
            let Some(value) = self.call_map_updater(update, None, old) else {
                return Type::Unknown;
            };
            invoked = true;
            if value == Type::Never {
                // A normal result is possible only with no extra entries.
                map.extra = ExtraFields::Closed;
            } else {
                pair[1] = value;
            }
        }
        if !invoked {
            self.call_prepared(update.callable, Vec::new(), update.span, update.env, false);
        }
        if !returns {
            return Type::Never;
        }
        if matches!(map.extra, ExtraFields::Closed) {
            map.fields
                .retain(|_, field| field.required || field.ty != Type::Never);
        }
        Type::Map(map)
    }

    fn update_map_keys(&mut self, map: MapType, update: &MapUpdate<'_, '_, '_>) -> Type {
        let MapType { mut fields, extra } = map;
        if let ExtraFields::Typed(pair) = extra {
            let [keys, value] = *pair;
            let Some(choices) = literal_key_choices(&keys) else {
                return self
                    .unsupported(
                        update.target,
                        "update-keys requires a finite set of possible keyword or string keys",
                    )
                    .known;
            };
            if fields.len().saturating_add(choices.len()) > MAX_RENAMED_KEYS {
                return self.unsupported(update.target, format!(
                    "update-keys has too many possible input keys (limit {MAX_RENAMED_KEYS})",
                )).known;
            }
            for key in choices {
                fields.entry(key.clone()).or_insert_with(|| Field {
                    ty: value.clone(),
                    required: false,
                });
            }
        }
        let mut sources: BTreeMap<Type, (Type, Field)> = BTreeMap::new();
        let mut invoked = false;
        let mut returns = true;
        for (key, mut field) in fields {
            if field.ty == Type::Never {
                continue;
            }
            let start = self.diagnostics.len();
            let Some(updated) = self.call_map_updater(update, Some(&key), key.clone()) else {
                return Type::Unknown;
            };
            invoked = true;
            if updated == Type::Never {
                returns &= !field.required;
                continue;
            }
            let Some(choices) = literal_key_choices(&updated) else {
                if self.diagnostics.len() == start {
                    self.unsupported(
                        update.updater,
                        format!(
                            "update-keys cannot determine the new keyword or string key for {key}",
                        ),
                    );
                }
                return Type::Unknown;
            };
            if sources.len().saturating_add(choices.len()) > MAX_RENAMED_KEYS {
                return self.unsupported(update.updater, format!(
                    "update-keys has too many possible result keys (limit {MAX_RENAMED_KEYS})",
                )).known;
            }
            field.required &= choices.len() == 1;
            for destination in choices {
                match sources.entry(destination.clone()) {
                    Entry::Vacant(slot) => {
                        slot.insert((key.clone(), field.clone()));
                    }
                    Entry::Occupied(slot) => {
                        let (previous, previous_field) = slot.get();
                        if previous_field.required && field.required {
                            self.error(
                                "PNX2790",
                                update.updater,
                                format!(
                                    "update-keys maps both {previous} and {key} to {destination}"
                                ),
                            );
                        } else {
                            self.unsupported(update.updater, format!(
                                "update-keys may map both {previous} and {key} to {destination}",
                            ));
                        }
                        return Type::Unknown;
                    }
                }
            }
        }
        if !invoked {
            self.call_prepared(update.callable, Vec::new(), update.span, update.env, false);
        }
        if !returns {
            return Type::Never;
        }
        Type::Map(MapType {
            fields: sources
                .into_iter()
                .map(|(key, (_, field))| (key, field))
                .collect(),
            extra: ExtraFields::Closed,
        })
    }

    fn call_map_updater(
        &mut self,
        update: &MapUpdate<'_, '_, '_>,
        key: Option<&Type>,
        old: Type,
    ) -> Option<Type> {
        let updating_keys = update.name == "update-keys";
        if !self.take_map_callback_step() {
            let part = if updating_keys { "key" } else { "value" };
            self.unsupported(
                update.span,
                format!(
                    "{} {part} analysis limit ({MAX_MAP_CALLBACK_CHECKS}) exceeded",
                    update.name
                ),
            );
            return None;
        }
        let value_span = key
            .and_then(|key| update.field_spans.get(key))
            .copied()
            .unwrap_or(update.target);
        let args = vec![(Value::with_notes(old, update.notes.to_vec()), value_span)];
        let start = self.diagnostics.len();
        let result = self.call_prepared(update.callable, args, update.span, update.env, true);
        for diagnostic in &mut self.diagnostics[start..] {
            if diagnostic.code == "PNX2322" && diagnostic.span == update.target {
                diagnostic.suggestions.clear();
            }
            diagnostic.notes.push(key.map_or_else(
                || "While updating dictionary values".into(),
                |key| {
                    if updating_keys {
                        format!("While updating the key {key}")
                    } else {
                        format!("While updating the value at {key}")
                    }
                },
            ));
        }
        Some(result.known)
    }
}

fn literal_key_choices(ty: &Type) -> Option<&[Type]> {
    match ty {
        Type::Union(choices) if choices.iter().all(Type::is_literal_map_key) => Some(choices),
        ty if ty.is_literal_map_key() => Some(std::slice::from_ref(ty)),
        _ => None,
    }
}
