//! Strict indexed access through clojure.core/nth.
use super::super::{Checker, Value};
use crate::Diagnostic;
use crate::reader::Span;
use crate::types::{IndexedItem, Type};

impl Checker<'_> {
    pub(super) fn nth(&mut self, args: &[(Value, Span)]) -> Value {
        let (collection, collection_span) = &args[0];
        let (key, key_span) = &args[1];
        let fallback = args.get(2).map(|(value, _)| &value.known);
        Value::new(self.nth_value(
            (&collection.known, *collection_span),
            (&key.known, *key_span),
            fallback,
        ))
    }

    fn nth_value(
        &mut self,
        collection: (&Type, Span),
        key: (&Type, Span),
        fallback: Option<&Type>,
    ) -> Type {
        match key.0 {
            Type::Never | Type::Unknown => key.0.clone(),
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.nth_value(collection, (part, key.1), fallback)),
            ),
            Type::Integer(index) => self.nth_at(collection, (*index, key.1), fallback),
            Type::Number => {
                self.unsupported(
                    key.1,
                    "nth analysis requires a known decimal integer index in the i32 range",
                )
                .known
            }
            _ => {
                self.diagnostics.push(Diagnostic::type_mismatch(
                    key.1,
                    "clojure.core/nth index",
                    "Number".into(),
                    key.0.inferred().to_string(),
                ));
                Type::Unknown
            }
        }
    }

    fn nth_at(
        &mut self,
        collection: (&Type, Span),
        index: (i32, Span),
        fallback: Option<&Type>,
    ) -> Type {
        let (ty, span) = collection;
        match ty {
            Type::Nil => return fallback.cloned().unwrap_or(Type::Nil),
            Type::Never | Type::Unknown => return ty.clone(),
            Type::Union(parts) => {
                return Type::union(
                    parts
                        .iter()
                        .map(|part| self.nth_at((part, span), index, fallback)),
                );
            }
            _ => {}
        }
        let Some(item) = ty.nth_item(index.0) else {
            self.diagnostics.push(Diagnostic::type_mismatch(
                span,
                "clojure.core/nth",
                "Vector | Seq | String | Nil".into(),
                ty.inferred().to_string(),
            ));
            return Type::Unknown;
        };
        match item {
            IndexedItem::Present(item) => item.clone(),
            IndexedItem::Optional(item) => fallback.map_or_else(
                || item.clone(),
                |default| Type::union([item.clone(), default.clone()]),
            ),
            IndexedItem::Absent(length) => {
                if let Some(default) = fallback {
                    return default.clone();
                }
                let message = match length {
                    Some(length) => format!(
                        "Index {} is out of bounds for a collection of length {length}",
                        index.0
                    ),
                    None => format!(
                        "Index {} is negative; nth requires a nonnegative index",
                        index.0
                    ),
                };
                let mut diagnostic = Diagnostic::new("PNX2801", index.1, message);
                diagnostic.help =
                    Some("Use an in-range index or pass a fallback as the third argument.".into());
                self.diagnostics.push(diagnostic);
                Type::Never
            }
        }
    }
}
