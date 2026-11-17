//! Keyword lookup, path updates and key presence.
use std::collections::BTreeMap;

use super::{Checker, Env, Value, nil_part};
use crate::Diagnostic;
use crate::reader::{Form, Span};
use crate::types::{ExtraFields, Field, MapSlot, MapType, Type};

#[derive(Clone, Copy)]
pub(super) enum Lookup<'a> {
    Optional(Option<&'a Value>),
    Path(Option<&'a Value>),
    Required,
    Presence,
}

impl Checker<'_> {
    pub(super) fn lookup(
        &mut self,
        value: &Value,
        key: &str,
        default: Option<&Value>,
        span: Span,
    ) -> Value {
        self.lookup_value(value, key, Lookup::Optional(default), span)
    }

    pub(super) fn checked_lookup(
        &mut self,
        value: &Value,
        key: &str,
        mode: Lookup<'_>,
        span: Span,
    ) -> Value {
        let mut result = self.lookup_value(value, key, mode, span);
        if !result.known.contains_unknown() && nil_part(&result.known) == Type::Nil {
            result.notes.push("A checked lookup requires the key to exist; an explicitly present Nil remains Nil.".into());
        }
        if result.known == Type::Never && value.known != Type::Never {
            let mut diagnostic = Diagnostic::new(
                "PNX2741",
                span,
                format!("Required key {key} is absent; checked lookup would throw"),
            );
            diagnostic.notes = value.notes.clone();
            diagnostic.help = Some(format!(
                "Provide {key} in the input collection. An explicitly present Nil still satisfies the presence check."
            ));
            self.diagnostics.push(diagnostic);
        }
        result
    }

    fn lookup_value(&mut self, value: &Value, key: &str, mode: Lookup<'_>, span: Span) -> Value {
        let default = match mode {
            Lookup::Optional(default) | Lookup::Path(default) => default,
            _ => None,
        };

        let default_type = match mode {
            Lookup::Optional(default) | Lookup::Path(default) => {
                default.map_or(Type::Nil, |v| v.known.clone())
            }
            Lookup::Required | Lookup::Presence => Type::Never,
        };
        let mut notes = value.notes.clone();
        let ty = match &value.known {
            Type::Nil | Type::MapEntry(_) => default_type,
            Type::Never => Type::Never,
            Type::Unknown => Type::Unknown,
            Type::Union(xs) => {
                let values = xs
                    .iter()
                    .map(|t| self.lookup_value(&Value::new(t.clone()), key, mode, span))
                    .collect::<Vec<_>>();
                notes.extend(values.iter().flat_map(|v| v.notes.clone()));
                Type::union(values.into_iter().map(|v| v.known))
            }
            Type::Set(element) => {
                let key_type = Type::Key(key.into());
                if matches!(mode, Lookup::Path(_))
                    && !key_type.subtype_of(element)
                    && !element.contains_unknown()
                {
                    default_type
                } else if matches!(mode, Lookup::Optional(_) | Lookup::Path(_)) {
                    Type::union([key_type, default_type])
                } else if key_type.subtype_of(element) || element.contains_unknown() {
                    key_type
                } else {
                    Type::Never
                }
            }
            Type::Map(map) => match map.slot(key) {
                MapSlot::Present(ty) => ty.clone(),
                MapSlot::Optional(ty) if matches!(mode, Lookup::Required | Lookup::Presence) => {
                    ty.clone()
                }
                MapSlot::Optional(ty) => {
                    notes.push(format!(
                        "Field {key} is optional; a missing key returns {default_type}"
                    ));
                    Type::union([ty.clone(), default_type])
                }
                MapSlot::Absent => default_type,
                MapSlot::Undeclared if matches!(mode, Lookup::Presence) => Type::Unknown,
                MapSlot::Undeclared => self.undeclared_key(key, span),
            },
            _ if matches!(mode, Lookup::Path(_)) => default_type,
            _ => {
                self.error(
                    "PNX2322",
                    span,
                    format!("Keyword lookup expects a map, set or Nil, got {}", value.ty),
                );
                Type::Unknown
            }
        };
        if let Some(default) = default {
            notes.extend(default.notes.clone());
        }
        Value::with_notes(ty, notes)
    }

    fn undeclared_key(&mut self, key: &str, span: Span) -> Type {
        self.error(
            "PNX9003",
            span,
            format!("No declared type for {key} in this open map"),
        );
        Type::Unknown
    }

    pub(super) fn contains(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        let [_, target, key] = xs else {
            self.error("PNX2554", span, "contains? requires a target and key");
            return Value::new(Type::Unknown);
        };
        let value = self.infer(target, env);
        let key_value = self.infer(key, env);
        let Type::Key(key) = key_value.known else {
            return self.unsupported(key.span, "contains? currently requires a known keyword key");
        };
        Value::new(self.contains_type(&value.known, &key, target.span))
    }

    fn contains_type(&mut self, ty: &Type, key: &str, span: Span) -> Type {
        match ty {
            Type::Never | Type::Unknown => ty.clone(),
            Type::Union(parts) => {
                Type::union(parts.iter().map(|part| self.contains_type(part, key, span)))
            }
            Type::Map(_) | Type::MapEntry(_) | Type::Nil | Type::Set(_) | Type::Vector(_) => {
                key_presence(ty, key).map_or(Type::Boolean, Type::Bool)
            }
            _ => {
                self.error(
                    "PNX2322",
                    span,
                    format!(
                        "contains? expects a map, set, vector or Nil, got {}",
                        ty.inferred()
                    ),
                );
                Type::Unknown
            }
        }
    }

    fn keyword_path(&mut self, form: &Form, name: &str) -> Option<Vec<(String, Span)>> {
        if let Some(path) = form.vector()
            && let Some(keys) = path
                .iter()
                .map(|key| key.keyword().map(|name| (self.model.key(name), key.span)))
                .collect()
        {
            return Some(keys);
        }
        self.unsupported(
            form.span,
            format!("{name} currently requires a vector of literal keywords"),
        );
        None
    }

    pub(super) fn get_in(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if !(3..=4).contains(&xs.len()) {
            self.error(
                "PNX2554",
                span,
                "get-in requires a target, key path and optional default",
            );
            return Value::new(Type::Unknown);
        }
        let value = self.infer(&xs[1], env);
        let default = xs.get(3).map(|form| self.infer(form, env));
        let Some(path) = self.keyword_path(&xs[2], "get-in") else {
            return Value::new(Type::Unknown);
        };
        self.lookup_path(&value, &path, default.as_ref())
    }

    fn lookup_path(
        &mut self,
        value: &Value,
        path: &[(String, Span)],
        default: Option<&Value>,
    ) -> Value {
        let mut result = None;
        // Intermediate misses become Nil. At the last key an absent entry uses
        // the caller's fallback, while a present Nil value is preserved.
        for (index, (key, span)) in path.iter().enumerate() {
            let fallback = if index + 1 == path.len() {
                default
            } else {
                None
            };
            result = Some(self.lookup_value(
                result.as_ref().unwrap_or(value),
                key,
                Lookup::Path(fallback),
                *span,
            ));
        }
        result.unwrap_or_else(|| value.clone())
    }

    pub(super) fn update(&mut self, xs: &[Form], span: Span, env: &Env, name: &str) -> Value {
        let [_, target, key, updater, extra @ ..] = xs else {
            self.error(
                "PNX2554",
                span,
                format!("{name} requires a target, key or path, and updater"),
            );
            return Value::new(Type::Unknown);
        };
        let target_span = target.span;
        let target = self.infer(target, env);
        let path = if name.ends_with("-in") {
            let Some(path) = self.keyword_path(key, name) else {
                return Value::new(Type::Unknown);
            };
            if path.is_empty() || path.len() > 32 {
                return self.unsupported(
                    key.span,
                    "update-in currently supports paths of 1 to 32 keywords",
                );
            }
            path
        } else {
            let value = self.infer(key, env);
            let Type::Key(name) = value.known else {
                return self.unsupported(key.span, "update currently requires a known keyword key");
            };
            vec![(name, key.span)]
        };
        let (last_key, old_span) = path.last().unwrap();
        let old = self.lookup_path(&target, &path, None);
        let unknown = old.known.contains_unknown();
        let mut args = Vec::with_capacity(extra.len() + 1);
        args.push((old, *old_span));
        args.extend(extra.iter().map(|arg| (self.infer(arg, env), arg.span)));

        let start = self.diagnostics.len();
        let value = self.call_form(updater, args, span, env, true);
        for diagnostic in &mut self.diagnostics[start..] {
            if diagnostic.span == *old_span && diagnostic.code == "PNX2322" {
                diagnostic.suggestions.clear();
                diagnostic.notes.push(format!("The updater receives the old value at {last_key}, or Nil if the key is missing."));
                diagnostic.help = Some("Handle the old value in the updater or ensure the field has the required type before updating it.".into());
            }
        }
        if unknown {
            return Value::new(Type::Unknown);
        }
        Value::new(self.assoc_path(target.known, &path, &value.known, target_span, name))
    }

    pub(super) fn assoc_in(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if xs.len() != 4 {
            self.error(
                "PNX2554",
                span,
                "assoc-in requires a target, key path and value",
            );
            return Value::new(Type::Unknown);
        }
        let target = self.infer(&xs[1], env);
        let value = self.infer(&xs[3], env);
        let Some(path) = self.keyword_path(&xs[2], "assoc-in") else {
            return Value::new(Type::Unknown);
        };
        if path.is_empty() || path.len() > 32 {
            return self.unsupported(
                xs[2].span,
                "assoc-in currently supports paths of 1 to 32 keywords",
            );
        }
        Value::new(self.assoc_path(target.known, &path, &value.known, xs[1].span, "assoc-in"))
    }

    fn assoc_path(
        &mut self,
        target: Type,
        path: &[(String, Span)],
        value: &Type,
        span: Span,
        name: &str,
    ) -> Type {
        if let Type::Union(parts) = target {
            return Type::union(
                parts
                    .into_iter()
                    .map(|part| self.assoc_path(part, path, value, span, name)),
            );
        }
        let mut map = match self.assoc_map(target, span, name) {
            Ok(map) => map,
            Err(ty) => return ty,
        };
        let (key, key_span) = &path[0];
        let ty = if path.len() == 1 {
            value.clone()
        } else {
            let parent = match map.slot(key) {
                MapSlot::Present(ty) => ty.clone(),
                MapSlot::Optional(ty) => ty.clone().nilable(),
                MapSlot::Absent => Type::Nil,
                MapSlot::Undeclared => self.undeclared_key(key, *key_span),
            };
            self.assoc_path(parent, &path[1..], value, *key_span, name)
        };
        map.set_field(key.clone(), Field { ty, required: true });
        Type::Map(map)
    }

    fn assoc_map(&mut self, target: Type, span: Span, name: &str) -> Result<MapType, Type> {
        match target {
            Type::Map(map) => Ok(map),
            Type::Nil => Ok(MapType {
                fields: BTreeMap::new(),
                extra: ExtraFields::Closed,
            }),
            Type::Unknown | Type::Never => Err(target),
            _ => {
                self.error(
                    "PNX2322",
                    span,
                    format!("{name} expects a map or Nil, got {}", target.inferred()),
                );
                Err(Type::Unknown)
            }
        }
    }

    pub(super) fn assoc(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if xs.len() < 4 || !xs.len().is_multiple_of(2) {
            self.error("PNX2554", span, "assoc requires a map and key/value pairs");
            return Value::new(Type::Unknown);
        }
        let target = self.infer(&xs[1], env);
        let mut updates = Vec::new();
        for pair in xs[2..].as_chunks::<2>().0 {
            let key = self.infer(&pair[0], env);
            let value = self.infer(&pair[1], env);
            let Type::Key(key) = key.ty else {
                return self.unsupported(
                    pair[0].span,
                    "assoc currently requires literal keyword keys",
                );
            };
            updates.push((key, value.known));
        }
        let parts = match target.known {
            Type::Union(xs) => xs,
            t => vec![t],
        };
        let mut results = Vec::new();
        for part in parts {
            let mut map = match self.assoc_map(part, xs[1].span, "assoc") {
                Ok(map) => map,
                Err(ty) => {
                    results.push(ty);
                    continue;
                }
            };
            for (key, ty) in &updates {
                map.set_field(
                    key.clone(),
                    Field {
                        ty: ty.clone(),
                        required: true,
                    },
                );
            }
            results.push(Type::Map(map));
        }
        Value::new(Type::union(results))
    }
}

/// Presence is independent of the stored value's truthiness.
fn key_presence(ty: &Type, key: &str) -> Option<bool> {
    match ty {
        Type::Map(map) => match map.slot(key) {
            MapSlot::Present(_) => Some(true),
            MapSlot::Absent => Some(false),
            _ => None,
        },
        Type::Nil | Type::Vector(_) | Type::MapEntry(_) => Some(false), // A keyword is not a vector index.
        Type::Set(element)
            if !Type::Key(key.to_owned()).subtype_of(element) && !element.contains_unknown() =>
        {
            Some(false)
        }
        _ => None,
    }
}

pub(super) fn narrow_key_presence(ty: &Type, key: &str, present: bool) -> Type {
    if let Type::Union(parts) = ty {
        return Type::union(
            parts
                .iter()
                .map(|part| narrow_key_presence(part, key, present)),
        );
    }
    match (key_presence(ty, key), ty) {
        (Some(known), _) if known != present => Type::Never,
        (None, Type::Map(map)) => {
            let mut map = map.clone();
            if present {
                // An undeclared field still has no known value type on the present path.
                if let MapSlot::Optional(ty) = map.slot(key) {
                    map.set_field(
                        key.to_owned(),
                        Field {
                            ty: ty.clone(),
                            required: true,
                        },
                    );
                }
            } else if !matches!(map.extra, ExtraFields::Closed) {
                // An explicit absent field masks a possible extra entry.
                map.set_field(
                    key.to_owned(),
                    Field {
                        ty: Type::Never,
                        required: false,
                    },
                );
            } else {
                map.fields.remove(key);
            }
            Type::Map(map)
        }
        _ => ty.clone(),
    }
}
