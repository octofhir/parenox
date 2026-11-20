//! Narrow immutable map reads back to their lexical owner.
use super::narrow_by_type;
use crate::checker::{
    Checker, Env,
    env::{MAX_SELECTOR_KEYS, Selector},
};
use crate::reader::{Form, Kind};
use crate::types::{Field, MapSlot, Type};

impl Checker<'_> {
    /// Only pure lookups rooted in a local are stable. In particular, a lookup
    /// default, computed key, or shadowed core call is not a selector.
    pub(in crate::checker) fn selector(&self, form: &Form, env: &Env) -> Option<Selector> {
        match &form.without_metadata().kind {
            Kind::Symbol(name) | Kind::AnonymousArgument(name) => env.selector_for(name),
            Kind::List(forms) => {
                let (head, args) = forms.split_first()?;
                if let [target] = args
                    && head.keyword().is_some()
                {
                    let key = self.literal_map_key(head)?;
                    let mut selector = self.selector(target, env)?;
                    selector.keys.push(key);
                    return (selector.keys.len() <= MAX_SELECTOR_KEYS).then_some(selector);
                }
                let [target, key] = args else { return None };
                let op = self.unshadowed_operation(head, env)?;
                if !matches!(op.as_str(), "clojure.core/get" | "clojure.core/get-in") {
                    return None;
                }
                let mut selector = self.selector(target, env)?;
                match op.as_str() {
                    "clojure.core/get" => selector.keys.push(self.literal_map_key(key)?),
                    "clojure.core/get-in" => {
                        let keys = key.vector()?;
                        if keys.is_empty() || selector.keys.len() + keys.len() > MAX_SELECTOR_KEYS {
                            return None;
                        }
                        for key in keys {
                            selector.keys.push(self.literal_map_key(key)?);
                        }
                    }
                    _ => return None,
                }
                (selector.keys.len() <= MAX_SELECTOR_KEYS).then_some(selector)
            }
            _ => None,
        }
    }

    pub(in crate::checker) fn narrow_subject(
        &self,
        form: &Form,
        env: &mut Env,
        refine: impl Fn(&Type) -> Type,
    ) {
        let selector = self.selector(form, env);
        // Narrow an alias directly even when the owner's declared shape does
        // not describe the path. A direct owner is narrowed through its root.
        if let Some(name) = form.symbol().filter(|name| env.is_alias(name))
            && let Some(value) = env.get_mut(name)
        {
            let narrowed = refine(&value.known);
            let impossible = narrowed == Type::Never;
            value.narrow(narrowed);
            if impossible {
                env.mark_impossible();
            }
        }
        if let Some(selector) = selector {
            refine_owner(&selector, env, refine);
        }
    }

    pub(super) fn guard_literal(&self, form: &Form) -> Option<Type> {
        match &form.without_metadata().kind {
            Kind::Nil => Some(Type::Nil),
            Kind::Bool(value) => Some(Type::Bool(*value)),
            Kind::Character(value) => Some(Type::Char(*value)),
            Kind::Keyword(_) | Kind::String(_) => self.literal_map_key(form),
            _ => None,
        }
    }
}

pub(in crate::checker) fn refine_owner(
    selector: &Selector,
    env: &mut Env,
    refine: impl Fn(&Type) -> Type,
) {
    if let Some(owner) = env.owner(&selector.root) {
        let narrowed = narrow_path(&owner.known, &selector.keys, &refine);
        if narrowed != owner.known {
            if narrowed == Type::Never {
                env.mark_impossible();
            } else {
                env.refresh_selectors(&selector.root, |keys| project_path(&narrowed, keys));
            }
            env.owner_mut(&selector.root).unwrap().narrow(narrowed);
        }
    }
}

fn project_path(owner: &Type, keys: &[Type]) -> Option<Type> {
    let Some((key, rest)) = keys.split_first() else {
        return Some(owner.clone());
    };
    match owner {
        Type::Never | Type::Nil => Some(owner.clone()),
        Type::Union(parts) => parts
            .iter()
            .map(|part| project_path(part, keys))
            .collect::<Option<Vec<_>>>()
            .map(Type::union),
        Type::Map(map) => match map.slot(key) {
            MapSlot::Present(ty) => project_path(ty, rest),
            MapSlot::Optional(ty) => project_path(ty, rest).map(Type::nilable),
            MapSlot::Absent => Some(Type::Nil),
            MapSlot::Undeclared => None,
        },
        _ => None,
    }
}

fn narrow_path(owner: &Type, path: &[Type], refine: &impl Fn(&Type) -> Type) -> Type {
    let Some((key, rest)) = path.split_first() else {
        return refine(owner);
    };
    match owner {
        Type::Union(parts) => Type::union(parts.iter().map(|part| narrow_path(part, path, refine))),
        Type::Nil => {
            if refine(&Type::Nil) == Type::Never {
                Type::Never
            } else {
                Type::Nil
            }
        }
        Type::Map(map) => {
            let (original, originally_required) = match map.slot(key) {
                MapSlot::Present(ty) => (ty, true),
                MapSlot::Optional(ty) => (ty, false),
                MapSlot::Absent if refine(&Type::Nil) == Type::Never => return Type::Never,
                MapSlot::Absent | MapSlot::Undeclared => return owner.clone(),
            };
            // With no default, absent keys and nil intermediate maps read as nil.
            let missing_can_match = !originally_required && refine(&Type::Nil) != Type::Never;
            let ty = narrow_path(original, rest, refine);
            if !missing_can_match && ty == Type::Never {
                return Type::Never;
            }
            let required = !missing_can_match;
            if &ty == original && originally_required == required {
                return owner.clone();
            }
            let mut map = map.clone();
            map.set_field(key.clone(), Field::new(ty, required));
            Type::Map(map)
        }
        _ => owner.clone(),
    }
}

pub(in crate::checker) fn narrow_literal(actual: &Type, literal: &Type, truth: bool) -> Type {
    match (actual, literal, truth) {
        (Type::Boolean, Type::Bool(value), false) => Type::Bool(!value),
        (Type::Union(parts), _, _) => Type::union(
            parts
                .iter()
                .map(|part| narrow_literal(part, literal, truth)),
        ),
        _ => narrow_by_type(actual, literal, truth),
    }
}
