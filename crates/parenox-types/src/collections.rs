//! Length constraints for Clojure collection types.
use crate::{Compatibility, ExtraFields, Type};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LengthBounds {
    pub min: usize,
    pub max: Option<usize>,
}

impl LengthBounds {
    pub const UNBOUNDED: Self = Self { min: 0, max: None };
    pub const NONEMPTY: Self = Self { min: 1, max: None };

    pub fn exact(count: usize) -> Self {
        Self {
            min: count,
            max: Some(count),
        }
    }

    pub fn includes(self, count: usize) -> bool {
        count >= self.min && self.max.is_none_or(|max| count <= max)
    }

    pub fn after_consuming(self, count: usize) -> Self {
        Self {
            min: self.min.saturating_sub(count),
            max: self.max.map(|max| max.saturating_sub(count)),
        }
    }

    fn cover(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.zip(other.max).map(|(left, right)| left.max(right)),
        }
    }

    pub(crate) fn compatibility(self, expected: Self) -> Compatibility {
        if self.max.is_some_and(|max| max < expected.min)
            || expected.max.is_some_and(|max| max < self.min)
        {
            Compatibility::Mismatch
        } else if self.min >= expected.min
            && expected
                .max
                .is_none_or(|max| self.max.is_some_and(|actual| actual <= max))
        {
            Compatibility::Proven
        } else {
            Compatibility::Unproven
        }
    }
}

impl Type {
    pub fn length_bounds(&self) -> LengthBounds {
        match self {
            Self::BoundedCollection(_, bounds) => *bounds,
            Self::Nil => LengthBounds::exact(0),
            Self::Tuple(items) | Self::SeqTuple(items) | Self::ListTuple(items) => {
                LengthBounds::exact(items.len())
            }
            Self::MapEntry(_) => LengthBounds::exact(2),
            Self::Seq(_) | Self::List(_) => LengthBounds::NONEMPTY,
            Self::Vector(item) | Self::Set(item) if **item == Self::Never => LengthBounds::exact(0),
            Self::Str(text) => LengthBounds::exact(text.encode_utf16().count()),
            Self::Map(map) => LengthBounds {
                min: map
                    .fields
                    .values()
                    .filter(|field| field.required && field.ty != Self::Never)
                    .count(),
                max: (map.extra == ExtraFields::Closed).then(|| {
                    map.fields
                        .values()
                        .filter(|field| field.ty != Self::Never)
                        .count()
                }),
            },
            Self::Union(parts) => parts
                .iter()
                .map(Self::length_bounds)
                .reduce(LengthBounds::cover)
                .unwrap_or(LengthBounds::UNBOUNDED),
            Self::Parameter(parameter) => parameter
                .upper
                .as_ref()
                .map_or(LengthBounds::UNBOUNDED, Self::length_bounds),
            _ => LengthBounds::UNBOUNDED,
        }
    }
}
