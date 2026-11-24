//! Map lookup, path updates and key presence.
mod binding_input;
mod grouping;
mod keys;
mod merge;
mod paths;
mod projection;
mod transform;
mod zip;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{
    Checker, Env, LiteralContainer, Value, calls::call_returns, nil_part, quote::quoted_symbol,
};
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::{
    ExtraFields, Field, IndexedItem, IntegerClass, IntegerType, MapSlot, MapType, Type,
};

const MAX_MAP_CALLBACK_CHECKS: usize = 128;

pub(super) fn literal_key_type(
    form: &Form,
    resolve_keyword: impl FnOnce(&str) -> String,
) -> Option<Type> {
    match &form.without_metadata().kind {
        Kind::Nil => Some(Type::Nil),
        Kind::Bool(value) => Some(Type::Bool(*value)),
        Kind::Character(value) => Some(Type::Char(*value)),
        Kind::Keyword(key) => Some(Type::Key(resolve_keyword(key))),
        Kind::String(key) => Some(Type::Str(key.clone())),
        _ => None,
    }
}

pub(super) fn literal_map_in_source_order(pairs: &[[Form; 2]]) -> bool {
    // Clojure 1.13 alpha11's RT.map uses an array map for up to eight entries
    // or up to 64 when every key after the eighth is a keyword.
    pairs.len() <= 8
        || (pairs.len() <= 64 && pairs[8..].iter().all(|[key, _]| key.keyword().is_some()))
}

impl Value {
    fn equivalent_literal_map_key(&self, key: &Type) -> Option<Type> {
        let address = equivalent_integer_map_key(key)?;
        (self.is_literal_map() && matches!(self.known, Type::Map(_))).then_some(address)
    }

    fn equivalent_literal_map_key_choices(&self, key: &Type) -> Option<Type> {
        let Type::Union(parts) = key else {
            return self.equivalent_literal_map_key(key);
        };
        if !self.is_literal_map()
            || !matches!(self.known, Type::Map(_))
            || !parts.iter().any(|part| matches!(part, Type::Integral(_)))
        {
            return None;
        }
        let addresses = parts
            .iter()
            .map(|part| {
                if part.is_literal_map_key() {
                    Some(part.clone())
                } else {
                    self.equivalent_literal_map_key(part)
                }
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Type::union(addresses))
    }

    fn retaining_map_projection(&self, known: Type) -> Self {
        let mut projected = Self::with_notes(known, self.notes.clone());
        if let (Some(LiteralContainer::Map(children)), Type::Map(map)) =
            (&self.literal_container, &projected.known)
        {
            let children = children.as_ref().and_then(|children| {
                let retained = |key: &Type, child: &LiteralContainer| {
                    map.fields
                        .get(key)
                        .is_some_and(|field| child.matches_type(&field.ty))
                };
                if children.iter().all(|(key, child)| retained(key, child)) {
                    return Some(Arc::clone(children));
                }
                let fields: BTreeMap<_, _> = children
                    .iter()
                    .filter(|(key, child)| retained(key, child))
                    .map(|(key, child)| (key.clone(), child.clone()))
                    .collect();
                (!fields.is_empty()).then(|| Arc::new(fields))
            });
            projected.literal_container = Some(LiteralContainer::Map(children));
        }
        projected
    }
}

#[derive(Clone, Copy)]
pub(super) enum Lookup<'a> {
    Optional(Option<&'a Value>),
    Path(Option<&'a Value>),
    Required,
    Presence,
}

#[derive(Clone, Copy)]
pub(super) enum MapLiteral<'a> {
    Expression(&'a Env),
    Quoted { precise_symbols: bool },
}

pub(super) fn map_predicate(input: &Type) -> Type {
    match input {
        Type::Never => Type::Never,
        Type::Map(_) => Type::Bool(true),
        Type::Nil
        | Type::Bool(_)
        | Type::Boolean
        | Type::Char(_)
        | Type::Character
        | Type::Str(_)
        | Type::String
        | Type::Key(_)
        | Type::Keyword
        | Type::SpecKeyword
        | Type::Symbol
        | Type::SimpleSymbol
        | Type::QualifiedSymbol
        | Type::Sym(_)
        | Type::Integer(_)
        | Type::Integral(IntegerType {
            class: IntegerClass::Fixed | IntegerClass::IntOrLong,
            ..
        }) => Type::Bool(false),
        Type::Union(parts) => Type::union(parts.iter().map(map_predicate)),
        Type::Parameter(parameter) => parameter
            .upper
            .as_ref()
            .map_or(Type::Boolean, map_predicate),
        // An opaque JVM class can implement IPersistentMap alongside another
        // interface or even extend Number at the same time.
        _ => Type::Boolean,
    }
}

fn scalar_without_lookup(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Nil
            | Type::Bool(_)
            | Type::Boolean
            | Type::Char(_)
            | Type::Character
            | Type::Key(_)
            | Type::Keyword
            | Type::SpecKeyword
            | Type::Integer(_)
            | Type::Integral(IntegerType {
                class: IntegerClass::Fixed | IntegerClass::IntOrLong,
                ..
            })
    )
}

pub(super) fn narrow_map_predicate(input: &Type, truth: bool) -> Type {
    match input {
        Type::Union(parts) => {
            Type::union(parts.iter().map(|part| narrow_map_predicate(part, truth)))
        }
        Type::Any | Type::Unknown | Type::SpecValue | Type::SpecTruthy if truth => {
            Type::Map(MapType {
                fields: BTreeMap::new(),
                extra: ExtraFields::Open,
            })
        }
        _ if map_predicate(input) == Type::Bool(!truth) => Type::Never,
        _ => input.clone(),
    }
}

impl Checker<'_> {
    fn take_map_callback_step(&mut self) -> bool {
        if self.map_callback_checks >= MAX_MAP_CALLBACK_CHECKS {
            return false;
        }
        self.map_callback_checks += 1;
        true
    }

    pub(super) fn map_literal(&mut self, xs: &[Form], context: MapLiteral<'_>) -> Value {
        self.map_literal_with(xs, context, |checker, _, form, reachable| match context {
            MapLiteral::Expression(env) => checker.infer_path(form, env, reachable, false),
            MapLiteral::Quoted { precise_symbols } => {
                checker.quoted_data_with_symbols(form, precise_symbols)
            }
        })
    }

    pub(super) fn written_map_key(&self, form: &Form) -> Option<Type> {
        self.literal_map_key(form)
            .or_else(|| Type::integer_literal(form, self.source))
            .or_else(|| quoted_symbol(form).map(|name| Type::Sym(name.to_owned())))
    }

    fn evaluated_map_key(
        &mut self,
        form: &Form,
        ordered_env: Option<&Env>,
        reachable: bool,
    ) -> Option<Type> {
        if let Some(key) = self.written_map_key(form) {
            return Some(key);
        }
        let env = ordered_env?;
        if matches!(form.without_metadata().kind, Kind::Symbol(_)) {
            return self.symbol_map_key(form, env, reachable);
        }
        let value = self.infer_path(form, env, reachable, false);
        if value.stops_evaluation() {
            Some(Type::Never)
        } else {
            value.known.is_literal_map_key().then_some(value.known)
        }
    }

    pub(super) fn map_literal_with<'form>(
        &mut self,
        xs: &'form [Form],
        context: MapLiteral<'_>,
        mut infer_value: impl FnMut(&mut Self, Option<&Type>, &'form Form, bool) -> Value,
    ) -> Value {
        let pairs = xs.as_chunks::<2>().0;
        let ordered =
            matches!(context, MapLiteral::Quoted { .. }) || literal_map_in_source_order(pairs);
        let ordered_env = match context {
            MapLiteral::Expression(env) if ordered => Some(env),
            _ => None,
        };
        let mut fields = BTreeMap::new();
        let mut container_fields = BTreeMap::new();
        let mut extra_keys = Vec::new();
        let mut extra_values = Vec::new();
        let mut returns = true;
        let mut keys_known = true;
        let mut duplicate_key = false;
        let mut runtime_duplicate = None;
        for [key_form, value_form] in pairs {
            let key_reachable = returns || !ordered;
            let key = match context {
                MapLiteral::Expression(_) => {
                    self.evaluated_map_key(key_form, ordered_env, key_reachable)
                }
                MapLiteral::Quoted { precise_symbols } => self
                    .literal_map_key(key_form)
                    .or_else(|| Type::integer_literal(key_form, self.source))
                    .or_else(|| {
                        let symbol = key_form.symbol().filter(|_| precise_symbols)?;
                        Some(Type::Sym(symbol.to_owned()))
                    }),
            };
            let failed_key = key == Some(Type::Never);
            returns &= !failed_key;
            let key = key.filter(|_| !failed_key);
            let value = infer_value(self, key.as_ref(), value_form, returns || !ordered);
            returns &= !value.stops_evaluation();
            if let Some(key) = key {
                if fields
                    .insert(key.clone(), Field::new(value.known, true))
                    .is_some()
                {
                    duplicate_key = true;
                    match context {
                        MapLiteral::Expression(_) => {
                            runtime_duplicate.get_or_insert((key, key_form.span));
                        }
                        MapLiteral::Quoted { .. } => {
                            self.error(
                                "PNX2790",
                                key_form.span,
                                format!("Duplicate map key {key}"),
                            );
                        }
                    }
                } else if let Some(container) = value.literal_container {
                    container_fields.insert(key, container);
                }
            } else if let MapLiteral::Quoted { precise_symbols } = context {
                extra_keys.push(
                    self.quoted_data_with_symbols(key_form, precise_symbols)
                        .known,
                );
                extra_values.push(value.known);
            } else if key_reachable && !failed_key {
                self.unchecked_form(
                    key_form.span,
                    "Map key analysis needs an exact nil, boolean, character, keyword, string, symbol or supported integer",
                );
                keys_known = false;
            }
        }
        if !returns {
            return Value::new(Type::Never);
        }
        if !keys_known {
            return Value::new(Type::Unknown);
        }
        if let Some((key, span)) = runtime_duplicate {
            if self.path_reachable {
                self.error("PNX2790", span, format!("Duplicate map key {key}"));
                self.illegal_argument();
            }
            return Value::new(Type::Never);
        }
        let container_fields =
            (!duplicate_key && extra_keys.is_empty() && !container_fields.is_empty())
                .then(|| Arc::new(container_fields));
        let mut literal = Value::new(Type::Map(MapType {
            fields,
            extra: if extra_keys.is_empty() {
                ExtraFields::Closed
            } else {
                ExtraFields::Typed(Box::new([
                    Type::union(extra_keys),
                    Type::union(extra_values),
                ]))
            },
        }));
        literal.literal_container =
            (!duplicate_key).then_some(LiteralContainer::Map(container_fields));
        literal
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
        let scalar = scalar_without_lookup(&value.known);
        match key {
            Type::Never => return Value::new(Type::Never),
            Type::Unknown if !scalar => return Value::new(Type::Unknown),
            Type::Union(parts) if !scalar => {
                let values = parts
                    .iter()
                    .map(|key| self.lookup_value(value, key, mode, span));
                return values.fold(Value::new(Type::Never), Value::join);
            }
            _ => {}
        }
        let equivalent_key = value.equivalent_literal_map_key(key);
        let key = equivalent_key.as_ref().unwrap_or(key);
        let default = match mode {
            Lookup::Optional(default) | Lookup::Path(default) => default,
            _ => None,
        };

        let default_type = || match mode {
            Lookup::Optional(default) | Lookup::Path(default) => {
                default.map_or(Type::Nil, |v| v.known.clone())
            }
            Lookup::Required | Lookup::Presence => Type::Never,
        };
        let mut notes = value.notes.clone();
        let mut literal_container = None;
        let mut selected_default = false;
        let mut absent = || {
            selected_default = true;
            default_type()
        };
        let ty = match &value.known {
            _ if scalar => absent(),
            indexed
                if let Some(index) = key.associative_index()
                    && let Some(item) = indexed.indexed_item(index) =>
            {
                match item {
                    IndexedItem::Present(item)
                        if key.may_be_byte_or_short() && !indexed.subtype_of(&Type::String) =>
                    {
                        if matches!(mode, Lookup::Required | Lookup::Presence) {
                            item.clone()
                        } else {
                            Type::union([item.clone(), default_type()])
                        }
                    }
                    IndexedItem::Present(item) => {
                        literal_container = usize::try_from(index).ok().and_then(|index| {
                            value.literal_container.as_ref()?.position_child(index)
                        });
                        item.clone()
                    }
                    IndexedItem::PresentChar(value) => Type::Char(value),
                    IndexedItem::Optional(item)
                        if matches!(mode, Lookup::Required | Lookup::Presence) =>
                    {
                        item.clone()
                    }
                    IndexedItem::Optional(item) => Type::union([item.clone(), default_type()]),
                    IndexedItem::Absent(_)
                        if matches!(indexed, Type::String | Type::Str(_))
                            || matches!(
                                value.literal_container,
                                Some(LiteralContainer::Vector(_))
                            ) =>
                    {
                        absent()
                    }
                    IndexedItem::Absent(_) => default_type(),
                }
            }
            ty if ty.is_vector() || matches!(ty, Type::String | Type::Str(_)) => default_type(),
            Type::Never => Type::Never,
            Type::Any => {
                self.may_throw();
                Type::Any
            }
            Type::Unknown | Type::Parameter(_) => Type::Unknown,
            Type::Number => {
                self.unchecked_form(
                    span,
                    "Lookup on this JVM type may use a custom ILookup implementation",
                )
                .known
            }
            Type::Union(xs) => {
                let values = xs
                    .iter()
                    .map(|t| self.lookup_value(&Value::new(t.clone()), key, mode, span))
                    .collect::<Vec<_>>();
                notes.extend(values.iter().flat_map(|v| v.notes.clone()));
                Type::union(values.into_iter().map(|v| v.known))
            }
            Type::Set(_) if value.set_presence(key) == Some(false) => absent(),
            Type::Set(element) => value.set_lookup_type(element, key, default_type()),
            Type::Map(map)
                if let Some([stored_key, stored]) = map.dictionary_types()
                    && !matches!(
                        value.literal_container,
                        Some(LiteralContainer::Map(_) | LiteralContainer::GroupBuckets)
                    ) =>
            {
                // The contract describes stored entries. A comparator can
                // accept a lookup key outside the stored key domain.
                if !stored_key.key_overlaps(key) {
                    self.may_throw();
                }
                if matches!(mode, Lookup::Required | Lookup::Presence) {
                    stored.clone()
                } else {
                    Type::union([stored.clone(), default_type()])
                }
            }
            Type::Map(map) if !key.is_literal_map_key() => {
                match map.possible_values(key) {
                    Some(Type::Never) => absent(),
                    Some(ty) if matches!(mode, Lookup::Required | Lookup::Presence) => ty,
                    Some(ty) => {
                        let default_type = default_type();
                        notes.push(format!("A key of type {key} may be absent; a missing key returns {default_type}"));
                        Type::union([ty, default_type])
                    }
                    None if matches!(mode, Lookup::Presence) => Type::Unknown,
                    None => self.undeclared_key(key, span),
                }
            }
            Type::Map(map) => match map.slot(key) {
                MapSlot::Present(ty) => {
                    literal_container = value
                        .literal_container
                        .as_ref()
                        .and_then(|container| container.map_field(key));
                    ty.clone()
                }
                MapSlot::Optional(ty) if matches!(mode, Lookup::Required | Lookup::Presence) => {
                    ty.clone()
                }
                MapSlot::Optional(ty) => {
                    let default_type = default_type();
                    notes.push(format!(
                        "Field {key} is optional; a missing key returns {default_type}"
                    ));
                    Type::union([ty.clone(), default_type])
                }
                MapSlot::Absent => absent(),
                MapSlot::Undeclared if matches!(mode, Lookup::Presence) => Type::Unknown,
                MapSlot::Undeclared => self.undeclared_key(key, span),
            },
            _ if matches!(mode, Lookup::Path(_) | Lookup::Required) => default_type(),
            _ => {
                self.error(
                    "PNX2322",
                    span,
                    format!("Lookup expects a map, set or Nil, got {}", value.ty),
                );
                Type::Unknown
            }
        };
        if !selected_default
            && matches!(
                value.literal_container,
                Some(LiteralContainer::GroupBuckets)
            )
            && default.is_none_or(|value| {
                value.known == Type::Nil
                    || matches!(
                        value.literal_container,
                        Some(LiteralContainer::NonemptyVector)
                    )
            })
        {
            literal_container = Some(&LiteralContainer::NonemptyVector);
        }
        if let Some(default) = default {
            notes.extend(default.notes.clone());
            if selected_default {
                literal_container = default.literal_container.as_ref();
            }
        }
        let mut result = Value::with_notes(ty, notes);
        result.literal_container = literal_container
            .filter(|container| container.matches_type(&result.known))
            .cloned();
        result
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
            return self.reject_runtime_arity(
                &xs[1..],
                span,
                env,
                "contains? requires a target and key",
            );
        };
        let args: Vec<_> = self.eval_args(&xs[1..], env, true).collect();
        if !call_returns(&args) {
            return Value::new(Type::Never);
        }
        let mut args = args.into_iter();
        let (value, _) = args.next().unwrap();
        let (key_value, key_span) = args.next().unwrap();
        let key_type = if matches!(value.known, Type::Set(_)) {
            key_value.known
        } else {
            let Some(key_type) = self.validate_map_key(key_value, key_span, "contains?", true)
            else {
                self.may_throw();
                return Value::new(Type::Unknown);
            };
            key_type
        };
        let equivalent_key = value.equivalent_literal_map_key_choices(&key_type);
        let key_type = equivalent_key.as_ref().unwrap_or(&key_type);
        let presence = env
            .presence(target, key)
            .or_else(|| value.set_presence(key_type));
        Value::new(match presence {
            Some(present) => Type::Bool(present),
            None => self.contains_type(&value.known, key_type, target.span, value.is_literal_set()),
        })
    }

    fn contains_type(&mut self, ty: &Type, key: &Type, span: Span, literal_set: bool) -> Type {
        match key {
            Type::Any | Type::Unknown => {
                self.may_throw();
                return Type::Unknown;
            }
            Type::Never => return Type::Never,
            Type::Union(parts) => {
                return Type::union(
                    parts
                        .iter()
                        .map(|key| self.contains_type(ty, key, span, literal_set)),
                );
            }
            _ => {}
        }
        match ty {
            Type::Any | Type::Unknown => {
                self.may_throw();
                Type::Unknown
            }
            Type::Never => Type::Never,
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.contains_type(part, key, span, literal_set)),
            ),
            Type::Set(element)
                if literal_set && element.hash_set_member_type(key) == Type::Never =>
            {
                Type::Bool(false)
            }
            Type::Map(map) if let Some([stored_key, _]) = map.dictionary_types() => {
                if !stored_key.key_overlaps(key) {
                    self.may_throw();
                }
                Type::Boolean
            }
            ty if ty.is_vector() || matches!(ty, Type::Map(_) | Type::Nil | Type::Set(_)) => {
                key_presence(ty, key).map_or(Type::Boolean, Type::Bool)
            }
            Type::String | Type::Str(_) if key.associative_index().is_some() => {
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
            Type::Any | Type::Unknown => {
                self.may_throw();
                Err(Type::Unknown)
            }
            ty if ty.is_vector() => {
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
            self.error(
                "PNX2554",
                span,
                "assoc requires a target and key/value pairs",
            );
            return Value::new(Type::Unknown);
        }
        let args: Vec<_> = self.eval_args(&xs[1..], env, true).collect();
        if !call_returns(&args) {
            return Value::new(Type::Never);
        }
        let pairs = args[1..].as_chunks::<2>().0;
        if args[0].0.known.is_vector()
            && !matches!(args[0].0.known, Type::MapEntry(_))
            && xs[0]
                .symbol()
                .is_some_and(|name| self.model.can_use_standard_core(name, self.globals))
        {
            return self.assoc_vector(&args[0], pairs);
        }
        let target = &args[0].0;
        let ordinary = target.known == Type::Nil || target.is_literal_map();
        let mut literal_children = match &target.literal_container {
            Some(LiteralContainer::Map(children)) => children.clone(),
            _ => None,
        };
        let mut updates = Vec::new();
        for [(key, key_span), (value, _)] in pairs {
            let numeric = if ordinary {
                match key.known {
                    Type::Integer(n) => Some((Type::Integer(n), false)),
                    _ => target
                        .equivalent_literal_map_key(&key.known)
                        .map(|address| (address, true)),
                }
            } else {
                None
            };
            let update = numeric.or_else(|| {
                self.validate_map_key(key.clone(), *key_span, "assoc", false)
                    .map(|key| (key, false))
            });
            let Some((key, replace_only)) = update.filter(|(key, _)| !key.contains_unknown())
            else {
                self.may_throw();
                return Value::new(Type::Unknown);
            };
            updates.push((key, value, replace_only));
        }
        let parts = match &target.known {
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
            for ((key, value, replace_only), [(_, key_span), _]) in updates.iter().zip(pairs) {
                // Replacing an equivalent key retains the original stored object.
                if *replace_only && !matches!(map.slot(key), MapSlot::Present(_)) {
                    return self.unsupported(
                        *key_span,
                        "assoc does not model storing a new key of this integer class",
                    );
                }
                if unresolved_integer_key_class(&map, key) {
                    return self.unsupported(
                        *key_span,
                        "assoc cannot determine the stored class of an equivalent integer key",
                    );
                }
                map.associate(key, &value.known);
                if ordinary {
                    associate_literal_child(
                        &mut literal_children,
                        key,
                        &value.known,
                        value.literal_container.as_ref(),
                    );
                }
            }
            results.push(Type::Map(map));
        }
        let mut result = Value::new(Type::union(results));
        if ordinary && matches!(result.known, Type::Map(_)) {
            result.literal_container = Some(LiteralContainer::Map(
                literal_children.filter(|children| !children.is_empty()),
            ));
        }
        result
    }

    pub(super) fn dissoc(&mut self, args: &[(Value, Span)]) -> Value {
        let [(target, span), keys @ ..] = args else {
            unreachable!("arity was checked");
        };
        if keys.is_empty() {
            return target.clone();
        }
        let start = self.diagnostics.len();
        let ordinary = target.is_literal_map().then_some(target);
        let result = self.dissoc_type(&target.known, keys, *span, ordinary);
        self.dedup_diagnostics(start);
        target.retaining_map_projection(result)
    }

    fn dissoc_type(
        &mut self,
        target: &Type,
        keys: &[(Value, Span)],
        span: Span,
        ordinary: Option<&Value>,
    ) -> Type {
        match target {
            Type::Map(map) => {
                // A runtime map can have a comparator that rejects a key.
                self.may_throw();
                let mut map = map.clone();
                for (value, span) in keys {
                    let key = ordinary
                        .and_then(|target| match value.known {
                            Type::Integer(n) => Some(Type::Integer(n)),
                            _ => target.equivalent_literal_map_key_choices(&value.known),
                        })
                        .or_else(|| self.validate_map_key(value.clone(), *span, "dissoc", false));
                    let Some(key) = key.filter(|key| !key.contains_unknown()) else {
                        return Type::Unknown;
                    };
                    map.dissociate(&key);
                }
                Type::Map(map)
            }
            Type::Nil | Type::Never => target.clone(),
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.dissoc_type(part, keys, span, None)),
            ),
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.dissoc_type(upper, keys, span, None)
            }
            Type::Any | Type::Unknown => {
                self.may_throw();
                Type::Unknown
            }
            _ => {
                self.class_cast();
                self.diagnostics.push(Diagnostic::type_mismatch(
                    span,
                    "clojure.core/dissoc",
                    "Map | Nil".into(),
                    target.inferred().to_string(),
                ));
                Type::Never
            }
        }
    }
}

fn associate_literal_child(
    children: &mut Option<Arc<BTreeMap<Type, LiteralContainer>>>,
    key: &Type,
    value: &Type,
    proof: Option<&LiteralContainer>,
) {
    if key.is_literal_map_key() {
        if let Some(proof) = proof.filter(|child| child.matches_type(value)) {
            let children = children.get_or_insert_with(|| Arc::new(BTreeMap::new()));
            Arc::make_mut(children).insert(key.clone(), proof.clone());
        } else if let Some(children) = children
            && children.contains_key(key)
        {
            Arc::make_mut(children).remove(key);
        }
    } else if let Some(children) = children
        && children.keys().any(|stored| key.key_overlaps(stored))
    {
        Arc::make_mut(children).retain(|stored, _| !key.key_overlaps(stored));
    }
}

fn equivalent_integer_map_key(key: &Type) -> Option<Type> {
    // The field address leaves the stored object's type intact.
    if matches!(
        key,
        Type::Integral(IntegerType {
            class: IntegerClass::Fixed | IntegerClass::IntOrLong | IntegerClass::BigInt,
            ..
        })
    ) {
        key.exact_integer_value().map(Type::Integer)
    } else {
        None
    }
}

fn unresolved_integer_key_class(map: &MapType, key: &Type) -> bool {
    matches!(key, Type::Integer(_))
        && !map.fields.contains_key(key)
        && !matches!(map.slot(key), MapSlot::Absent)
}

/// Presence is independent of the stored value's truthiness.
fn key_presence(ty: &Type, key: &Type) -> Option<bool> {
    if let Type::Union(keys) = key {
        let mut keys = keys.iter();
        let first = key_presence(ty, keys.next()?)?;
        return keys
            .all(|key| key_presence(ty, key) == Some(first))
            .then_some(first);
    }
    match ty {
        Type::Map(map) if map.dictionary_types().is_some() => None,
        Type::Map(map) if !key.is_literal_map_key() => {
            (map.possible_values(key) == Some(Type::Never)).then_some(false)
        }
        Type::Map(map) => match map.slot(key) {
            MapSlot::Present(_) => Some(true),
            MapSlot::Absent => Some(false),
            _ => None,
        },
        Type::Nil => Some(false),
        ty if ty.is_vector() || matches!(ty, Type::String | Type::Str(_)) => {
            let Some(index) = key.associative_index() else {
                return Some(false);
            };
            match ty.indexed_item(index) {
                Some(IndexedItem::Present(_)) => (matches!(ty, Type::String | Type::Str(_))
                    || !key.may_be_byte_or_short())
                .then_some(true),
                Some(IndexedItem::PresentChar(_)) => Some(true),
                Some(IndexedItem::Absent(_)) => Some(false),
                _ => None,
            }
        }
        Type::Set(element) if **element == Type::Never => Some(false),
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
                    map.set_field(key.to_owned(), Field::new(ty.clone(), true));
                }
            } else {
                map.dissociate(key);
            }
            Type::Map(map)
        }
        _ => ty.clone(),
    }
}
