//! Source-mapped key paths and nested map operations.
use super::super::{Checker, Env, Value, calls::call_returns};
use super::Lookup;
use crate::reader::{Form, Span};
use crate::types::{MapSlot, MapType, Type};

type KeyPath = Vec<(Type, Span)>;

enum KeyPaths {
    Single(KeyPath),
    Alternatives(Vec<KeyPath>),
}

impl KeyPaths {
    fn as_slice(&self) -> &[KeyPath] {
        match self {
            Self::Single(path) => std::slice::from_ref(path),
            Self::Alternatives(paths) => paths,
        }
    }
}

impl Checker<'_> {
    fn key_paths(
        &mut self,
        form: &Form,
        env: &Env,
        name: &str,
        writing: bool,
    ) -> Result<KeyPaths, Type> {
        if let Some(keys) = form.vector() {
            let mut path = Vec::with_capacity(keys.len());
            let mut reachable = true;
            for form in keys {
                let key = self.inspect_path(reachable, |checker| {
                    checker
                        .literal_map_key(form)
                        .or_else(|| checker.read_map_key(form, env, name, !writing))
                });
                if reachable {
                    match key {
                        Some(Type::Never) => reachable = false,
                        Some(ty) if !ty.contains_unknown() => path.push((ty, form.span)),
                        _ => {}
                    }
                }
            }
            if !reachable {
                return Err(Type::Never);
            }
            self.check_path_length(keys.len(), form.span, name, writing)?;
            return if path.len() == keys.len() {
                Ok(KeyPaths::Single(path))
            } else {
                Err(Type::Unknown)
            };
        }
        let inferred = self.infer(form, env).known;
        if let Type::Union(parts) = inferred {
            let alternatives = parts
                .into_iter()
                .map(|part| self.path_from_type(part, form.span, name, writing))
                .collect::<Result<_, _>>()?;
            Ok(KeyPaths::Alternatives(alternatives))
        } else {
            self.path_from_type(inferred, form.span, name, writing)
                .map(KeyPaths::Single)
        }
    }

    fn path_from_type(
        &mut self,
        ty: Type,
        span: Span,
        name: &str,
        writing: bool,
    ) -> Result<KeyPath, Type> {
        let items: Box<[Type]> = match ty {
            Type::Tuple(items) => items,
            Type::MapEntry(items) => items,
            Type::Vector(element) if *element == Type::Never => Box::new([]),
            Type::Unknown | Type::Never => return Err(ty),
            _ => {
                self.unsupported(
                    span,
                    format!("{name} requires a vector path with known positions"),
                );
                return Err(Type::Unknown);
            }
        };
        self.check_path_length(items.len(), span, name, writing)?;
        items
            .into_vec()
            .into_iter()
            .enumerate()
            .map(|(index, key)| {
                if key.contains_unknown() || key == Type::Never {
                    return Err(if key == Type::Never {
                        Type::Never
                    } else {
                        Type::Unknown
                    });
                }
                if !key.is_lookup_key(!writing) {
                    self.unsupported(span, format!(
                    "{name} analysis supports keyword and string keys; path item {} has type {}",
                    index + 1,
                    key.inferred(),
                ));
                    return Err(Type::Unknown);
                }
                Ok((key, span))
            })
            .collect()
    }

    fn check_path_length(
        &mut self,
        length: usize,
        span: Span,
        name: &str,
        writing: bool,
    ) -> Result<(), Type> {
        if writing && !(1..=32).contains(&length) {
            self.unsupported(
                span,
                format!("{name} currently supports paths of 1 to 32 keys"),
            );
            Err(Type::Unknown)
        } else {
            Ok(())
        }
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
        let path = self.key_paths(&xs[2], env, "get-in", false);
        let default = xs.get(3).map(|form| self.infer(form, env));
        match path {
            Ok(path) => {
                let presence = xs[2]
                    .vector()
                    .and_then(|keys| keys.first())
                    .and_then(|key| env.presence(&xs[1], key));
                self.lookup_paths(&value, &path, default.as_ref(), presence)
            }
            Err(ty) => Value::new(ty),
        }
    }

    fn lookup_paths(
        &mut self,
        value: &Value,
        paths: &KeyPaths,
        default: Option<&Value>,
        presence: Option<bool>,
    ) -> Value {
        match paths {
            KeyPaths::Single(path) => self.lookup_path(value, path, default, presence),
            KeyPaths::Alternatives(paths) => {
                let mut notes = Vec::new();
                let types = paths.iter().map(|path| {
                    let result = self.lookup_path(value, path, default, presence);
                    for note in result.notes {
                        if !notes.contains(&note) {
                            notes.push(note);
                        }
                    }
                    result.known
                });
                Value::with_notes(Type::union(types), notes)
            }
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
        let target_returns = target.known != Type::Never;
        let path = self.inspect_path(target_returns, |checker| {
            if nested {
                checker.key_paths(key, env, name, true)
            } else {
                match checker.read_map_key(key, env, name, false) {
                    Some(ty) if !matches!(ty, Type::Never | Type::Unknown) => {
                        Ok(KeyPaths::Single(vec![(ty, key.span)]))
                    }
                    Some(ty) => Err(ty),
                    None => Err(Type::Unknown),
                }
            }
        });
        let key_returns = target_returns && !matches!(path, Err(Type::Never));
        let callable = self.inspect_path(key_returns, |checker| {
            checker.prepare_callable(updater, env)
        });
        let ready = key_returns && callable.returns();
        let extra: Vec<_> = self.eval_args(extra, env, ready).collect();
        if !ready || !call_returns(&extra) {
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }
        let path = match path {
            Ok(path) => path,
            Err(ty) => {
                self.call_prepared(&callable, Vec::new(), span, env, false);
                return Value::new(ty);
            }
        };
        let (last_key, old_span) = path.as_slice()[0].last().unwrap();
        let old = self.lookup_paths(&target, &path, None, presence);
        let unknown = old.known.contains_unknown();
        let mut args = Vec::with_capacity(extra.len() + 1);
        args.push((old, *old_span));
        args.extend(extra);

        let start = self.diagnostics.len();
        let value = self.call_prepared(&callable, args, span, env, true);
        for diagnostic in &mut self.diagnostics[start..] {
            if diagnostic.span == *old_span && diagnostic.code == "PNX2322" {
                diagnostic.suggestions.clear();
                let key = if (!nested || key.vector().is_some()) && last_key.is_literal_map_key() {
                    last_key.to_string()
                } else {
                    self.source[old_span.start.offset..old_span.end.offset].to_owned()
                };
                diagnostic.notes.push(format!(
                    "The updater receives the old value at {key}, or Nil if the entry is missing."
                ));
                diagnostic.help = Some("Handle the old value in the updater or ensure the field has the required type before updating it.".into());
            }
        }
        if value.known == Type::Never {
            return value;
        }
        self.may_throw();
        if unknown {
            return Value::new(Type::Unknown);
        }
        Value::new(self.assoc_paths(
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
        let path = self.key_paths(&xs[2], env, "assoc-in", true);
        let value = self.infer(&xs[3], env);
        let path = match path {
            Ok(path) => path,
            Err(ty) => return Value::new(ty),
        };
        Value::new(self.assoc_paths(
            target.known,
            &path,
            &value.known,
            xs[1].span,
            "assoc-in",
            presence,
        ))
    }

    fn assoc_paths(
        &mut self,
        target: Type,
        paths: &KeyPaths,
        value: &Type,
        span: Span,
        name: &str,
        presence: Option<bool>,
    ) -> Type {
        match paths {
            KeyPaths::Single(path) => self.assoc_path(target, path, value, span, name, presence),
            KeyPaths::Alternatives(paths) => Type::union(
                paths
                    .iter()
                    .map(|path| self.assoc_path(target.clone(), path, value, span, name, presence)),
            ),
        }
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
