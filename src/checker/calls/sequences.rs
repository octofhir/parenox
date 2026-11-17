//! The vector and seq operations used to traverse a collection in a loop.
use super::super::{Checker, Value};
use crate::Diagnostic;
use crate::reader::Span;
use crate::types::Type;

#[derive(Clone, Copy)]
enum ReadSequence {
    Start,
    Head,
    Tail,
}

impl ReadSequence {
    fn fixed(self, items: &[Type]) -> Type {
        match self {
            Self::Start => nonempty_seq(items),
            Self::Head => items.first().cloned().unwrap_or(Type::Nil),
            Self::Tail => nonempty_seq(items.get(1..).unwrap_or_default()),
        }
    }

    fn open(self, element: &Type, nonempty: bool) -> Type {
        let result = match self {
            Self::Head => element.clone(),
            Self::Start | Self::Tail => Type::Seq(Box::new(element.clone())),
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
    pub(super) fn sequence_call(&mut self, name: &str, (value, span): &(Value, Span)) -> Value {
        let operation = match name {
            "clojure.core/seq" => ReadSequence::Start,
            "clojure.core/first" => ReadSequence::Head,
            "clojure.core/next" => ReadSequence::Tail,
            _ => unreachable!(),
        };
        let result = self.read_sequence(operation, &value.known, name, *span);
        Value::with_notes(result, value.notes.clone())
    }

    fn read_sequence(&mut self, op: ReadSequence, ty: &Type, name: &str, span: Span) -> Type {
        match ty {
            Type::Nil | Type::Never | Type::Unknown => ty.clone(),
            Type::Tuple(items) | Type::SeqTuple(items) => op.fixed(items),
            Type::MapEntry(items) => op.fixed(items.as_ref()),
            Type::Vector(element) if **element == Type::Never => Type::Nil,
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
                    format!("{name} analysis currently supports vectors, nonempty seqs and Nil"),
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
