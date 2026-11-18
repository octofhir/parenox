//! Map lookup, path updates and key presence.
mod keys;
mod merge;
mod paths;
use std::collections::BTreeMap;

use super::{Checker, Env, Value, calls::call_returns, nil_part};
use crate::Diagnostic;
use crate::reader::{Form, Span};
use crate::types::{ExtraFields, Field, IndexedItem, MapSlot, MapType, Type};

#[derive(Clone, Copy)]
pub(super) enum Lookup<'a> {
    Optional(Option<&'a Value>),
    Path(Option<&'a Value>),
    Required,
    Presence,
}

impl Checker<'_> {
    pub(super) fn map_literal(
        &mut self,
        xs: &[Form],
        mut evaluate: impl FnMut(&mut Self, &Form) -> Value,
    ) -> Value {
        let mut fields = BTreeMap::new();
        let mut returns = true;
        for pair in xs.as_chunks::<2>().0 {
            let value = evaluate(self, &pair[1]);
            returns &= value.known != Type::Never;
            if let Some(key) = self.literal_map_key(&pair[0]) {
                if fields
                    .insert(
                        key.clone(),
                        Field {
                            ty: value.known,
                            required: true,
                        },
                    )
                    .is_some()
                {
                    self.error("PNX2790", pair[0].span, format!("Duplicate map key {key}"));
                }
            } else {
                self.unsupported(
                    pair[0].span,
                    "Map literals currently require keyword or string keys",
                );
            }
        }
        if !returns {
            return Value::new(Type::Never);
        }
        Value::new(Type::Map(MapType {
            fields,
            extra: ExtraFields::Closed,
        }))
    }

    pub(super) fn lookup(
        &mut self,
        value: &Value,
        key: &Type,
        default: Option<&Value>,
        span: Span,
    ) -> Value {
        self.lookup_value(value, key, Lookup::Optional(default), span)
    }

    pub(super) fn lookup_default(
        &mut self,
        value: &Value,
        key: &Type,
        default: Option<&Value>,
        span: Span,
    ) -> Value {
        self.lookup_value(value, key, Lookup::Path(default), span)
    }

    pub(super) fn checked_lookup(
        &mut self,
        value: &Value,
        key: &Type,
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

    fn lookup_value(&mut self, value: &Value, key: &Type, mode: Lookup<'_>, span: Span) -> Value {
        match key {
            Type::Unknown | Type::Never => return Value::new(key.clone()),
            Type::Union(parts) => {
                let values = parts
                    .iter()
                    .map(|key| self.lookup_value(value, key, mode, span));
                return values.fold(Value::new(Type::Never), Value::join);
            }
            _ => {}
        }
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
            indexed
                if let Type::Integer(index) = key
                    && let Some(item) = indexed.indexed_item(*index) =>
            {
                match item {
                    IndexedItem::Present(item) => item.clone(),
                    IndexedItem::Optional(item)
                        if matches!(mode, Lookup::Required | Lookup::Presence) =>
                    {
                        item.clone()
                    }
                    IndexedItem::Optional(item) => Type::union([item.clone(), default_type]),
                    IndexedItem::Absent(_) => default_type,
                }
            }
            Type::Nil
            | Type::MapEntry(_)
            | Type::Vector(_)
            | Type::Tuple(_)
            | Type::String
            | Type::Str(_) => default_type,
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
                let key_type = key.clone();
                if matches!(mode, Lookup::Path(_))
                    && !key_type.key_overlaps(element)
                    && !element.contains_unknown()
                {
                    default_type
                } else if matches!(mode, Lookup::Optional(_) | Lookup::Path(_)) {
                    Type::union([key_type, default_type])
                } else if key_type.key_overlaps(element) || element.contains_unknown() {
                    key_type
                } else {
                    Type::Never
                }
            }
            Type::Map(map) if !key.is_literal_map_key() => {
                match map.possible_values(key) {
                    Some(Type::Never) => default_type,
                    Some(ty) if matches!(mode, Lookup::Required | Lookup::Presence) => ty,
                    Some(ty) => {
                        notes.push(format!("A key of type {key} may be absent; a missing key returns {default_type}"));
                        Type::union([ty, default_type])
                    }
                    None if matches!(mode, Lookup::Presence) => Type::Unknown,
                    None => self.undeclared_key(key, span),
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
            _ if matches!(mode, Lookup::Path(_) | Lookup::Required) => default_type,
            _ => {
                self.error(
                    "PNX2322",
                    span,
                    format!("Lookup expects a map, set or Nil, got {}", value.ty),
                );
                Type::Unknown
            }
        };
        if let Some(default) = default {
            notes.extend(default.notes.clone());
        }
        Value::with_notes(ty, notes)
    }

    fn undeclared_key(&mut self, key: &Type, span: Span) -> Type {
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
        let args: Vec<_> = self.eval_args(&xs[1..], env, true).collect();
        if !call_returns(&args) {
            return Value::new(Type::Never);
        }
        let mut args = args.into_iter();
        let (value, _) = args.next().unwrap();
        let (key_value, key_span) = args.next().unwrap();
        let Some(key_type) = self.validate_map_key(key_value, key_span, "contains?", true) else {
            self.may_throw();
            return Value::new(Type::Unknown);
        };
        Value::new(match env.presence(target, key) {
            Some(present) => Type::Bool(present),
            None => self.contains_type(&value.known, &key_type, target.span),
        })
    }

    fn contains_type(&mut self, ty: &Type, key: &Type, span: Span) -> Type {
        match key {
            Type::Unknown => {
                self.may_throw();
                return Type::Unknown;
            }
            Type::Never => return Type::Never,
            Type::Union(parts) => {
                return Type::union(parts.iter().map(|key| self.contains_type(ty, key, span)));
            }
            _ => {}
        }
        match ty {
            Type::Unknown => {
                self.may_throw();
                Type::Unknown
            }
            Type::Never => Type::Never,
            Type::Union(parts) => {
                Type::union(parts.iter().map(|part| self.contains_type(part, key, span)))
            }
            Type::Map(_)
            | Type::MapEntry(_)
            | Type::Nil
            | Type::Set(_)
            | Type::Vector(_)
            | Type::Tuple(_) => key_presence(ty, key).map_or(Type::Boolean, Type::Bool),
            Type::String | Type::Str(_) if matches!(key, Type::Integer(_)) => {
                key_presence(ty, key).map_or(Type::Boolean, Type::Bool)
            }
            _ => {
                self.illegal_argument();
                self.error(
                    "PNX2322",
                    span,
                    format!(
                        "contains? expects a map, set, vector, string with an integer index or Nil, got {}",
                        ty.inferred()
                    ),
                );
                Type::Never
            }
        }
    }

    fn assoc_map(&mut self, target: Type, span: Span, name: &str) -> Result<MapType, Type> {
        match target {
            Type::Map(map) => Ok(map),
            Type::Nil => Ok(MapType {
                fields: BTreeMap::new(),
                extra: ExtraFields::Closed,
            }),
            Type::Never => Err(Type::Never),
            Type::Unknown => {
                self.may_throw();
                Err(Type::Unknown)
            }
            Type::Tuple(_) | Type::Vector(_) | Type::MapEntry(_) => {
                self.may_throw();
                Err(self
                    .unsupported(span, format!("{name} does not yet analyze vector writes"))
                    .known)
            }
            _ => {
                self.class_cast();
                self.diagnostics.push(Diagnostic::type_mismatch(
                    span,
                    name,
                    "Map | Nil".into(),
                    target.inferred().to_string(),
                ));
                Err(Type::Never)
            }
        }
    }

    pub(super) fn assoc(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if xs.len() < 4 || !xs.len().is_multiple_of(2) {
            self.error("PNX2554", span, "assoc requires a map and key/value pairs");
            return Value::new(Type::Unknown);
        }
        let args: Vec<_> = self.eval_args(&xs[1..], env, true).collect();
        if !call_returns(&args) {
            return Value::new(Type::Never);
        }
        let mut updates = Vec::new();
        for [(key, key_span), (value, _)] in args[1..].as_chunks::<2>().0 {
            let key = self.validate_map_key(key.clone(), *key_span, "assoc", false);
            let Some(key) = key.filter(|key| !key.contains_unknown()) else {
                self.may_throw();
                return Value::new(Type::Unknown);
            };
            updates.push((key, &value.known));
        }
        let parts = match &args[0].0.known {
            Type::Union(xs) => xs.clone(),
            t => vec![t.clone()],
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
                map.associate(key, ty);
            }
            results.push(Type::Map(map));
        }
        Value::new(Type::union(results))
    }
}

/// Presence is independent of the stored value's truthiness.
fn key_presence(ty: &Type, key: &Type) -> Option<bool> {
    match ty {
        Type::Map(map) if !key.is_literal_map_key() => {
            (map.possible_values(key) == Some(Type::Never)).then_some(false)
        }
        Type::Map(map) => match map.slot(key) {
            MapSlot::Present(_) => Some(true),
            MapSlot::Absent => Some(false),
            _ => None,
        },
        Type::Nil => Some(false),
        Type::Vector(_) | Type::Tuple(_) | Type::MapEntry(_) | Type::String | Type::Str(_) => {
            let Type::Integer(index) = key else {
                return Some(false);
            };
            match ty.indexed_item(*index) {
                Some(IndexedItem::Present(_)) => Some(true),
                Some(IndexedItem::Absent(_)) => Some(false),
                _ => None,
            }
        }
        Type::Set(element) if !key.key_overlaps(element) && !element.contains_unknown() => {
            Some(false)
        }
        _ => None,
    }
}

pub(super) fn narrow_key_presence(ty: &Type, key: &Type, present: bool) -> Type {
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
