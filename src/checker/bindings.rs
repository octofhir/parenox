//! Lexical bindings and destructuring.
mod positions;
use std::collections::BTreeMap;

use super::maps::Lookup;
use super::{Checker, Env, Value};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

/// Collect the names introduced by a binding without evaluating defaults or selectors.
pub(super) fn binding_names<'a>(form: &'a Form, names: &mut Vec<(&'a str, Span)>) {
    let form = form.without_metadata();
    match &form.kind {
        Kind::Symbol(name) | Kind::AnonymousArgument(name) if name != "&" => {
            names.push((name, form.span));
        }
        Kind::Vector(items) => {
            for item in items {
                binding_names(item, names);
            }
        }
        Kind::Map(entries) => map_binding_names(entries, names),
        _ => {}
    }
}

fn map_binding_names<'a>(entries: &'a [Form], names: &mut Vec<(&'a str, Span)>) {
    for [binding, value] in entries.as_chunks::<2>().0 {
        match binding.keyword() {
            Some(":as" | ":all" | ":excess" | ":missing" | ":select") => {
                binding_names(value, names);
            }
            Some(":or" | ":defaults") => {}
            Some(":req!") => {
                if let Some(required) = value.map() {
                    map_binding_names(required, names);
                }
            }
            Some(_) => {
                if let Some(items) = value.vector() {
                    for item in items.iter().take_while(|item| item.symbol() != Some("&")) {
                        map_binding_name(item, names);
                    }
                }
            }
            None => map_binding_name(binding, names),
        }
    }
}

fn map_binding_name<'a>(form: &'a Form, names: &mut Vec<(&'a str, Span)>) {
    if let Some(name) = form.symbol().or_else(|| form.keyword()) {
        let local = name.trim_start_matches(':').rsplit('/').next().unwrap();
        names.push((local, form.without_metadata().span));
    } else {
        binding_names(form, names);
    }
}

#[derive(Default)]
struct Defaults {
    locals: BTreeMap<String, (Value, Span)>,
    keys: BTreeMap<Type, (Value, Span)>,
}

impl Checker<'_> {
    pub(super) fn bind_parameters(
        &mut self,
        mut env: Env,
        params: &[Form],
        value: impl Fn(usize) -> Value,
    ) -> Env {
        // All ordinary arguments exist before fn destructuring, including later ones.
        // Destructuring then proceeds in parameter order and can shadow those names.
        for (index, param) in params
            .iter()
            .enumerate()
            .filter(|(_, p)| p.symbol().is_some())
        {
            self.bind(param, value(index), &mut env);
        }
        for (index, param) in params
            .iter()
            .enumerate()
            .filter(|(_, p)| p.symbol().is_none())
        {
            self.bind(param, value(index), &mut env);
        }
        env
    }

    pub(super) fn bind(&mut self, form: &Form, value: Value, env: &mut Env) {
        match &form.without_metadata().kind {
            Kind::Symbol(name) | Kind::AnonymousArgument(name) => {
                env.insert(name.clone(), value);
            }
            Kind::Vector(xs) => {
                self.may_throw();
                self.bind_positions(xs, form.span, value, env);
            }
            Kind::Map(xs) => {
                self.may_throw();
                // :missing changes checked lookups into collection of absent keys.
                // Do not apply throwing semantics before this mode is implemented.
                if xs
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|p| p[0].keyword() == Some(":missing"))
                {
                    self.error(
                        "PNX9001",
                        form.span,
                        "Destructuring with :missing is not supported yet",
                    );
                    return;
                }
                // Clojure evaluates every default once in the outer lexical scope,
                // before introducing this map's locals and :as alias.
                let defaults = xs
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .find(|p| p[0].keyword() == Some(":or"))
                    .map(|p| self.binding_defaults(&p[1], env))
                    .unwrap_or_default();
                for pair in xs.as_chunks::<2>().0 {
                    let directive = pair[0].keyword().map(|key| self.model.key(key));
                    match directive.as_deref() {
                        Some(":or") => {}
                        Some(":as") if pair[1].symbol().is_some() => {
                            env.insert(pair[1].symbol().unwrap().into(), value.clone());
                        }
                        Some(":req!") => {
                            let Some(entries) = pair[1].map() else {
                                self.error(
                                    "PNX9001",
                                    pair[1].span,
                                    ":req! requires a map of bindings to keys",
                                );
                                continue;
                            };
                            for entry in entries.as_chunks::<2>().0 {
                                self.bind_entry(entry, &value, true, &defaults, env);
                            }
                        }
                        Some(directive)
                            if matches!(directive, ":strs" | ":strs!")
                                || matches!(
                                    directive.rsplit('/').next(),
                                    Some(":keys" | ":keys!" | "keys" | "keys!")
                                ) =>
                        {
                            self.bind_keys(directive, &pair[1], &value, &defaults, env);
                        }
                        _ => self.bind_entry(pair, &value, false, &defaults, env),
                    }
                }
            }
            _ => self.error("PNX9001", form.span, "Unsupported binding pattern"),
        }
    }

    fn bind_entry(
        &mut self,
        pair: &[Form; 2],
        value: &Value,
        required: bool,
        defaults: &Defaults,
        env: &mut Env,
    ) {
        if !matches!(
            pair[0].without_metadata().kind,
            Kind::Symbol(_) | Kind::AnonymousArgument(_) | Kind::Map(_) | Kind::Vector(_)
        ) {
            self.error(
                "PNX9001",
                pair[0].span,
                "Unsupported destructuring directive",
            );
            return;
        }
        let Some(key) = self.literal_map_key(&pair[1]) else {
            self.error(
                "PNX9001",
                pair[1].span,
                "Destructuring currently requires literal keyword or string keys",
            );
            return;
        };
        let mode = if required {
            Lookup::Required
        } else {
            Lookup::Optional(None)
        };
        let field = self.binding_value(value, key, pair[0].symbol(), mode, defaults, pair[1].span);
        self.bind(&pair[0], field, env);
    }

    fn bind_keys(
        &mut self,
        directive: &str,
        form: &Form,
        value: &Value,
        defaults: &Defaults,
        env: &mut Env,
    ) {
        let Some(keys) = form.vector() else {
            self.error(
                "PNX9001",
                form.span,
                format!("{directive} requires a vector"),
            );
            return;
        };
        let namespace = directive.rsplit_once('/').map(|(ns, _)| ns);
        let required = directive.ends_with('!');
        let string_keys = matches!(directive, ":strs" | ":strs!");
        let mut presence_only = false;
        for key in keys {
            if key.symbol() == Some("&") {
                if presence_only {
                    self.error("PNX9001", key.span, "A keys binding can contain only one &");
                }
                presence_only = true;
                continue;
            }
            let (local, mut lookup_key) = if presence_only {
                let lookup_key = if string_keys {
                    match &key.without_metadata().kind {
                        Kind::String(key) => Some(key.clone()),
                        _ => None,
                    }
                } else {
                    key.keyword().map(|name| self.model.key(name))
                };
                let Some(lookup_key) = lookup_key else {
                    let kind = if string_keys { "strings" } else { "keywords" };
                    self.error(
                        "PNX9001",
                        key.span,
                        format!("Keys after & must be literal {kind}"),
                    );
                    continue;
                };
                (None, lookup_key)
            } else {
                let Some(name) = key.symbol() else {
                    self.error(
                        "PNX9001",
                        key.span,
                        format!("{directive} requires symbols before &"),
                    );
                    continue;
                };
                if matches!(key.without_metadata().kind, Kind::AnonymousArgument(_)) {
                    self.error(
                        "PNX9001",
                        key.span,
                        "Key inference for a generated #() argument name is not supported",
                    );
                    env.insert(name.into(), Value::new(Type::Unknown));
                    continue;
                }
                if namespace.is_some() && name.contains('/') {
                    self.error(
                        "PNX9001",
                        key.span,
                        "Namespaced keys directives require unqualified binding symbols",
                    );
                    continue;
                }
                if string_keys && name.contains('/') {
                    self.error(
                        "PNX9001",
                        key.span,
                        ":strs bindings require unqualified symbols",
                    );
                    continue;
                }
                if name.starts_with('/') || name.ends_with('/') || name.matches('/').count() > 1 {
                    self.error("PNX9001", key.span, "Invalid symbol in keys binding");
                    continue;
                }
                let local = name.rsplit('/').next().unwrap();

                (
                    Some(local),
                    if string_keys {
                        name.to_owned()
                    } else {
                        format!(":{name}")
                    },
                )
            };
            if let Some(ns) = namespace
                && !lookup_key.contains('/')
            {
                lookup_key = format!("{ns}/{}", lookup_key.trim_start_matches(':'));
            }
            let key_type = if string_keys {
                Type::Str(lookup_key)
            } else {
                Type::Key(lookup_key)
            };
            if let Some(local) = local {
                let mode = if required {
                    Lookup::Required
                } else {
                    Lookup::Optional(None)
                };
                let field =
                    self.binding_value(value, key_type, Some(local), mode, defaults, key.span);
                env.insert(local.into(), field);
            } else if required {
                self.binding_value(value, key_type, None, Lookup::Presence, defaults, key.span);
            }
        }
    }

    fn binding_defaults(&mut self, form: &Form, env: &Env) -> Defaults {
        let mut defaults = Defaults::default();
        let Some(entries) = form.map() else {
            self.error(
                "PNX1001",
                form.span,
                ":or requires a map of locals or literal keys to defaults",
            );
            return defaults;
        };
        for pair in entries.as_chunks::<2>().0 {
            let value = self.infer(&pair[1], env);
            let target = pair[0].span;
            let duplicate = if let Some(name) = pair[0].symbol() {
                defaults
                    .locals
                    .insert(name.to_owned(), (value, target))
                    .is_some()
            } else if let Some(key) = self.literal_map_key(&pair[0]) {
                defaults.keys.insert(key, (value, target)).is_some()
            } else {
                self.error(
                    "PNX9001",
                    pair[0].span,
                    "Defaults require local symbols, keywords or strings",
                );
                continue;
            };
            if duplicate {
                let key = &self.source[target.start.offset..target.end.offset];
                self.error(
                    "PNX2790",
                    pair[0].span,
                    format!("Duplicate :or default {key}"),
                );
            }
        }
        defaults
    }

    fn binding_value(
        &mut self,
        value: &Value,
        key: Type,
        local: Option<&str>,
        mode: Lookup<'_>,
        defaults: &Defaults,
        span: Span,
    ) -> Value {
        let by_local = local.and_then(|name| defaults.locals.get(name));
        let by_key = defaults.keys.get(&key);
        if let (Some(_), Some((_, span))) = (by_local, by_key) {
            self.error(
                "PNX1001",
                *span,
                format!(
                    "Both local {} and key {key} have :or defaults; choose one",
                    local.unwrap()
                ),
            );
            return Value::new(Type::Unknown);
        }
        let default = by_local.or(by_key);
        match mode {
            Lookup::Optional(_) | Lookup::Path(_) => {
                self.lookup(value, &key, default.map(|(value, _)| value), span)
            }
            Lookup::Required | Lookup::Presence => {
                if let Some((_, span)) = default {
                    self.error(
                        "PNX1001",
                        *span,
                        format!("Required key {key} cannot have an :or default"),
                    );
                    Value::new(Type::Unknown)
                } else {
                    self.checked_lookup(value, &key, mode, span)
                }
            }
        }
    }
}
