//! Source-mapped key paths and nested map operations.
use super::super::{Checker, Env, Value};
use super::Lookup;
use crate::reader::{Form, Span};
use crate::types::{MapSlot, MapType, Type};

type KeyPath = Vec<(Type, Span)>;

impl Checker<'_> {
    fn key_path(
        &mut self,
        form: &Form,
        env: &Env,
        name: &str,
        writing: bool,
    ) -> Result<KeyPath, Type> {
        let Some(keys) = form.vector() else {
            self.unsupported(
                form.span,
                format!("{name} currently requires a vector expression as its key path"),
            );
            return Err(Type::Unknown);
        };
        if writing && !(1..=32).contains(&keys.len()) {
            self.unsupported(
                form.span,
                format!("{name} currently supports paths of 1 to 32 keys"),
            );
            return Err(Type::Unknown);
        }
        keys.iter()
            .map(|form| {
                let key = self
                    .literal_map_key(form)
                    .or_else(|| self.read_map_key(form, env, name));
                match key {
                    Some(ty) if !ty.contains_unknown() && ty != Type::Never => Ok((ty, form.span)),
                    Some(Type::Never) => Err(Type::Never),
                    _ => Err(Type::Unknown),
                }
            })
            .collect()
    }

    pub(in crate::checker) fn get_in(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if !(3..=4).contains(&xs.len()) {
            self.error(
                "PNX2554",
                span,
                "get-in requires a target, key path and optional default",
            );
            return Value::new(Type::Unknown);
        }
        let value = self.infer(&xs[1], env);
        let path = self.key_path(&xs[2], env, "get-in", false);
        let default = xs.get(3).map(|form| self.infer(form, env));
        match path {
            Ok(path) => {
                let presence = xs[2]
                    .vector()
                    .and_then(|keys| keys.first())
                    .and_then(|key| env.presence(&xs[1], key));
                self.lookup_path(&value, &path, default.as_ref(), presence)
            }
            Err(ty) => Value::new(ty),
        }
    }

    fn lookup_path(
        &mut self,
        value: &Value,
        path: &[(Type, Span)],
        default: Option<&Value>,
        presence: Option<bool>,
    ) -> Value {
        if presence == Some(false) {
            return default.cloned().unwrap_or_else(|| Value::new(Type::Nil));
        }
        let mut result = None;
        for (index, (key, span)) in path.iter().enumerate() {
            let mode = if index == 0 && presence == Some(true) {
                Lookup::Required
            } else if index + 1 == path.len() {
                Lookup::Path(default)
            } else {
                Lookup::Path(None)
            };
            result = Some(self.lookup_value(result.as_ref().unwrap_or(value), key, mode, *span));
        }
        result.unwrap_or_else(|| value.clone())
    }

    pub(in crate::checker) fn update(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        name: &str,
    ) -> Value {
        let [_, target, key, updater, extra @ ..] = xs else {
            self.error(
                "PNX2554",
                span,
                format!("{name} requires a target, key or path, and updater"),
            );
            return Value::new(Type::Unknown);
        };
        let target_span = target.span;
        let nested = name == "clojure.core/update-in";
        let presence = if nested {
            key.vector()
                .and_then(|keys| keys.first())
                .and_then(|key| env.presence(target, key))
        } else {
            env.presence(target, key)
        };
        let target = self.infer(target, env);
        let path = if nested {
            match self.key_path(key, env, name, true) {
                Ok(path) => path,
                Err(ty) => return Value::new(ty),
            }
        } else {
            let Some(key_type) = self.read_map_key(key, env, name) else {
                return Value::new(Type::Unknown);
            };
            if matches!(key_type, Type::Never | Type::Unknown) {
                return Value::new(key_type);
            }
            vec![(key_type, key.span)]
        };
        let (last_key, old_span) = path.last().unwrap();
        let old = self.lookup_path(&target, &path, None, presence);
        let unknown = old.known.contains_unknown();
        let mut args = Vec::with_capacity(extra.len() + 1);
        args.push((old, *old_span));
        args.extend(extra.iter().map(|arg| (self.infer(arg, env), arg.span)));

        let start = self.diagnostics.len();
        let value = self.call_form(updater, args, span, env, true);
        for diagnostic in &mut self.diagnostics[start..] {
            if diagnostic.span == *old_span && diagnostic.code == "PNX2322" {
                diagnostic.suggestions.clear();
                let key = if last_key.is_literal_map_key() {
                    last_key.to_string()
                } else {
                    self.source[old_span.start.offset..old_span.end.offset].to_owned()
                };
                diagnostic.notes.push(format!(
                    "The updater receives the old value at {key}, or Nil if the key is missing."
                ));
                diagnostic.help = Some("Handle the old value in the updater or ensure the field has the required type before updating it.".into());
            }
        }
        if unknown {
            return Value::new(Type::Unknown);
        }
        Value::new(self.assoc_path(
            target.known,
            &path,
            &value.known,
            target_span,
            name,
            presence,
        ))
    }

    pub(in crate::checker) fn assoc_in(&mut self, xs: &[Form], span: Span, env: &Env) -> Value {
        if xs.len() != 4 {
            self.error(
                "PNX2554",
                span,
                "assoc-in requires a target, key path and value",
            );
            return Value::new(Type::Unknown);
        }
        let presence = xs[2]
            .vector()
            .and_then(|keys| keys.first())
            .and_then(|key| env.presence(&xs[1], key));
        let target = self.infer(&xs[1], env);
        let path = self.key_path(&xs[2], env, "assoc-in", true);
        let value = self.infer(&xs[3], env);
        let path = match path {
            Ok(path) => path,
            Err(ty) => return Value::new(ty),
        };
        Value::new(self.assoc_path(
            target.known,
            &path,
            &value.known,
            xs[1].span,
            "assoc-in",
            presence,
        ))
    }

    fn assoc_path(
        &mut self,
        target: Type,
        path: &[(Type, Span)],
        value: &Type,
        span: Span,
        name: &str,
        presence: Option<bool>,
    ) -> Type {
        if let Type::Union(parts) = target {
            return Type::union(
                parts
                    .into_iter()
                    .map(|part| self.assoc_path(part, path, value, span, name, presence)),
            );
        }
        if presence == Some(true) && target == Type::Nil {
            return Type::Never;
        }
        let mut map = match self.assoc_map(target, span, name) {
            Ok(map) => map,
            Err(ty) => return ty,
        };
        let (key, key_span) = &path[0];
        let ty = if path.len() == 1 {
            value.clone()
        } else {
            let parent = self.assoc_parent(&map, key, *key_span, presence);
            self.assoc_path(parent, &path[1..], value, *key_span, name, None)
        };
        if ty == Type::Never {
            return Type::Never;
        }
        map.associate(key, &ty);
        Type::Map(map)
    }

    fn assoc_parent(
        &mut self,
        map: &MapType,
        key: &Type,
        span: Span,
        presence: Option<bool>,
    ) -> Type {
        if presence == Some(false) {
            return Type::Nil;
        }
        let value = if key.is_literal_map_key() {
            match map.slot(key) {
                MapSlot::Present(ty) => return ty.clone(),
                MapSlot::Optional(ty) => Some(ty.clone()),
                MapSlot::Absent => Some(Type::Never),
                MapSlot::Undeclared => None,
            }
        } else {
            map.possible_values(key)
        };
        match value {
            Some(ty) if presence == Some(true) => ty,
            Some(Type::Never) => Type::Nil,
            Some(ty) => ty.nilable(),
            None => self.undeclared_key(key, span),
        }
    }
}
