//! Lexical bindings and destructuring.
mod positions;
use std::borrow::Cow;
use std::collections::BTreeMap;

use super::env::Selector;
use super::maps::{Lookup, literal_map_in_source_order};
use super::metadata::java_type_hint;
use super::{Checker, Env, Value};
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

pub(super) fn raw_local_symbol(form: &Form) -> Option<&str> {
    form.symbol()
        .filter(|name| *name != "&" && (*name == "/" || !name.contains('/')))
}

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

fn shadows_missing_local(pairs: &[[Form; 2]], local: &str) -> bool {
    let as_binding = pairs
        .iter()
        .any(|[kind, alias]| kind.keyword() == Some(":as") && alias.symbol() == Some(local));
    let mut names = Vec::new();
    map_binding_names(pairs.as_flattened(), &mut names);
    // The :as binding runs before the accumulator is initialized.
    let allowed = 1 + usize::from(as_binding);
    names
        .iter()
        .filter(|(name, _)| *name == local)
        .take(allowed + 1)
        .count()
        > allowed
}

pub(super) fn map_defaults_form(pairs: &[[Form; 2]]) -> Option<&Form> {
    pairs
        .iter()
        .find_map(|[key, form]| (key.keyword() == Some(":or")).then_some(form))
}

fn map_binding_name<'a>(form: &'a Form, names: &mut Vec<(&'a str, Span)>) {
    if let Some(name) = form.symbol().or_else(|| form.keyword()) {
        names.push((map_local_name(name), form.without_metadata().span));
    } else {
        binding_names(form, names);
    }
}

fn map_local_name(name: &str) -> &str {
    let name = name.trim_start_matches(':');
    if name == "/" {
        name
    } else {
        name.rsplit('/').next().unwrap()
    }
}

#[derive(Default)]
struct Defaults {
    locals: BTreeMap<String, (Value, Span)>,
    keys: BTreeMap<Type, (Value, Span)>,
    stopped: bool,
}

#[derive(Default)]
struct ProjectionKeys<'form> {
    keys: Vec<Type>,
    locals: Option<BTreeMap<&'form str, Option<usize>>>,
    required: Vec<Type>,
    select: Option<BTreeMap<Type, NestedProjection<'form>>>,
    all: Option<BTreeMap<Type, NestedProjection<'form>>>,
    missing: Option<BTreeMap<Type, NestedProjection<'form>>>,
    excess: Option<BTreeMap<Type, NestedProjection<'form>>>,
}

enum NestedProjection<'form> {
    Generated(Box<Value>),
    Binding(&'form str),
}

#[derive(Default)]
struct NestedProjections<'form> {
    select: Option<NestedProjection<'form>>,
    all: Option<NestedProjection<'form>>,
    missing: Option<NestedProjection<'form>>,
    excess: Option<NestedProjection<'form>>,
}

#[derive(Clone, Copy, Default)]
struct InheritedProjections {
    select: bool,
    all: bool,
    missing: bool,
    excess: bool,
}

impl InheritedProjections {
    fn any(self) -> bool {
        self.select || self.all || self.missing || self.excess
    }
}

fn resolve_nested_projections(
    children: BTreeMap<Type, NestedProjection<'_>>,
    env: &Env,
) -> Option<Vec<(Type, Value)>> {
    children
        .into_iter()
        .map(|(key, child)| {
            let value = match child {
                NestedProjection::Generated(value) => *value,
                NestedProjection::Binding(local) => env.get(local)?.clone(),
            };
            Some((key, value))
        })
        .collect()
}

enum ProjectionDefaultError {
    AlreadyReported,
    Invalid(Span, &'static str),
    Ambiguous(Span),
}

fn nested_projection_input<'input>(
    base: Option<&'input Value>,
    children: Option<BTreeMap<Type, NestedProjection<'_>>>,
    env: &Env,
) -> Option<Cow<'input, Value>> {
    let base = base?;
    if base.known == Type::Unknown {
        return Some(Cow::Borrowed(base));
    }
    let children = resolve_nested_projections(children?, env)?;
    if children.is_empty() {
        Some(Cow::Borrowed(base))
    } else {
        base.clone()
            .merge_destructured_children(children)
            .map(Cow::Owned)
    }
}

impl<'form> ProjectionKeys<'form> {
    fn push(&mut self, local: Option<&'form str>, key: &Type, required: bool) {
        self.require(key, required);
        if self.select.is_none() && self.all.is_none() && self.excess.is_none() {
            return;
        }
        let index = self.keys.len();
        self.keys.push(key.clone());
        if let (Some(locals), Some(local)) = (&mut self.locals, local) {
            let recorded = locals.entry(local).or_insert(Some(index));
            if recorded.is_some_and(|old| self.keys[old] != *key) {
                *recorded = None;
            }
        }
    }

    fn require(&mut self, key: &Type, required: bool) {
        if required && self.missing.is_some() {
            self.required.push(key.clone());
        }
    }

    fn collects_children(&self) -> bool {
        self.select.is_some()
            || self.all.is_some()
            || self.excess.is_some()
            || self.missing.is_some()
    }

    fn record_child(&mut self, key: &Type, child: NestedProjections<'form>) {
        for (state, value) in [
            (&mut self.select, child.select),
            (&mut self.all, child.all),
            (&mut self.excess, child.excess),
            (&mut self.missing, child.missing),
        ] {
            if let Some(children) = state {
                if let Some(value) = value {
                    children.insert(key.clone(), value);
                } else {
                    *state = None;
                }
            }
        }
    }

    fn defaults<'value>(
        &self,
        defaults: &'value Defaults,
    ) -> Result<BTreeMap<Type, &'value Value>, ProjectionDefaultError> {
        let mut fields = BTreeMap::new();
        for (local, (value, span)) in &defaults.locals {
            let index = self
                .locals
                .as_ref()
                .and_then(|locals| locals.get(local.as_str()))
                .ok_or(ProjectionDefaultError::Invalid(
                    *span,
                    "Default for a projection must refer to a selected binding",
                ))?
                .ok_or(ProjectionDefaultError::Ambiguous(*span))?;
            if fields.insert(self.keys[index].clone(), value).is_some() {
                return Err(ProjectionDefaultError::Invalid(
                    *span,
                    "Multiple defaults select the same map key",
                ));
            }
        }
        for (key, (value, span)) in &defaults.keys {
            if !self.keys.contains(key) {
                return Err(ProjectionDefaultError::Invalid(
                    *span,
                    "Default for a projection must refer to a selected key",
                ));
            }
            if fields.insert(key.clone(), value).is_some() {
                // The binding's local/key conflict was checked in binding_value.
                return Err(ProjectionDefaultError::AlreadyReported);
            }
        }
        Ok(fields)
    }
}

#[derive(Clone, Copy)]
struct MapBinding<'a> {
    value: &'a Value,
    defaults: &'a Defaults,
    source: Option<&'a Selector>,
    collect: InheritedProjections,
    missing_local: Option<&'a str>,
    source_order: bool,
}

impl MapBinding<'_> {
    fn advance_missing(self, env: &mut Env) {
        if let Some(local) = self.missing_local {
            // A nested default can read the partially collected map.
            env.insert(local.into(), Value::new(Type::Unknown));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShorthandKey {
    Keyword,
    Symbol,
    String,
}

#[derive(Clone, Copy)]
struct ShorthandBinding<'a> {
    namespace: Option<&'a str>,
    key: ShorthandKey,
    required: bool,
}

impl<'a> ShorthandBinding<'a> {
    fn parse(directive: &'a str) -> Option<Self> {
        let directive = directive.strip_prefix(':')?;
        let (namespace, operator) = directive
            .rsplit_once('/')
            .map_or((None, directive), |(namespace, name)| {
                (Some(namespace), name)
            });
        let required = operator.ends_with('!');
        let key = match operator.strip_suffix('!').unwrap_or(operator) {
            "keys" => ShorthandKey::Keyword,
            "syms" => ShorthandKey::Symbol,
            "strs" if namespace.is_none() => ShorthandKey::String,
            _ => return None,
        };
        Some(Self {
            namespace,
            key,
            required,
        })
    }

    fn key_for_symbol(self, name: &str) -> Type {
        match self.key {
            ShorthandKey::Keyword => Type::Key(match self.namespace {
                Some(namespace) => format!(":{namespace}/{name}"),
                None => format!(":{name}"),
            }),
            ShorthandKey::Symbol => Type::Sym(match self.namespace {
                Some(namespace) => format!("{namespace}/{name}"),
                None => name.into(),
            }),
            ShorthandKey::String => Type::Str(name.into()),
        }
    }
}

fn is_saved_map(ty: &Type) -> bool {
    match ty {
        Type::Map(_) | Type::Nil => true,
        Type::Union(types) => types.iter().all(is_saved_map),
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
            // Read the initializer's class selection in the previous scope.
            let inherited_hint = pattern
                .symbol()
                .filter(|_| pattern.metadata(":tag").is_none())
                .and_then(|name| {
                    env.java_hint(init)
                        .map(|hint| (name, hint.map(str::to_owned)))
                });
            checker.bind_selected(pattern, value, env, source, defaults);
            if let Some((name, hint)) = inherited_hint {
                env.set_java_hint(name, hint);
            }
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
                if let Some(hint) = java_type_hint(form) {
                    env.set_java_hint(name, hint.map(str::to_owned));
                }
            }
            Kind::Vector(xs) => {
                self.may_throw();
                self.bind_positions(xs, form.span, value, env);
            }
            Kind::Map(_) => {
                self.bind_map(
                    form,
                    value,
                    source,
                    defaults,
                    env,
                    InheritedProjections::default(),
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

    fn bind_map<'form>(
        &mut self,
        form: &'form Form,
        value: Value,
        source: Option<Selector>,
        defaults: Option<Defaults>,
        env: &mut Env,
        inherited: InheritedProjections,
    ) -> Option<NestedProjections<'form>> {
        let pairs = form.map().unwrap().as_chunks::<2>().0;
        let defaults = defaults.unwrap_or_else(|| self.map_defaults(pairs, env, env.reachable()));
        if defaults.stopped {
            env.mark_impossible();
        }
        self.inspect_path(env.reachable(), |checker| {
            checker.bind_map_fields((pairs, form.span), value, source, &defaults, env, inherited)
        })
    }

    fn bind_map_fields<'form>(
        &mut self,
        (pairs, span): (&'form [[Form; 2]], Span),
        value: Value,
        mut source: Option<Selector>,
        defaults: &Defaults,
        env: &mut Env,
        inherited: InheritedProjections,
    ) -> Option<NestedProjections<'form>> {
        self.may_throw();
        // Clojure can convert a sequence into a map while destructuring it.
        // Map and nil inputs retain the same value throughout the pattern.
        let value = if value.may_convert_for_map_binding() {
            // The saved value is the conversion, not the original initializer.
            source = None;
            value.destructuring_sequence().unwrap_or_else(|| {
                self.unsupported(
                    span,
                    "Map destructuring requires fixed sequence positions, supported keys, and a supported trailing collection",
                )
            })
        } else {
            value
        };
        let source = is_saved_map(&value.known)
            .then(|| source.unwrap_or_else(|| env.captured_selector(value.clone())));
        let (mut selection, mut all, mut excess, mut missing) = (None, None, None, None);
        for [key, binding] in pairs {
            match key.keyword() {
                Some(":select") => selection = Some(binding),
                Some(":all") => all = Some(binding),
                Some(":excess") => excess = Some(binding),
                Some(":missing") => missing = Some(binding),
                _ => {}
            }
        }
        let missing_local = missing.and_then(raw_local_symbol);
        let shadowed_missing =
            missing_local.is_some_and(|local| shadows_missing_local(pairs, local));
        let input = MapBinding {
            value: &value,
            defaults,
            source: source.as_ref(),
            collect: InheritedProjections {
                select: inherited.select || selection.is_some(),
                all: inherited.all || all.is_some(),
                missing: inherited.missing || missing.is_some(),
                excess: inherited.excess || excess.is_some(),
            },
            missing_local,
            source_order: literal_map_in_source_order(pairs),
        };
        let mut selected = input.collect.any().then(|| ProjectionKeys {
            keys: Vec::new(),
            locals: ((input.collect.select || input.collect.all) && !defaults.locals.is_empty())
                .then(BTreeMap::new),
            required: Vec::new(),
            select: input.collect.select.then(BTreeMap::new),
            all: input.collect.all.then(BTreeMap::new),
            missing: (input.collect.missing && !shadowed_missing).then(BTreeMap::new),
            excess: input.collect.excess.then(BTreeMap::new),
        });
        env.with_owner_pinned(input.source, |env| {
            if let Some([_, alias]) = pairs
                .iter()
                .find(|[kind, alias]| kind.keyword() == Some(":as") && alias.symbol().is_some())
            {
                self.bind_selected(alias, value.clone(), env, source.clone(), None);
            }
            if let Some(local) = missing_local {
                let initial = if input.source_order {
                    Type::Nil
                } else {
                    Type::Unknown
                };
                env.insert(local.into(), Value::new(initial));
            }
            for pair in pairs {
                let directive = pair[0].keyword().map(|key| self.model.key(key));
                match directive.as_deref() {
                    Some(":or" | ":select" | ":all" | ":excess" | ":missing") => {}
                    Some(":as") if pair[1].symbol().is_some() => {}
                    Some(":req!") => {
                        let Some(entries) = pair[1].map() else {
                            selected = None;
                            self.error(
                                "PNX9001",
                                pair[1].span,
                                ":req! requires a map of bindings to keys",
                            );
                            continue;
                        };
                        let entries = entries.as_chunks::<2>().0;
                        let ordered = literal_map_in_source_order(entries);
                        let required = MapBinding {
                            source_order: input.source_order && ordered,
                            ..input
                        };
                        for (i, entry) in entries.iter().enumerate() {
                            // destmap* prepends the entries to the remaining bindings.
                            let entry = if ordered {
                                &entries[entries.len() - 1 - i]
                            } else {
                                entry
                            };
                            self.bind_entry(entry, required, true, env, &mut selected);
                        }
                    }
                    _ => {
                        if let Some(directive) = directive.as_deref()
                            && let Some(shorthand) = ShorthandBinding::parse(directive)
                        {
                            self.bind_keys(
                                directive,
                                shorthand,
                                &pair[1],
                                input,
                                env,
                                &mut selected,
                            );
                        } else {
                            self.bind_entry(pair, input, false, env, &mut selected);
                        }
                    }
                }
            }
            let defaults_span = selection.or(all).map(|binding| binding.span);
            let with_defaults = selected.as_ref()
                .filter(|_| input.collect.select || input.collect.all)
                .filter(|_| map_defaults_form(pairs).is_none_or(|form| form.map().is_some()))
                .and_then(|keys| {
                    self.inspect_path(env.reachable(), |checker| match keys.defaults(defaults) {
                        Ok(defaults) => {
                            checker.merge_destructuring_defaults(&value, defaults, defaults_span)
                        }
                        Err(ProjectionDefaultError::AlreadyReported) => {
                            Some(Cow::Owned(Value::new(Type::Unknown)))
                        }
                        Err(ProjectionDefaultError::Invalid(span, message)) => {
                            checker.error("PNX1001", span, message);
                            Some(Cow::Owned(Value::new(Type::Unknown)))
                        }
                        Err(ProjectionDefaultError::Ambiguous(span)) => {
                            Some(Cow::Owned(checker.unsupported(
                                span,
                                "Cannot resolve the map key for this :or default",
                            )))
                        }
                    })
                });
            let keys = selected.unwrap_or_default();
            let selected_value = nested_projection_input(
                with_defaults.as_deref(), keys.select, env,
            ).map(|base| {
                if base.known == Type::Unknown {
                    base.into_owned()
                } else {
                    self.inspect_path(env.reachable(), |checker| {
                        checker.select_destructured_keys(
                            &base,
                            &keys.keys,
                            selection.map_or(span, |binding| binding.span),
                        )
                    })
                }
            });
            let select = self.bind_projected_value(
                ":select",
                selection,
                selected_value,
                ":select requires known closed maps or Nil and supported literal keys in nested binding order",
                env,
            );
            let all_value = nested_projection_input(
                with_defaults.as_deref(), keys.all, env,
            ).map(Cow::into_owned);
            let all = self.bind_projected_value(
                ":all",
                all,
                all_value,
                ":all requires known closed maps or Nil and supported literal keys in nested binding order",
                env,
            );
            let excess_result = keys.excess.and_then(|children| {
                let children = resolve_nested_projections(children, env)?;
                value.excess_destructured_keys(&keys.keys, children)
            });
            let excess = self.bind_projected_value(
                ":excess",
                excess,
                excess_result,
                ":excess requires known closed maps or Nil and supported literal keys in nested binding order",
                env,
            );
            let missing_result = keys.missing.and_then(|children| {
                let children = resolve_nested_projections(children, env)?;
                value.missing_destructured_keys(&keys.required, children)
            });
            let missing = self.bind_projected_value(
                ":missing",
                missing,
                missing_result,
                if shadowed_missing {
                    ":missing uses a local also bound by the destructuring pattern"
                } else {
                    ":missing requires known closed maps or Nil and supported literal keys"
                },
                env,
            );
            inherited.any().then(|| NestedProjections {
                select: inherited.select.then_some(select).flatten(),
                all: inherited.all.then_some(all).flatten(),
                missing: inherited.missing.then_some(missing).flatten(),
                excess: inherited.excess.then_some(excess).flatten(),
            })
        })
    }

    fn bind_projected_value<'form>(
        &mut self,
        directive: &str,
        binding: Option<&'form Form>,
        result: Option<Value>,
        unsupported: &'static str,
        env: &mut Env,
    ) -> Option<NestedProjection<'form>> {
        let Some(binding) = binding else {
            return result.map(|value| NestedProjection::Generated(Box::new(value)));
        };
        let Some(local) = raw_local_symbol(binding) else {
            self.error(
                "PNX1001",
                binding.span,
                format!("{directive} requires an unqualified binding symbol"),
            );
            return None;
        };
        let known = result.is_some();
        let value = result.unwrap_or_else(|| {
            self.inspect_path(env.reachable(), |checker| {
                checker.unsupported(binding.span, unsupported)
            })
        });
        self.bind_local(local, binding.without_metadata().span, value, None, env);
        known.then_some(NestedProjection::Binding(local))
    }

    fn bind_entry<'form>(
        &mut self,
        pair: &'form [Form; 2],
        input: MapBinding<'_>,
        required: bool,
        env: &mut Env,
        selected: &mut Option<ProjectionKeys<'form>>,
    ) {
        let binding = &pair[0].without_metadata().kind;
        if !matches!(
            binding,
            Kind::Symbol(_) | Kind::AnonymousArgument(_) | Kind::Map(_) | Kind::Vector(_)
        ) {
            *selected = None;
            self.error(
                "PNX9001",
                pair[0].span,
                "Unsupported destructuring directive",
            );
            return;
        }
        let Some(key) = self.written_map_key(&pair[1]) else {
            *selected = None;
            self.error(
                "PNX9001",
                pair[1].span,
                "Destructuring requires a supported scalar literal key",
            );
            return;
        };
        let nested_map = matches!(binding, Kind::Map(_));
        if let Some(keys) = selected {
            keys.push(pair[0].symbol(), &key, required);
        }
        let nested_key = (nested_map
            && selected
                .as_ref()
                .is_some_and(ProjectionKeys::collects_children))
        .then(|| key.clone());
        let mode = if required {
            Lookup::Required
        } else {
            Lookup::Optional(None)
        };
        let child = self.inspect_path(!input.source_order || env.reachable(), |checker| {
            let (field, source) =
                checker.binding_value(input, key, pair[0].symbol(), mode, pair[1].span);
            if required {
                input.advance_missing(env);
            }
            if nested_map && input.collect.any() {
                checker.bind_map(&pair[0], field, source, None, env, input.collect)
            } else {
                checker.bind_selected(&pair[0], field, env, source, None);
                None
            }
        });
        if let (Some(key), Some(keys)) = (nested_key, selected) {
            let child = child.filter(|_| input.source_order).unwrap_or_default();
            keys.record_child(&key, child);
        }
    }

    fn bind_keys<'form>(
        &mut self,
        directive: &str,
        shorthand: ShorthandBinding<'_>,
        form: &'form Form,
        input: MapBinding<'_>,
        env: &mut Env,
        selected: &mut Option<ProjectionKeys<'form>>,
    ) {
        let Some(keys) = form.vector() else {
            *selected = None;
            self.error(
                "PNX9001",
                form.span,
                format!("{directive} requires a vector"),
            );
            return;
        };
        let mut presence_only = false;
        for key in keys {
            if key.symbol() == Some("&") {
                if presence_only {
                    *selected = None;
                    self.error("PNX9001", key.span, "A keys binding can contain only one &");
                }
                presence_only = true;
                continue;
            }
            let (local, key_type) = if presence_only {
                let Some(mut key_type) = self.written_map_key(key) else {
                    *selected = None;
                    self.error(
                        "PNX9001",
                        key.span,
                        "Keys after & must be supported scalar literals",
                    );
                    continue;
                };
                if shorthand.key == ShorthandKey::Keyword
                    && let (Some(namespace), Type::Key(name)) = (shorthand.namespace, &mut key_type)
                    && (name == ":/" || !name.contains('/'))
                {
                    *name = format!(":{namespace}/{}", name.trim_start_matches(':'));
                }
                (None, key_type)
            } else {
                let keyword = (shorthand.key == ShorthandKey::Keyword
                    && shorthand.namespace.is_none())
                .then(|| key.keyword())
                .flatten();
                let Some(name) = key
                    .symbol()
                    .or_else(|| keyword.map(|name| name.trim_start_matches(':')))
                else {
                    *selected = None;
                    self.error(
                        "PNX9001",
                        key.span,
                        if shorthand.key == ShorthandKey::Keyword && shorthand.namespace.is_none() {
                            format!("{directive} requires symbols or keywords before &")
                        } else {
                            format!("{directive} requires symbols before &")
                        },
                    );
                    continue;
                };
                if matches!(key.without_metadata().kind, Kind::AnonymousArgument(_)) {
                    *selected = None;
                    self.error(
                        "PNX9001",
                        key.span,
                        "Key inference for a generated #() argument name is not supported",
                    );
                    env.insert(name.into(), Value::new(Type::Unknown));
                    continue;
                }
                let qualified = name != "/" && name.contains('/');
                if shorthand.namespace.is_some() && qualified {
                    *selected = None;
                    self.error(
                        "PNX9001",
                        key.span,
                        "Namespaced keys directives require unqualified binding symbols",
                    );
                    continue;
                }
                if shorthand.key == ShorthandKey::String && qualified {
                    *selected = None;
                    self.error(
                        "PNX9001",
                        key.span,
                        ":strs bindings require unqualified symbols",
                    );
                    continue;
                }
                if qualified
                    && (name.starts_with('/')
                        || name.ends_with('/')
                        || name.matches('/').count() > 1)
                {
                    *selected = None;
                    self.error("PNX9001", key.span, "Invalid symbol in keys binding");
                    continue;
                }
                let key_type = keyword.map_or_else(
                    || shorthand.key_for_symbol(name),
                    |keyword| Type::Key(self.model.key(keyword)),
                );
                (Some(map_local_name(name)), key_type)
            };
            if let Some(selected) = selected {
                selected.push(local, &key_type, shorthand.required);
            }
            self.inspect_path(!input.source_order || env.reachable(), |checker| {
                if let Some(local) = local {
                    let mode = if shorthand.required {
                        Lookup::Required
                    } else {
                        Lookup::Optional(None)
                    };
                    let (field, source) =
                        checker.binding_value(input, key_type, Some(local), mode, key.span);
                    if shorthand.required {
                        input.advance_missing(env);
                    }
                    checker.bind_local(local, key.without_metadata().span, field, source, env);
                } else if shorthand.required {
                    let (field, _) =
                        checker.binding_value(input, key_type, None, Lookup::Presence, key.span);
                    input.advance_missing(env);
                    if field.known == Type::Never {
                        env.mark_impossible();
                    }
                }
            });
        }
    }

    fn map_defaults(&mut self, pairs: &[[Form; 2]], env: &Env, reachable: bool) -> Defaults {
        let mut defaults = Defaults {
            stopped: !reachable,
            ..Defaults::default()
        };
        let Some(form) = map_defaults_form(pairs) else {
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
        // Hash-map order cannot be inferred from the written defaults. One
        // definite failure still stops the binding before the initializer.
        let ordered = super::maps::literal_map_in_source_order(pairs);
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
            } else if let Some(key) = self.written_map_key(&pair[0]) {
                defaults.keys.insert(key, (value, target)).is_some()
            } else {
                self.error(
                    "PNX9001",
                    pair[0].span,
                    "Defaults require local symbols or supported scalar literal keys",
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
                } else if input.collect.missing {
                    // The compiler collects absence and binds Nil in place of its sentinel.
                    self.lookup_default(input.value, &key, None, span)
                } else {
                    self.checked_lookup(input.value, &key, mode, span)
                }
            }
        };
        (field, source)
    }
}
