//! Map shapes and types for their additional entries.
use std::{collections::BTreeMap, fmt};

use super::Type;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MapType {
    pub fields: BTreeMap<String, Field>,
    pub extra: ExtraFields,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Field {
    pub ty: Type,
    pub required: bool,
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

    fn accepts_type(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Open, _) | (_, Self::Closed) => true,
            (Self::Typed(expected), Self::Typed(actual)) => actual
                .iter()
                .zip(expected.iter())
                .all(|(actual, expected)| actual.subtype_of(expected)),
            _ => false,
        }
    }

    fn accepts_field(&self, key: &str, ty: &Type) -> bool {
        match self {
            Self::Open => true,
            Self::Closed => false,
            Self::Typed(types) => accepts_keyword(&types[0], key) && ty.subtype_of(&types[1]),
        }
    }
}

fn accepts_keyword(domain: &Type, key: &str) -> bool {
    match domain {
        Type::Keyword => true,
        Type::Key(name) => name == key,
        Type::Union(parts) => parts.iter().any(|part| accepts_keyword(part, key)),
        _ => false,
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

    pub fn slot(&self, key: &str) -> MapSlot<'_> {
        match self.fields.get(key) {
            Some(field) if field.required => MapSlot::Present(&field.ty),
            Some(field) if field.ty != Type::Never => MapSlot::Optional(&field.ty),
            Some(_) => MapSlot::Absent,
            None => match &self.extra {
                ExtraFields::Closed => MapSlot::Absent,
                ExtraFields::Open => MapSlot::Undeclared,
                ExtraFields::Typed(types)
                    if accepts_keyword(&types[0], key) || types[0].contains_unknown() =>
                {
                    MapSlot::Optional(&types[1])
                }
                ExtraFields::Typed(_) => MapSlot::Absent,
            },
        }
    }

    pub fn set_field(&mut self, key: String, field: Field) {
        if let ExtraFields::Typed(types) = &mut self.extra {
            // A fixed field replaces that entry of a finite key domain.
            let keys = &mut types[0];
            match keys {
                Type::Key(existing) if *existing == key => *keys = Type::Never,
                Type::Union(parts) => {
                    let previous_len = parts.len();
                    parts.retain(|part| !matches!(part, Type::Key(existing) if *existing == key));
                    if parts.len() != previous_len {
                        *keys = Type::union(std::mem::take(parts));
                    }
                }
                _ => {}
            }
            if *keys == Type::Never {
                self.extra = ExtraFields::Closed;
            }
        }
        self.fields.insert(key, field);
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
                .map(|(key, field)| Type::MapEntry(Box::new([Type::Key(key), field.ty])))
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
                        },
                    )
                })
                .collect(),
            extra: self.extra.inferred(),
        }
    }

    pub(super) fn any_type(&self, predicate: fn(&Type) -> bool) -> bool {
        self.fields.values().any(|field| predicate(&field.ty))
            || matches!(&self.extra, ExtraFields::Typed(types) if types.iter().any(predicate))
    }

    pub(super) fn subtype_of(&self, expected: &Self) -> bool {
        expected.extra.accepts_type(&self.extra)
            && expected
                .fields
                .iter()
                .all(|(key, field)| match self.slot(key) {
                    MapSlot::Present(actual) => actual.subtype_of(&field.ty),
                    MapSlot::Optional(actual) => !field.required && actual.subtype_of(&field.ty),
                    MapSlot::Absent => !field.required,
                    MapSlot::Undeclared => false,
                })
            && (expected.extra.is_open()
                || self.fields.iter().all(|(key, field)| {
                    field.ty == Type::Never
                        || expected.fields.contains_key(key)
                        || expected.extra.accepts_field(key, &field.ty)
                }))
    }

    pub(super) fn is_shape(&self) -> bool {
        !matches!(self.extra, ExtraFields::Typed(_))
    }

    pub(super) fn join_values(mut self, mut other: Self) -> Self {
        self.fields.retain(|key, field| {
            if let Some(right) = other.fields.remove(key) {
                let left = std::mem::replace(&mut field.ty, Type::Never);
                field.ty = left.join_values(right.ty);
                field.required &= right.required;
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
