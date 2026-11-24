//! Positions for the modeled Clojure indexed collections.
use super::{IntegerClass, IntegerRange, IntegerType, LengthBounds, Type};

pub enum IndexedItem<'a> {
    Present(&'a Type),
    PresentChar(char),
    Optional(&'a Type),
    Absent(Option<usize>),
}

impl IndexedItem<'_> {
    fn with_bounds(self, position: Option<usize>, bounds: LengthBounds) -> Self {
        match (position, self) {
            (Some(index), Self::Optional(element)) if index < bounds.min => Self::Present(element),
            (Some(index), _) if bounds.max == Some(bounds.min) && index >= bounds.min => {
                Self::Absent(Some(bounds.min))
            }
            (_, item) => item,
        }
    }
}

impl Type {
    /// The position when an associative operation accepts this numeric class.
    pub fn associative_index(&self) -> Option<i32> {
        match self {
            Self::Integer(index) => Some(*index),
            Self::Integral(_) if self.is_lookup_key(true) => {
                self.exact_long_value().map(|value| value as i32)
            }
            _ => None,
        }
    }

    /// Vectors reject Byte and Short keys even when their value fits an index.
    pub fn may_be_byte_or_short(&self) -> bool {
        matches!(
            self.integer_type(),
            Some(IntegerType {
                class: IntegerClass::Any | IntegerClass::Fixed,
                range: IntegerRange::Exact(value),
            }) if i16::try_from(value).is_ok()
        )
    }

    pub fn sequential_item(&self, index: usize) -> Option<Type> {
        match self {
            Type::Nil | Type::Unknown | Type::Never => Some(self.clone()),
            Type::Union(parts) => parts
                .iter()
                .map(|part| part.sequential_item(index))
                .collect::<Option<Vec<_>>>()
                .map(Type::union),
            _ => match self.nth_item(i32::try_from(index).ok()?)? {
                IndexedItem::Present(item) => Some(item.clone()),
                IndexedItem::PresentChar(value) => Some(Type::Char(value)),
                IndexedItem::Optional(item) => Some(item.clone().nilable()),
                IndexedItem::Absent(_) => Some(Type::Nil),
            },
        }
    }

    pub fn nth_item(&self, index: i32) -> Option<IndexedItem<'_>> {
        match self {
            Type::Seq(element) | Type::List(element) => Some(match index {
                0 => IndexedItem::Present(element),
                1.. => IndexedItem::Optional(element),
                _ => IndexedItem::Absent(None),
            }),
            Type::SeqTuple(items) | Type::ListTuple(items) => {
                Some(fixed_item(items, usize::try_from(index).ok()))
            }
            Type::BoundedCollection(collection, bounds) => Some(
                collection
                    .nth_item(index)?
                    .with_bounds(usize::try_from(index).ok(), *bounds),
            ),
            _ => self.indexed_item(index),
        }
    }

    pub fn indexed_item(&self, index: i32) -> Option<IndexedItem<'_>> {
        if let Type::Parameter(parameter) = self {
            return parameter.upper.as_ref()?.indexed_item(index);
        }
        let position = usize::try_from(index).ok();
        let fixed = match self {
            Type::Tuple(items) => items.as_ref(),
            Type::MapEntry(items) => items.as_ref(),
            Type::Vector(element) if **element == Type::Never => &[],
            Type::Vector(element) => {
                return Some(match position {
                    Some(_) => IndexedItem::Optional(element),
                    None => IndexedItem::Absent(None),
                });
            }
            Type::BoundedCollection(collection, bounds) => {
                return Some(
                    collection
                        .indexed_item(index)?
                        .with_bounds(position, *bounds),
                );
            }
            Type::Str(text) => {
                let mut length = 0;
                for (index, unit) in text.encode_utf16().enumerate() {
                    if position == Some(index) {
                        return Some(char::from_u32(u32::from(unit)).map_or(
                            IndexedItem::Present(&Type::Character),
                            IndexedItem::PresentChar,
                        ));
                    }
                    length = index + 1;
                }
                return Some(IndexedItem::Absent(Some(length)));
            }
            Type::String => {
                return Some(match position {
                    Some(_) => IndexedItem::Optional(&Type::Character),
                    None => IndexedItem::Absent(None),
                });
            }
            _ => return None,
        };
        Some(fixed_item(fixed, position))
    }
}

fn fixed_item(items: &[Type], index: Option<usize>) -> IndexedItem<'_> {
    match index.and_then(|index| items.get(index)) {
        Some(item) => IndexedItem::Present(item),
        None => IndexedItem::Absent(Some(items.len())),
    }
}
