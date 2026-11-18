//! Vector and sequence traversal and accumulation.
use std::collections::BTreeMap;

use super::super::{Checker, Value};
use crate::Diagnostic;
use crate::reader::Span;
use crate::types::{ExtraFields, MAX_TUPLE_ITEMS, MapType, Type};

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

    fn open(self, element: &Type, minimum: usize) -> Type {
        let result = match self {
            Self::Head => element.clone(),
            Self::Start | Self::Tail | Self::Rest => Type::Seq(Box::new(element.clone())),
        };
        match self {
            Self::Rest if minimum < 2 => Type::union([Type::empty_seq(), result]),
            Self::Tail if minimum < 2 => result.nilable(),
            Self::Start | Self::Head if minimum == 0 => result.nilable(),
            _ => result,
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

pub(super) fn list_value(args: &[(Value, Span)]) -> Value {
    let items = args.iter().map(|(value, _)| value.known.clone());
    Value::new(if args.len() <= MAX_TUPLE_ITEMS {
        Type::SeqTuple(items.collect())
    } else {
        Type::Seq(Box::new(Type::union(items)))
    })
}

impl Checker<'_> {
    pub(super) fn cons(&mut self, args: &[(Value, Span)]) -> Value {
        let (item, _) = &args[0];
        let (tail, span) = &args[1];
        let sequence = self.seq_type(&tail.known, "clojure.core/cons", *span);
        if sequence.contains_unknown() {
            self.may_throw();
        }
        let result = self.conj_type(&sequence, std::slice::from_ref(&item.known), *span);
        Value::with_notes(result, tail.notes.clone())
    }

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

    pub(super) fn emptiness_call(&mut self, name: &str, (value, span): &(Value, Span)) -> Value {
        let Some(empty) = collection_emptiness(&value.known) else {
            self.diagnostics.push(Diagnostic::type_mismatch(
                *span,
                name,
                "Seqable or Nil".into(),
                value.ty.to_string(),
            ));
            return Value::new(Type::Unknown);
        };
        if name == "clojure.core/empty?" {
            return Value::with_notes(empty, value.notes.clone());
        }
        let nonempty = narrow_emptiness(&value.known, false);
        let result = if empty == Type::Bool(false) {
            nonempty
        } else {
            nonempty.nilable()
        };
        Value::with_notes(result, value.notes.clone())
    }

    pub(in crate::checker) fn sequence_call(
        &mut self,
        name: &str,
        value: &Value,
        span: Span,
    ) -> Value {
        if iteration_may_throw(&value.known) {
            self.may_throw();
        }
        let operation = match name {
            "clojure.core/seq" => ReadSequence::Start,
            "clojure.core/first" => ReadSequence::Head,
            "clojure.core/next" => ReadSequence::Tail,
            "clojure.core/rest" => ReadSequence::Rest,
            _ => unreachable!(),
        };
        let result = self.read_sequence(operation, &value.known, name, span);
        Value::with_notes(result, value.notes.clone())
    }

    pub(in crate::checker) fn seq_type(&mut self, ty: &Type, name: &str, span: Span) -> Type {
        self.read_sequence(ReadSequence::Start, ty, name, span)
    }

    pub(super) fn map_sequence_call(&mut self, name: &str, (value, span): &(Value, Span)) -> Value {
        let start = self.diagnostics.len();
        let result = self.read_map_component(&value.known, name, *span);
        self.dedup_diagnostics(start);
        Value::with_notes(result, value.notes.clone())
    }

    fn read_map_component(&mut self, ty: &Type, name: &str, span: Span) -> Type {
        match ty {
            Type::Nil | Type::Never => ty.clone(),
            Type::Unknown => {
                self.may_throw();
                Type::Unknown
            }
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.read_map_component(part, name, span)),
            ),
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.read_map_component(upper, name, span)
            }
            Type::Map(map) => {
                let keys = name == "clojure.core/keys";
                let additional = match &map.extra {
                    ExtraFields::Closed => None,
                    ExtraFields::Typed(pair) => Some(&pair[usize::from(!keys)]),
                    ExtraFields::Open => {
                        return self
                            .unsupported(
                                span,
                                format!("{name} cannot infer element types for an open map"),
                            )
                            .known;
                    }
                };
                let fields = map.fields.iter().filter_map(|(key, field)| {
                    (field.ty != Type::Never).then_some(if keys { key } else { &field.ty })
                });
                let item = Type::union(fields.chain(additional).cloned());
                map_items(ReadSequence::Start, map, item)
            }
            _ if collection_emptiness(ty).is_some() => {
                // JVM keys and vals can also traverse sequences of map entries.
                self.unsupported(
                    span,
                    format!("{name} analysis currently supports Map or Nil"),
                )
                .known
            }
            actual => {
                self.illegal_argument();
                self.diagnostics.push(Diagnostic::type_mismatch(
                    span,
                    name,
                    "Map or Nil".into(),
                    actual.inferred().to_string(),
                ));
                Type::Never
            }
        }
    }

    fn read_sequence(&mut self, op: ReadSequence, ty: &Type, name: &str, span: Span) -> Type {
        match ty {
            Type::Never | Type::Unknown => ty.clone(),
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.read_sequence(op, upper, name, span)
            }
            Type::Nil => op.fixed(&[]),
            Type::Tuple(items) | Type::SeqTuple(items) => op.fixed(items),
            Type::MapEntry(items) => op.fixed(items.as_ref()),
            Type::Vector(element) | Type::Set(element) if **element == Type::Never => op.fixed(&[]),
            Type::Vector(element) | Type::Set(element) => op.open(element, 0),
            Type::Seq(element) => op.open(element, 1),
            Type::Str(text) => {
                let items: Vec<_> = text
                    .encode_utf16()
                    .take(MAX_TUPLE_ITEMS + 1)
                    .map(|unit| char::from_u32(u32::from(unit)).map_or(Type::Character, Type::Char))
                    .collect();
                if items.len() <= MAX_TUPLE_ITEMS {
                    op.fixed(&items)
                } else {
                    op.open(&Type::Character, 2)
                }
            }
            Type::String => op.open(&Type::Character, 0),
            Type::Map(map) => self.read_map_sequence(op, map, name, span),
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.read_sequence(op, part, name, span)),
            ),
            actual => {
                self.illegal_argument();
                self.diagnostics.push(Diagnostic::type_mismatch(
                    span,
                    name,
                    "Seqable or Nil".into(),
                    actual.inferred().to_string(),
                ));
                Type::Never
            }
        }
    }

    fn read_map_sequence(
        &mut self,
        op: ReadSequence,
        map: &MapType,
        name: &str,
        span: Span,
    ) -> Type {
        let Some(entry) = map.clone().into_entry_type() else {
            return self
                .unsupported(
                    span,
                    format!("{name} cannot infer entry types for an open map"),
                )
                .known;
        };
        map_items(op, map, entry)
    }
}

fn map_items(op: ReadSequence, map: &MapType, item: Type) -> Type {
    let minimum = map
        .fields
        .values()
        .filter(|field| field.required && field.ty != Type::Never)
        .count();
    if matches!(map.extra, ExtraFields::Closed) {
        let possible = map
            .fields
            .values()
            .filter(|field| field.ty != Type::Never)
            .count();
        if possible == minimum && possible <= MAX_TUPLE_ITEMS {
            // Count is known. Any position can hold any of the map's items.
            return op.fixed(&vec![item; possible]);
        }
        if possible == 1 {
            return Type::union([op.fixed(&[]), op.fixed(&[item])]);
        }
    }
    op.open(&item, minimum)
}

fn conj_seq(previous: &[Type], items: &[Type]) -> Type {
    let prepended = items.iter().rev().chain(previous).cloned();
    if previous.len() + items.len() <= MAX_TUPLE_ITEMS {
        Type::SeqTuple(prepended.collect())
    } else {
        Type::Seq(Box::new(Type::union(prepended)))
    }
}

pub(in crate::checker) fn iteration_may_throw(ty: &Type) -> bool {
    match ty {
        Type::Seq(_) | Type::Unknown => true,
        Type::Union(parts) => parts.iter().any(iteration_may_throw),
        _ => false,
    }
}

pub(super) fn collection_emptiness(ty: &Type) -> Option<Type> {
    Some(match ty {
        Type::Nil => Type::Bool(true),
        Type::Never => Type::Never,
        Type::Unknown => Type::Boolean,
        Type::Parameter(parameter) => return collection_emptiness(parameter.upper.as_ref()?),
        Type::Tuple(items) | Type::SeqTuple(items) => Type::Bool(items.is_empty()),
        Type::Seq(_) | Type::MapEntry(_) => Type::Bool(false),
        Type::Vector(element) | Type::Set(element) if **element == Type::Never => Type::Bool(true),
        Type::Vector(_) | Type::Set(_) | Type::String => Type::Boolean,
        Type::Str(text) => Type::Bool(text.is_empty()),
        Type::Map(map) => {
            if map.fields.values().any(|field| field.required) {
                Type::Bool(false)
            } else if map.fields.is_empty() && matches!(map.extra, ExtraFields::Closed) {
                Type::Bool(true)
            } else {
                Type::Boolean
            }
        }
        Type::Union(parts) => Type::union(
            parts
                .iter()
                .map(collection_emptiness)
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => return None,
    })
}

pub(in crate::checker) fn narrow_emptiness(ty: &Type, empty: bool) -> Type {
    if let Type::Union(parts) = ty {
        return Type::union(parts.iter().map(|part| narrow_emptiness(part, empty)));
    }
    if collection_emptiness(ty) == Some(Type::Bool(!empty)) {
        return Type::Never;
    }
    match (ty, empty) {
        (Type::Vector(_), true) => Type::Tuple(Box::new([])),
        (Type::Set(_), true) => Type::Set(Box::new(Type::Never)),
        (Type::String, true) => Type::Str(String::new()),
        (Type::Map(_), true) => Type::Map(MapType {
            fields: BTreeMap::new(),
            extra: ExtraFields::Closed,
        }),
        _ => ty.clone(),
    }
}
