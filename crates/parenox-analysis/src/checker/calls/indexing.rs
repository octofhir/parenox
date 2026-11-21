//! Strict indexing through clojure.core/nth and vector invocation.
use super::super::{Checker, LiteralContainer, Value, literal_vector_value};
use crate::Diagnostic;
use crate::reader::Span;
use crate::types::{IndexedItem, IntegerRange, IntegerType, Type};

#[derive(Clone, Copy)]
enum IndexedAccess {
    Nth,
    VectorCall,
}

impl IndexedAccess {
    fn name(self) -> &'static str {
        match self {
            Self::Nth => "clojure.core/nth",
            Self::VectorCall => "Vector invocation",
        }
    }
}

impl Checker<'_> {
    pub(super) fn nth(&mut self, args: &[(Value, Span)]) -> Value {
        let (collection, collection_span) = &args[0];
        let (key, key_span) = &args[1];
        let fallback = args.get(2).map(|(value, _)| value);
        let index = exact_integer_index(&key.known);
        let proof = match (&collection.known, &collection.literal_container) {
            (Type::Tuple(items), Some(vector @ LiteralContainer::Vector(_))) => {
                Some((items, vector))
            }
            _ => None,
        };
        if self.partial_scope.is_some()
            && (index.is_none() || (proof.is_none() && collection.known != Type::Nil))
        {
            return Value::new(Type::Unknown);
        }
        let mut result = Value::new(self.nth_value(
            (&collection.known, *collection_span),
            (&key.known, *key_span),
            fallback.map(|value| &value.known),
            IndexedAccess::Nth,
        ));
        if let Some(index) = index {
            let fallback_proof = || fallback.and_then(|value| value.literal_container.as_ref());
            let selected = if collection.known == Type::Nil {
                fallback_proof()
            } else if let Some((items, vector)) = proof {
                match usize::try_from(index) {
                    Ok(index) if index < items.len() => vector.position_child(index),
                    _ => fallback_proof(),
                }
            } else {
                None
            };
            result.literal_container = selected.cloned();
        }
        result
    }

    pub(super) fn subvec(&mut self, args: &[(Value, Span)]) -> Value {
        let (source, source_span) = &args[0];
        let (Type::Tuple(items), Some(vector @ LiteralContainer::Vector(_))) =
            (&source.known, &source.literal_container)
        else {
            return self.unchecked_form(*source_span, "subvec needs a proven ordinary vector");
        };
        let (start, start_span) = &args[1];
        let Some(start) = exact_integer_index(&start.known) else {
            return self.unchecked_form(*start_span, "subvec needs a known i32 start index");
        };
        let length = items.len() as i32;
        let mut end = length;
        let mut end_span = *start_span;
        if let Some((bound, span)) = args.get(2) {
            let Some(index) = exact_integer_index(&bound.known) else {
                return self.unchecked_form(*span, "subvec needs a known i32 end index");
            };
            end = index;
            end_span = *span;
        }

        let invalid = if !(0..=length).contains(&start) {
            Some(*start_span)
        } else if end < start || end > length {
            Some(end_span)
        } else {
            None
        };
        if let Some(span) = invalid {
            let mut diagnostic = Diagnostic::new(
                "PNX2801",
                span,
                format!("Slice {start}..{end} is out of bounds for a vector of length {length}"),
            );
            diagnostic.help = Some(
                "Keep both indices within the vector and put the start before or at the end."
                    .into(),
            );
            self.diagnostics.push(diagnostic);
            self.index_out_of_bounds();
            return Value::new(Type::Never);
        }

        let start = start as usize;
        let selected = &items[start..end as usize];
        let contents = selected.iter().enumerate().map(|(offset, item)| {
            let mut value = Value::new(item.clone());
            value.literal_container = vector.position_child(start + offset).cloned();
            value
        });
        let mut result = literal_vector_value(contents, selected.len(), true);
        result.notes.clone_from(&source.notes);
        result
    }

    pub(super) fn vector_call(
        &mut self,
        vector: &Value,
        head: Span,
        args: &[(Value, Span)],
        span: Span,
    ) -> Value {
        let [(key, key_span)] = args else {
            self.error(
                "PNX2554",
                span,
                format!("Vector invocation expects 1 argument, got {}", args.len()),
            );
            return Value::new(Type::Unknown);
        };
        Value::new(self.nth_value(
            (&vector.known, head),
            (&key.known, *key_span),
            None,
            IndexedAccess::VectorCall,
        ))
    }

    fn nth_value(
        &mut self,
        collection: (&Type, Span),
        key: (&Type, Span),
        fallback: Option<&Type>,
        access: IndexedAccess,
    ) -> Type {
        match key.0 {
            Type::Never | Type::Unknown => key.0.clone(),
            Type::Any => {
                self.may_throw();
                Type::Unknown
            }
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.nth_value(collection, (part, key.1), fallback, access)),
            ),
            Type::Integer(index) => self.nth_at(collection, (*index, key.1), fallback, access),
            Type::Integral(IntegerType {
                range: IntegerRange::Exact(index),
                ..
            }) if matches!(access, IndexedAccess::Nth) => {
                self.nth_at(collection, (*index, key.1), fallback, access)
            }
            Type::NonIntegral if matches!(access, IndexedAccess::VectorCall) => {
                self.invalid_index(key, access)
            }
            Type::Number
            | Type::Integral(IntegerType {
                range:
                    IntegerRange::IntCache
                    | IntegerRange::Int32
                    | IntegerRange::Exact(_)
                    | IntegerRange::ExactLong(_)
                    | IntegerRange::OutsideInt32,
                ..
            }) if matches!(access, IndexedAccess::VectorCall) => {
                let source = &self.source[key.1.start.offset..key.1.end.offset];
                if let Some(index) = vector_literal_index(source) {
                    self.nth_at(collection, (index, key.1), fallback, access)
                } else {
                    self.unsupported(
                        key.1,
                        "Vector invocation analysis needs a known integer index",
                    )
                    .known
                }
            }
            Type::Integral(_) if matches!(access, IndexedAccess::VectorCall) => {
                self.unsupported(
                    key.1,
                    "Vector invocation analysis needs a known integer index",
                )
                .known
            }
            Type::Number | Type::Integral(_) | Type::NonIntegral => {
                self.unsupported(
                    key.1,
                    "nth analysis requires a known integer index in the i32 range",
                )
                .known
            }
            _ => self.invalid_index(key, access),
        }
    }

    fn invalid_index(&mut self, key: (&Type, Span), access: IndexedAccess) -> Type {
        let expected = match access {
            IndexedAccess::Nth => "Number",
            IndexedAccess::VectorCall => "Integer",
        };
        self.diagnostics.push(Diagnostic::type_mismatch(
            key.1,
            &format!("{} index", access.name()),
            expected.into(),
            key.0.inferred().to_string(),
        ));
        Type::Unknown
    }

    fn nth_at(
        &mut self,
        collection: (&Type, Span),
        index: (i32, Span),
        fallback: Option<&Type>,
        access: IndexedAccess,
    ) -> Type {
        let (ty, span) = collection;
        match ty {
            Type::Nil => return fallback.cloned().unwrap_or(Type::Nil),
            Type::Never | Type::Unknown => return ty.clone(),
            Type::Any => {
                self.may_throw();
                return Type::Unknown;
            }
            Type::Union(parts) => {
                return Type::union(
                    parts
                        .iter()
                        .map(|part| self.nth_at((part, span), index, fallback, access)),
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
                    None => {
                        let operation = match access {
                            IndexedAccess::Nth => "nth",
                            IndexedAccess::VectorCall => "vector invocation",
                        };
                        format!(
                            "Index {} is negative; {operation} requires a nonnegative index",
                            index.0
                        )
                    }
                };
                let mut diagnostic = Diagnostic::new("PNX2801", index.1, message);
                diagnostic.help = Some(
                    match access {
                        IndexedAccess::Nth => {
                            "Use an in-range index or pass a fallback as the third argument."
                        }
                        IndexedAccess::VectorCall => "Use an in-range integer index.",
                    }
                    .into(),
                );
                self.diagnostics.push(diagnostic);
                Type::Never
            }
        }
    }
}

fn exact_integer_index(ty: &Type) -> Option<i32> {
    match ty {
        Type::Integer(index)
        | Type::Integral(IntegerType {
            range: IntegerRange::Exact(index),
            ..
        }) => Some(*index),
        _ => None,
    }
}

fn vector_literal_index(token: &str) -> Option<i32> {
    let digits = token.strip_prefix(['+', '-']).unwrap_or(token);
    let value = if digits.len() > 1 && digits.starts_with('0') {
        let value = i64::from_str_radix(digits, 8).ok()?;
        if token.starts_with('-') {
            -value
        } else {
            value
        }
    } else {
        token.parse::<i64>().ok()?
    };
    // APersistentVector uses Number.intValue() after checking the JVM type.
    Some(value as i32)
}
