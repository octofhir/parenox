//! Positions for the modeled Clojure indexed collections.
use super::Type;

pub enum IndexedItem<'a> {
    Present(&'a Type),
    Optional(&'a Type),
    Absent(Option<usize>),
}

impl Type {
    pub fn sequential_item(&self, index: usize) -> Option<Type> {
        match self {
            Type::Nil | Type::Unknown | Type::Never => Some(self.clone()),
            Type::Union(parts) => parts
                .iter()
                .map(|part| part.sequential_item(index))
                .collect::<Option<Vec<_>>>()
                .map(Type::union),
            _ => match self.indexed_item(i32::try_from(index).ok()?)? {
                IndexedItem::Present(item) => Some(item.clone()),
                IndexedItem::Optional(item) => Some(item.clone().nilable()),
                IndexedItem::Absent(_) => Some(Type::Nil),
            },
        }
    }

    pub fn indexed_item(&self, index: i32) -> Option<IndexedItem<'_>> {
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
            Type::Str(text) => {
                let length = text.encode_utf16().count();
                return Some(if position.is_some_and(|index| index < length) {
                    IndexedItem::Present(&Type::Character)
                } else {
                    IndexedItem::Absent(Some(length))
                });
            }
            Type::String => {
                return Some(match position {
                    Some(_) => IndexedItem::Optional(&Type::Character),
                    None => IndexedItem::Absent(None),
                });
            }
            _ => return None,
        };
        Some(match position.and_then(|index| fixed.get(index)) {
            Some(item) => IndexedItem::Present(item),
            None => IndexedItem::Absent(Some(fixed.len())),
        })
    }
}
