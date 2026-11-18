//! Vector and sequence traversal and accumulation.
use super::super::{Checker, Value};
use crate::Diagnostic;
use crate::reader::Span;
use crate::types::{MAX_TUPLE_ITEMS, Type};

#[derive(Clone, Copy)]
enum ReadSequence {
    Start,
    Head,
    Tail,
    Rest,
}

impl ReadSequence {
    fn fixed(self, items: &[Type]) -> Type {
        match self {
            Self::Start => nonempty_seq(items),
            Self::Head => items.first().cloned().unwrap_or(Type::Nil),
            Self::Tail => nonempty_seq(items.get(1..).unwrap_or_default()),
            Self::Rest => Type::SeqTuple(items.get(1..).unwrap_or_default().into()),
        }
    }

    fn open(self, element: &Type, nonempty: bool) -> Type {
        let result = match self {
            Self::Head => element.clone(),
            Self::Start | Self::Tail => Type::Seq(Box::new(element.clone())),
            Self::Rest => {
                return Type::union([Type::empty_seq(), Type::Seq(Box::new(element.clone()))]);
            }
        };
        if nonempty && !matches!(self, Self::Tail) {
            result
        } else {
            result.nilable()
        }
    }
}

fn nonempty_seq(items: &[Type]) -> Type {
    if items.is_empty() {
        Type::Nil
    } else {
        Type::SeqTuple(items.into())
    }
}

impl Checker<'_> {
    pub(super) fn conj(&mut self, args: &[(Value, Span)]) -> Value {
        let Some(((collection, span), items)) = args.split_first() else {
            return Value::new(Type::Tuple(Box::new([])));
        };
        if items.is_empty() {
            return collection.clone();
        }
        let items: Vec<_> = items.iter().map(|(value, _)| value.known.clone()).collect();
        let result = self.conj_type(&collection.known, &items, *span);
        Value::with_notes(result, collection.notes.clone())
    }

    fn conj_type(&mut self, collection: &Type, items: &[Type], span: Span) -> Type {
        let joined = |element: &Type| {
            Box::new(Type::union(
                std::iter::once(element.clone()).chain(items.iter().cloned()),
            ))
        };
        match collection {
            Type::Unknown | Type::Never => collection.clone(),
            Type::Nil => conj_seq(&[], items),
            Type::SeqTuple(previous) => conj_seq(previous, items),
            Type::Seq(element) => Type::Seq(joined(element)),
            Type::Vector(element) => Type::Vector(joined(element)),
            Type::Set(element) => Type::Set(joined(element)),
            Type::Tuple(previous) => {
                let appended = previous.iter().chain(items).cloned();
                if previous.len() + items.len() <= MAX_TUPLE_ITEMS {
                    Type::Tuple(appended.collect())
                } else {
                    Type::Vector(Box::new(Type::union(appended)))
                }
            }
            Type::MapEntry(previous) => self.conj_type(
                &Type::Tuple(previous.as_ref().as_slice().into()),
                items,
                span,
            ),
            Type::Union(parts) => {
                Type::union(parts.iter().map(|part| self.conj_type(part, items, span)))
            }
            Type::Map(_) => {
                self.unsupported(span, "conj analysis for maps is not supported yet")
                    .known
            }
            other => {
                self.diagnostics.push(Diagnostic::type_mismatch(
                    span,
                    "clojure.core/conj",
                    "Collection or Nil".into(),
                    other.inferred().to_string(),
                ));
                Type::Unknown
            }
        }
    }

    pub(super) fn sequence_call(&mut self, name: &str, (value, span): &(Value, Span)) -> Value {
        let operation = match name {
            "clojure.core/seq" => ReadSequence::Start,
            "clojure.core/first" => ReadSequence::Head,
            "clojure.core/next" => ReadSequence::Tail,
            "clojure.core/rest" => ReadSequence::Rest,
            _ => unreachable!(),
        };
        let result = self.read_sequence(operation, &value.known, name, *span);
        Value::with_notes(result, value.notes.clone())
    }

    fn read_sequence(&mut self, op: ReadSequence, ty: &Type, name: &str, span: Span) -> Type {
        match ty {
            Type::Never | Type::Unknown => ty.clone(),
            Type::Nil => op.fixed(&[]),
            Type::Tuple(items) | Type::SeqTuple(items) => op.fixed(items),
            Type::MapEntry(items) => op.fixed(items.as_ref()),
            Type::Vector(element) if **element == Type::Never => op.fixed(&[]),
            Type::Vector(element) => op.open(element, false),
            Type::Seq(element) => op.open(element, true),
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.read_sequence(op, part, name, span)),
            ),
            Type::Map(_) | Type::Set(_) | Type::Str(_) | Type::String => {
                self.unsupported(
                    span,
                    format!("{name} analysis currently supports vectors, seqs and Nil"),
                )
                .known
            }
            actual => {
                self.diagnostics.push(Diagnostic::type_mismatch(
                    span,
                    name,
                    "Seqable or Nil".into(),
                    actual.inferred().to_string(),
                ));
                Type::Unknown
            }
        }
    }
}

fn conj_seq(previous: &[Type], items: &[Type]) -> Type {
    let prepended = items.iter().rev().chain(previous).cloned();
    if previous.len() + items.len() <= MAX_TUPLE_ITEMS {
        Type::SeqTuple(prepended.collect())
    } else {
        Type::Seq(Box::new(Type::union(prepended)))
    }
}
