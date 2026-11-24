//! Lookup and presence for keys supplied as expressions.
use std::sync::Arc;

use super::super::{
    Checker, Env, LiteralContainer, Value, calls::call_returns, model::DeclaredValue,
};
use super::{Lookup, key_presence, literal_key_type, narrow_key_presence, scalar_without_lookup};
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::{ExtraFields, IndexedItem, Type};

impl Checker<'_> {
    pub(super) fn symbol_map_key(
        &mut self,
        form: &Form,
        env: &Env,
        reachable: bool,
    ) -> Option<Type> {
        let Kind::Symbol(name) = &form.kind else {
            return None;
        };
        if !name.contains('/') && name.find('.').is_some_and(|dot| dot > 0) {
            return None;
        }
        let known = if let Some(binding) = env.get(name) {
            &binding.known
        } else {
            match self
                .model
                .declared_value(name, self.globals, form.span.start.offset)?
            {
                DeclaredValue::Literal(literal) => &literal.ty,
                DeclaredValue::ExternLiteral(literal) => &literal.ty,
                _ => return None,
            }
        };
        if !known.is_literal_map_key() {
            return None;
        }
        // Record evidence at the written symbol through the ordinary expression path.
        let checked = self
            .inspect_path(reachable && env.reachable(), |checker| {
                checker.infer(form, env)
            })
            .known;
        checked.is_literal_map_key().then_some(checked)
    }

    pub(in crate::checker) fn literal_map_key(&self, form: &Form) -> Option<Type> {
        literal_key_type(form, |key| self.model.key(key))
    }

    pub(super) fn read_map_key(
        &mut self,
        form: &Form,
        env: &Env,
        name: &str,
        allow_index: bool,
    ) -> Option<Type> {
        let key = self.infer(form, env);
        if key.stops_evaluation() {
            return Some(Type::Never);
        }
        self.validate_map_key(key, form.span, name, allow_index)
    }

    pub(super) fn validate_map_key(
        &mut self,
        key: Value,
        span: Span,
        name: &str,
        allow_index: bool,
    ) -> Option<Type> {
        if key.known.is_lookup_key(allow_index) {
            Some(key.known)
        } else {
            self.unsupported(
                span,
                format!(
                    "{name} analysis supports nil, boolean, character, keyword, string and symbol keys{}, got {}",
                    if allow_index {
                        " and known integer keys"
                    } else {
                        ""
                    },
                    key.ty
                ),
            );
            None
        }
    }

    pub(in crate::checker) fn lookup_form(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        name: &str,
    ) -> Value {
        let required = name == "clojure.core/req!";
        if !(3..=if required { 3 } else { 4 }).contains(&xs.len()) {
            return self.reject_runtime_arity(
                &xs[1..],
                span,
                env,
                format!(
                    "Invalid arity for {name}: expected a collection and key{}",
                    if required {
                        ""
                    } else {
                        " and optional default"
                    }
                ),
            );
        }
        let args: Vec<_> = self.eval_args(&xs[1..], env, true).collect();
        if !call_returns(&args) {
            return Value::new(Type::Never);
        }
        let mut args = args.into_iter();
        let (target, _) = args.next().unwrap();
        let (key, key_span) = args.next().unwrap();
        let default = args.next().map(|(value, _)| value);
        let scalar = scalar_without_lookup(&target.known);
        let key = if scalar || matches!(target.known, Type::Set(_)) {
            key.known
        } else {
            let Some(key) = self.validate_map_key(key, key_span, name, true) else {
                self.may_throw();
                return Value::new(Type::Unknown);
            };
            key
        };
        if !scalar && (target.known.contains_unknown() || key.contains_unknown()) {
            self.may_throw();
        }
        let equivalent_key = target.equivalent_literal_map_key_choices(&key);
        let key = equivalent_key.as_ref().unwrap_or(&key);
        let presence = env
            .presence(&xs[1], &xs[2])
            .or_else(|| target.set_presence(key));
        let result = if required {
            if presence.or_else(|| key_presence(&target.known, key)) != Some(true) {
                self.illegal_argument();
            }
            let target = if presence == Some(false) {
                Value::new(Type::Nil)
            } else {
                target
            };
            self.checked_lookup(&target, key, Lookup::Required, key_span)
        } else {
            self.lookup_with_presence(&target, key, default.as_ref(), key_span, presence)
        };
        self.observe_lookup(xs[2].without_metadata().span, &result);
        result
    }

    pub(super) fn lookup_with_presence(
        &mut self,
        target: &Value,
        key: &Type,
        default: Option<&Value>,
        span: Span,
        presence: Option<bool>,
    ) -> Value {
        match presence {
            Some(true) => self.lookup_value(target, key, Lookup::Required, span),
            Some(false) => default.cloned().unwrap_or_else(|| Value::new(Type::Nil)),
            None => self.lookup_default(target, key, default, span),
        }
    }

    pub(in crate::checker) fn find_entry(
        &mut self,
        (target, span): &(Value, Span),
        key: &(Value, Span),
    ) -> Value {
        let start = self.diagnostics.len();
        let equivalent_key = target.equivalent_literal_map_key_choices(&key.0.known);
        let result = match &equivalent_key {
            Some(key_type) => self.find_at(&target.known, key_type, key.1),
            None => self.find_type(&target.known, key, *span),
        };
        self.dedup_diagnostics(start);
        let mut result = Value::with_notes(result, target.notes.clone());
        if let Type::MapEntry(items) = &result.known
            && let Some(container) = &target.literal_container
        {
            let stored = if matches!(target.known, Type::Map(_)) {
                container.map_field(equivalent_key.as_ref().unwrap_or(&key.0.known))
            } else {
                key.0
                    .known
                    .associative_index()
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| container.position_child(index))
            };
            if let Some(stored) = stored.filter(|proof| proof.matches_type(&items[1])) {
                result.literal_container = Some(LiteralContainer::Positions(Arc::from([
                    None,
                    Some(stored.clone()),
                ])));
            }
        }
        result
    }

    fn find_type(&mut self, target: &Type, key: &(Value, Span), span: Span) -> Type {
        match target {
            Type::Nil | Type::Never => target.clone(),
            Type::Any | Type::Unknown => {
                self.may_throw();
                Type::Unknown
            }
            Type::Union(parts) => {
                Type::union(parts.iter().map(|part| self.find_type(part, key, span)))
            }
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.find_type(upper, key, span)
            }
            ty if ty.is_vector() || matches!(ty, Type::Map(_)) => {
                let (key, key_span) = key;
                let valid = self.validate_map_key(key.clone(), *key_span, "find", true);
                valid.map_or(Type::Unknown, |key| self.find_at(target, &key, *key_span))
            }
            actual => {
                self.illegal_argument();
                self.diagnostics.push(Diagnostic::type_mismatch(
                    span,
                    "clojure.core/find",
                    "Map, Vector or Nil".into(),
                    actual.inferred().to_string(),
                ));
                Type::Never
            }
        }
    }

    fn find_at(&mut self, target: &Type, key: &Type, span: Span) -> Type {
        match key {
            Type::Unknown => {
                self.may_throw();
                return Type::Unknown;
            }
            Type::Never => return Type::Never,
            Type::Union(parts) => {
                return Type::union(parts.iter().map(|key| self.find_at(target, key, span)));
            }
            _ => {}
        }
        let entry = |stored_key: &Type, value: &Type| {
            Type::MapEntry(Box::new([stored_key.clone(), value.clone()]))
        };
        if let Type::Map(map) = target {
            // An associative input can use a comparator that rejects the key.
            self.may_throw();
            if let Some((stored, field)) = map.fields.get_key_value(key) {
                return if field.required {
                    entry(stored, &field.ty)
                } else if field.ty == Type::Never {
                    Type::Nil
                } else {
                    entry(stored, &field.ty).nilable()
                };
            }
            let extra = match &map.extra {
                ExtraFields::Open => return self.undeclared_key(key, span),
                ExtraFields::Typed(types) => Some(Type::MapEntry(types.clone())),
                ExtraFields::Closed => None,
            };
            if key.is_literal_map_key() {
                return extra.map_or(Type::Nil, Type::nilable);
            }
            let stored = map
                .fields
                .iter()
                .filter(|(stored, field)| field.ty != Type::Never && key.key_overlaps(stored))
                .map(|(stored, field)| entry(stored, &field.ty));
            return Type::union(stored.chain(extra).chain([Type::Nil]));
        }
        let Some(index) = key.associative_index() else {
            return Type::Nil;
        };
        match target
            .indexed_item(index)
            .expect("an indexed associative target")
        {
            IndexedItem::Present(value) if key.may_be_byte_or_short() => {
                entry(key, value).nilable()
            }
            IndexedItem::Present(value) => entry(key, value),
            IndexedItem::PresentChar(_) => unreachable!("indexed find excludes strings"),
            IndexedItem::Optional(value) => entry(key, value).nilable(),
            IndexedItem::Absent(_) => Type::Nil,
        }
    }

    pub(in crate::checker) fn narrow_contains(&self, args: &[Form], truth: bool, env: &mut Env) {
        let [target, key] = args else { return };
        let key_symbol = key
            .symbol()
            .and_then(|name| env.get(name).map(|v| (name, &v.known)));
        let exact = self.written_map_key(key).or_else(|| {
            key_symbol
                .filter(|(_, ty)| ty.is_literal_map_key())
                .map(|(_, ty)| ty.clone())
        });
        if let Some(key) = exact {
            self.narrow_subject(target, env, |ty| narrow_key_presence(ty, &key, truth));
        } else if let Some((key_name, _)) = key_symbol
            && let Some(collection) = target.symbol()
            && env.contains_key(collection)
        {
            env.set_presence(collection, key_name, truth);
        }
    }
}
