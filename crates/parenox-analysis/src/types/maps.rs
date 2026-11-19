//! Map shapes and types for their additional entries.
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use super::{Compatibility, IntegerRange, Type};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MapType {
    pub fields: BTreeMap<Type, Field>,
    pub extra: ExtraFields,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Field {
    pub ty: Type,
    pub required: bool,
    check_value: bool,
}

impl Field {
    pub fn new(ty: Type, required: bool) -> Self {
        Self {
            ty,
            required,
            check_value: true,
        }
    }

    /// The key is declared, but there is no checked constraint on its value.
    /// Reading the field still yields Unknown.
    pub(crate) fn presence_only(required: bool) -> Self {
        Self {
            ty: Type::Unknown,
            required,
            check_value: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExtraFields {
    Closed,
    Open,
    Typed(Box<[Type; 2]>),
}

pub enum MapSlot<'a> {
    Present(&'a Type),
    Optional(&'a Type),
    Absent,
    Undeclared,
}

impl ExtraFields {
    fn is_open(&self) -> bool {
        matches!(self, Self::Open)
    }

    fn inferred(&self) -> Self {
        match self {
            Self::Typed(types) => Self::Typed(Box::new(types.each_ref().map(Type::inferred))),
            _ => self.clone(),
        }
    }

    pub(super) fn accepts_type(&self, other: &Self) -> Compatibility {
        match (self, other) {
            (Self::Open, _) | (_, Self::Closed) => Compatibility::Proven,
            (Self::Typed(expected), Self::Typed(actual)) => Compatibility::all(
                actual
                    .iter()
                    .zip(expected.iter())
                    .map(|(a, e)| a.compatibility(e)),
            ),
            _ => Compatibility::Mismatch,
        }
    }

    pub(super) fn accepts_field(&self, key: &Type, ty: &Type) -> Compatibility {
        match self {
            Self::Open => Compatibility::Proven,
            Self::Closed => Compatibility::Mismatch,
            Self::Typed(types) => key
                .compatibility(&types[0])
                .and(|| ty.compatibility(&types[1])),
        }
    }
}

impl MapSlot<'_> {
    pub(super) fn compatibility(&self, field: &Field) -> Compatibility {
        let value = |actual: &Type| {
            if field.check_value {
                actual.compatibility(&field.ty)
            } else {
                Compatibility::Proven
            }
        };
        match self {
            Self::Present(actual) => value(actual),
            Self::Optional(actual) => Compatibility::from(!field.required).and(|| value(actual)),
            Self::Absent => (!field.required).into(),
            Self::Undeclared => (!field.required && !field.check_value).into(),
        }
    }
}

impl Type {
    pub fn is_literal_map_key(&self) -> bool {
        matches!(self, Self::Key(_) | Self::Str(_) | Self::Integer(_))
    }

    pub fn is_lookup_key(&self, allow_index: bool) -> bool {
        match self {
            Self::Union(parts) => parts.iter().all(|key| key.is_lookup_key(allow_index)),
            Self::Integer(_) => allow_index,
            Self::String | Self::Keyword | Self::Unknown | Self::Never => true,
            key => key.is_literal_map_key(),
        }
    }

    pub fn key_overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Never, _) | (_, Self::Never) => false,
            (Self::Unknown | Self::Parameter(_), _) | (_, Self::Unknown | Self::Parameter(_)) => {
                true
            }
            (Self::Union(parts), other) => parts.iter().any(|key| key.key_overlaps(other)),
            (key, Self::Union(parts)) => parts.iter().any(|other| key.key_overlaps(other)),
            (Self::Integer(a), Self::Integer(b)) => a == b,
            (key, other) if let (Some(a), Some(b)) = (key.integer_type(), other.integer_type()) => {
                // Numeric map keys can compare equal across integer storage classes.
                a.range == b.range
                    || matches!(a.range, IntegerRange::Any)
                    || matches!(b.range, IntegerRange::Any)
            }
            (key, other) => key.subtype_of(other) || other.subtype_of(key),
        }
    }
}

impl MapType {
    pub fn dictionary(key: Type, value: Type) -> Self {
        Self {
            fields: BTreeMap::new(),
            extra: if key == Type::Never || value == Type::Never {
                ExtraFields::Closed
            } else {
                ExtraFields::Typed(Box::new([key, value]))
            },
        }
    }

    pub fn slot(&self, key: &Type) -> MapSlot<'_> {
        match self.fields.get(key) {
            Some(field) if field.required => MapSlot::Present(&field.ty),
            Some(field) if field.ty != Type::Never => MapSlot::Optional(&field.ty),
            Some(_) => MapSlot::Absent,
            None => match &self.extra {
                ExtraFields::Closed => MapSlot::Absent,
                ExtraFields::Open => MapSlot::Undeclared,
                ExtraFields::Typed(types) if key.key_overlaps(&types[0]) => {
                    MapSlot::Optional(&types[1])
                }
                ExtraFields::Typed(_) => MapSlot::Absent,
            },
        }
    }

    pub(crate) fn merge(&self, right: &Self) -> Self {
        let extra = match (&self.extra, &right.extra) {
            (ExtraFields::Open, _) | (_, ExtraFields::Open) => ExtraFields::Open,
            (ExtraFields::Closed, other) | (other, ExtraFields::Closed) => other.clone(),
            (ExtraFields::Typed(a), ExtraFields::Typed(b)) => ExtraFields::Typed(Box::new([
                Type::union([a[0].clone(), b[0].clone()]),
                Type::union([a[1].clone(), b[1].clone()]),
            ])),
        };
        let mut result = Self {
            fields: BTreeMap::new(),
            extra,
        };
        for key in self
            .fields
            .keys()
            .chain(right.fields.keys())
            .collect::<BTreeSet<_>>()
        {
            let slot = |slot| match slot {
                MapSlot::Present(ty) => (ty.clone(), true),
                MapSlot::Optional(ty) => (ty.clone(), false),
                MapSlot::Absent => (Type::Never, false),
                MapSlot::Undeclared => (Type::Unknown, false),
            };
            let (right_ty, required) = slot(right.slot(key));
            let field = if required {
                Field::new(right_ty, required)
            } else {
                let (left_ty, required) = slot(self.slot(key));
                let ty = if left_ty == Type::Unknown || right_ty == Type::Unknown {
                    Type::Unknown
                } else {
                    Type::union([left_ty, right_ty])
                };
                Field::new(ty, required)
            };
            result.set_field(key.clone(), field);
        }
        result
    }

    pub fn set_field(&mut self, key: Type, field: Field) {
        self.exclude_extra_key(&key);
        self.fields.insert(key, field);
    }

    fn exclude_extra_key(&mut self, key: &Type) {
        if let ExtraFields::Typed(types) = &mut self.extra {
            // A fixed field replaces that entry of a finite key domain.
            let keys = &mut types[0];
            if *keys == *key {
                *keys = Type::Never;
            } else if let Type::Union(parts) = keys {
                let previous_len = parts.len();
                parts.retain(|part| part != key);
                if parts.len() != previous_len {
                    *keys = Type::union(std::mem::take(parts));
                }
            }
            if *keys == Type::Never {
                self.extra = ExtraFields::Closed;
            }
        }
    }

    /// Possible values of a nonconstant key. Open shapes can contain undeclared values.
    pub fn possible_values(&self, key: &Type) -> Option<Type> {
        let extra_value = match &self.extra {
            ExtraFields::Open => return None,
            ExtraFields::Typed(types) if key.key_overlaps(&types[0]) => Some(types[1].clone()),
            _ => None,
        };
        if self.fields.is_empty() {
            return Some(extra_value.unwrap_or(Type::Never));
        }
        let values = self
            .fields
            .iter()
            .filter(|(field_key, _)| key.key_overlaps(field_key))
            .map(|(_, field)| field.ty.clone());
        Some(Type::union(values.chain(extra_value)))
    }

    pub fn associate(&mut self, key: &Type, value: &Type) {
        if key.is_literal_map_key() {
            self.set_field(key.clone(), Field::new(value.clone(), true));
            return;
        }
        // A dynamic key may replace any overlapping fixed field or create a new entry.
        for (field_key, field) in &mut self.fields {
            if field.check_value && key.key_overlaps(field_key) {
                field.ty =
                    Type::union([std::mem::replace(&mut field.ty, Type::Never), value.clone()]);
            }
        }
        match &mut self.extra {
            ExtraFields::Open => {}
            ExtraFields::Closed => {
                self.extra = ExtraFields::Typed(Box::new([key.clone(), value.clone()]))
            }
            ExtraFields::Typed(types) => {
                let [old_key, old_value] = types.as_mut();
                if !key.subtype_of(old_key) {
                    *old_key = Type::union([std::mem::replace(old_key, Type::Never), key.clone()]);
                }
                if !value.subtype_of(old_value) {
                    *old_value =
                        Type::union([std::mem::replace(old_value, Type::Never), value.clone()]);
                }
            }
        }
    }

    pub(crate) fn dissociate(&mut self, key: &Type) {
        if key.is_literal_map_key() {
            self.exclude_extra_key(key);
            self.fields.remove(key);
            if !matches!(self.slot(key), MapSlot::Absent) {
                // Mask this key when the remaining extra domain still admits it.
                self.fields
                    .insert(key.clone(), Field::new(Type::Never, false));
            }
        } else {
            for (name, field) in &mut self.fields {
                if field.required && key.key_overlaps(name) {
                    field.required = false;
                }
            }
        }
    }

    pub fn into_entry_type(self) -> Option<Type> {
        let additional = match self.extra {
            ExtraFields::Closed => None,
            ExtraFields::Open => return None,
            ExtraFields::Typed(types) => Some(Type::MapEntry(types)),
        };
        if self.fields.is_empty() {
            return Some(additional.unwrap_or(Type::Never));
        }
        Some(Type::union(
            self.fields
                .into_iter()
                .filter(|(_, field)| field.ty != Type::Never)
                .map(|(key, field)| Type::MapEntry(Box::new([key, field.ty])))
                .chain(additional),
        ))
    }

    pub(super) fn inferred(&self) -> Self {
        Self {
            fields: self
                .fields
                .iter()
                .map(|(key, field)| {
                    (
                        key.clone(),
                        Field {
                            ty: field.ty.inferred(),
                            required: field.required,
                            check_value: field.check_value,
                        },
                    )
                })
                .collect(),
            extra: self.extra.inferred(),
        }
    }

    pub(super) fn any_type(&self, predicate: impl Fn(&Type) -> bool) -> bool {
        self.fields.values().any(|field| predicate(&field.ty))
            || matches!(&self.extra, ExtraFields::Typed(types) if types.iter().any(&predicate))
    }

    pub(super) fn compatibility(&self, expected: &Self) -> Compatibility {
        expected
            .extra
            .accepts_type(&self.extra)
            .and(|| {
                Compatibility::all(
                    expected
                        .fields
                        .iter()
                        .map(|(key, field)| self.slot(key).compatibility(field)),
                )
            })
            .and(|| {
                if expected.extra.is_open() {
                    Compatibility::Proven
                } else {
                    Compatibility::all(
                        self.unlisted_fields(expected)
                            .map(|(key, field)| expected.extra.accepts_field(key, &field.ty)),
                    )
                }
            })
    }

    pub(super) fn unlisted_fields<'a>(
        &'a self,
        expected: &'a Self,
    ) -> impl Iterator<Item = (&'a Type, &'a Field)> {
        self.fields
            .iter()
            .filter(|(key, field)| field.ty != Type::Never && !expected.fields.contains_key(*key))
    }

    pub(super) fn is_shape(&self) -> bool {
        !matches!(self.extra, ExtraFields::Typed(_))
    }

    pub(super) fn join_values(mut self, mut other: Self) -> Self {
        self.fields.retain(|key, field| {
            if let Some(right) = other.fields.remove(key) {
                let required = field.required && right.required;
                *field = if field.check_value && right.check_value {
                    let left = std::mem::replace(&mut field.ty, Type::Never);
                    Field::new(left.join_values(right.ty), required)
                } else {
                    Field::presence_only(required)
                };
                true
            } else if other.extra.is_open() {
                // The other path can have an arbitrary undeclared value here.
                // Dropping the declaration keeps a later lookup an explicit gap.
                false
            } else {
                field.required = false;
                true
            }
        });
        if !self.extra.is_open() {
            for (key, mut field) in other.fields {
                field.required = false;
                self.fields.insert(key, field);
            }
        }
        if other.extra.is_open() {
            self.extra = ExtraFields::Open;
        }
        self
    }
}

impl fmt::Display for MapType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let ExtraFields::Typed(types) = &self.extra
            && self.fields.is_empty()
        {
            return write!(f, "Map<{}, {}>", types[0], types[1]);
        }
        let mut parts = self
            .fields
            .iter()
            .map(|(key, field)| {
                format!(
                    "{key}{}: {}",
                    if field.required { "" } else { "?" },
                    field.ty
                )
            })
            .collect::<Vec<_>>();
        match &self.extra {
            ExtraFields::Closed => {}
            ExtraFields::Open => parts.push("...".into()),
            ExtraFields::Typed(types) => parts.push(format!("...Map<{}, {}>", types[0], types[1])),
        }
        write!(f, "{{{}}}", parts.join(", "))
    }
}
