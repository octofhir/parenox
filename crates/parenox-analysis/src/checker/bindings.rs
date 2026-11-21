//! Lexical bindings and destructuring.
mod positions;
use std::collections::BTreeMap;

use super::env::Selector;
use super::maps::Lookup;
use super::{Checker, Env, Value};
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

pub(super) fn dotted_binding_error(name: &str, span: Span) -> Option<Diagnostic> {
    (!name.contains('/') && name.contains('.')).then(|| {
        Diagnostic::new(
            "PNX1001",
            span,
            format!("Clojure/JVM local binding {name} cannot contain a dot"),
        )
    })
}

pub(super) fn dotted_pattern_error(form: &Form) -> Option<Diagnostic> {
    let target = form.without_metadata();
    if let Some(name) = target.symbol() {
        return dotted_binding_error(name, target.span);
    }
    if !matches!(target.kind, Kind::Map(_) | Kind::Vector(_)) {
        return None;
    }
    let mut names = Vec::new();
    binding_names(target, &mut names);
    names
        .into_iter()
        .find_map(|(name, span)| dotted_binding_error(name, span))
}

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
    let pairs = entries.as_chunks::<2>().0;
    if let Some([_, alias]) = pairs.iter().find(|[kind, _]| kind.keyword() == Some(":as")) {
        binding_names(alias, names);
    }
    for [binding, value] in pairs {
        match binding.keyword() {
            Some(":all" | ":excess" | ":missing" | ":select") => {
                binding_names(value, names);
            }
            Some(":as" | ":or" | ":defaults") => {}
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
    stopped: bool,
}

#[derive(Clone, Copy)]
struct MapBinding<'a> {
    value: &'a Value,
    defaults: &'a Defaults,
    source: Option<&'a Selector>,
}

fn is_saved_map(ty: &Type) -> bool {
    match ty {
        Type::Map(_) | Type::Nil => true,
        Type::Union(types) => types.iter().all(is_saved_map),
        _ => false,
    }
}

fn may_convert_sequence(ty: &Type) -> bool {
    match ty {
        Type::Seq(_)
        | Type::List(_)
        | Type::SeqTuple(_)
        | Type::ListTuple(_)
        | Type::SpecCollection(_) => true,
        Type::Parameter(parameter) => parameter.upper.as_ref().is_none_or(may_convert_sequence),
        Type::Union(types) => types.iter().any(may_convert_sequence),
        _ => false,
    }
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
        self.bind_selected(form, value, env, None, None);
    }

    pub(super) fn bind_initializer(
        &mut self,
        form: &Form,
        init: &Form,
        value: Value,
        env: &mut Env,
    ) {
        let source = self.binding_selector(form, init, env);
        self.bind_selected(form, value, env, source, None);
    }

    pub(super) fn let_binding(
        &mut self,
        pattern: &Form,
        init: &Form,
        env: &mut Env,
        reachable: bool,
    ) -> bool {
        // A direct map binding in Clojure 1.13 evaluates its defaults before
        // its initializer. Call arguments and conditional tests may already
        // reside in a temporary when their binding patterns are evaluated.
        let defaults = pattern
            .map()
            .map(|entries| self.map_defaults(entries.as_chunks::<2>().0, env, reachable));
        let reachable = reachable && defaults.as_ref().is_none_or(|defaults| !defaults.stopped);
        let value = self.infer_path(init, env, reachable, false);
        let returns = !value.stops_evaluation();
        self.inspect_path(returns, |checker| {
            let source = checker.binding_selector(pattern, init, env);
            checker.bind_selected(pattern, value, env, source, defaults);
        });
        returns && env.reachable()
    }

    fn binding_selector(&self, form: &Form, init: &Form, env: &Env) -> Option<Selector> {
        (form.symbol().is_some() || form.map().is_some())
            .then(|| self.selector(init, env))
            .flatten()
    }

    fn bind_selected(
        &mut self,
        form: &Form,
        value: Value,
        env: &mut Env,
        source: Option<Selector>,
        defaults: Option<Defaults>,
    ) {
        let target = form.without_metadata();
        match &target.kind {
            Kind::Symbol(name) | Kind::AnonymousArgument(name) => {
                self.bind_local(name, target.span, value, source, env);
                // Class selection and non-nil evidence are independent.
                if let Some(hint) = form.metadata(":tag").and_then(Form::symbol) {
                    env.set_java_hint(name, hint);
                }
            }
            Kind::Vector(xs) => {
                self.may_throw();
                self.bind_positions(xs, form.span, value, env);
            }
            Kind::Map(xs) => {
                self.bind_map(
                    xs.as_chunks::<2>().0,
                    form.span,
                    value,
                    source,
                    defaults,
                    env,
                );
            }
            _ => self.error("PNX9001", form.span, "Unsupported binding pattern"),
        }
    }

    fn bind_local(
        &mut self,
        name: &str,
        span: Span,
        value: Value,
        source: Option<Selector>,
        env: &mut Env,
    ) {
        if let Some(diagnostic) = dotted_binding_error(name, span) {
            self.diagnostics.push(diagnostic);
            // Keep the value for the remaining checks and drop editor facts
            // from the body whose binding the compiler will reject.
            self.type_query = None;
        } else if env.reachable() {
            self.observe_binding(name, span, &value);
        }
        if value.known == Type::Never {
            env.mark_impossible();
        }
        env.insert_selected(name.into(), value, source);
    }

    fn bind_map(
        &mut self,
        pairs: &[[Form; 2]],
        span: Span,
        value: Value,
        source: Option<Selector>,
        defaults: Option<Defaults>,
        env: &mut Env,
    ) {
        // :missing changes checked lookups into collection of absent keys.
        if pairs.iter().any(|p| p[0].keyword() == Some(":missing")) {
            self.error(
                "PNX9001",
                span,
                "Destructuring with :missing is not supported yet",
            );
            return;
        }
        let defaults = defaults.unwrap_or_else(|| self.map_defaults(pairs, env, env.reachable()));
        if defaults.stopped {
            env.mark_impossible();
        }
        self.inspect_path(env.reachable(), |checker| {
            checker.bind_map_fields(pairs, span, value, source, &defaults, env);
        });
    }

    fn bind_map_fields(
        &mut self,
        pairs: &[[Form; 2]],
        span: Span,
        value: Value,
        source: Option<Selector>,
        defaults: &Defaults,
        env: &mut Env,
    ) {
        self.may_throw();
        // Clojure can convert a sequence into a map while destructuring it.
        // Map and nil inputs retain the same value throughout the pattern.
        let value = if may_convert_sequence(&value.known) {
            self.unsupported(span, "Map destructuring of sequence inputs is not modeled")
        } else {
            value
        };
        let source = is_saved_map(&value.known)
            .then(|| source.unwrap_or_else(|| env.captured_selector(value.clone())));
        let input = MapBinding {
            value: &value,
            defaults,
            source: source.as_ref(),
        };
        env.with_owner_pinned(input.source, |env| {
            if let Some([_, alias]) = pairs
                .iter()
                .find(|[kind, alias]| kind.keyword() == Some(":as") && alias.symbol().is_some())
            {
                self.bind_selected(alias, value.clone(), env, source.clone(), None);
            }
            for pair in pairs {
                let directive = pair[0].keyword().map(|key| self.model.key(key));
                match directive.as_deref() {
                    Some(":or") => {}
                    Some(":as") if pair[1].symbol().is_some() => {}
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
                            self.bind_entry(entry, input, true, env);
                        }
                    }
                    Some(directive)
                        if matches!(directive, ":strs" | ":strs!")
                            || matches!(
                                directive.rsplit('/').next(),
                                Some(":keys" | ":keys!" | "keys" | "keys!")
                            ) =>
                    {
                        self.bind_keys(directive, &pair[1], input, env);
                    }
                    _ => self.bind_entry(pair, input, false, env),
                }
            }
        });
    }

    fn bind_entry(
        &mut self,
        pair: &[Form; 2],
        input: MapBinding<'_>,
        required: bool,
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
        let (field, source) = self.binding_value(input, key, pair[0].symbol(), mode, pair[1].span);
        self.bind_selected(&pair[0], field, env, source, None);
    }

    fn bind_keys(&mut self, directive: &str, form: &Form, input: MapBinding<'_>, env: &mut Env) {
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
                let (field, source) =
                    self.binding_value(input, key_type, Some(local), mode, key.span);
                self.bind_local(local, key.without_metadata().span, field, source, env);
            } else if required {
                self.binding_value(input, key_type, None, Lookup::Presence, key.span);
            }
        }
    }

    fn map_defaults(&mut self, pairs: &[[Form; 2]], env: &Env, reachable: bool) -> Defaults {
        let mut defaults = Defaults {
            stopped: !reachable,
            ..Defaults::default()
        };
        let Some([_, form]) = pairs.iter().find(|[key, _]| key.keyword() == Some(":or")) else {
            return defaults;
        };
        let Some(entries) = form.map() else {
            self.error(
                "PNX1001",
                form.span,
                ":or requires a map of locals or literal keys to defaults",
            );
            return defaults;
        };
        let pairs = entries.as_chunks::<2>().0;
        // Alpha11 visits an array map's entries in order. Larger maps may
        // visit any default first, while one definite failure still stops
        // the whole binding before the initializer.
        let ordered = pairs.len() <= 8;
        for pair in pairs {
            let value = self.infer_path(
                &pair[1],
                env,
                reachable && (!ordered || !defaults.stopped),
                false,
            );
            defaults.stopped |= value.stops_evaluation();
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
        input: MapBinding<'_>,
        key: Type,
        local: Option<&str>,
        mode: Lookup<'_>,
        span: Span,
    ) -> (Value, Option<Selector>) {
        let by_local = local.and_then(|name| input.defaults.locals.get(name));
        let by_key = input.defaults.keys.get(&key);
        if let (Some(_), Some((_, span))) = (by_local, by_key) {
            self.error(
                "PNX1001",
                *span,
                format!(
                    "Both local {} and key {key} have :or defaults; choose one",
                    local.unwrap()
                ),
            );
            return (Value::new(Type::Unknown), None);
        }
        let default = by_local.or(by_key);
        let source = if default.is_none() && !matches!(mode, Lookup::Presence) {
            input.source.and_then(|source| source.field(key.clone()))
        } else {
            None
        };
        let field = match mode {
            Lookup::Optional(_) | Lookup::Path(_) => {
                self.lookup(input.value, &key, default.map(|(value, _)| value), span)
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
                    self.checked_lookup(input.value, &key, mode, span)
                }
            }
        };
        (field, source)
    }
}
