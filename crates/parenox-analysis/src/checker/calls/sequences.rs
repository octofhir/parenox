//! Vector and sequence traversal and accumulation.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::super::{Checker, LiteralContainer, Value, exact_set_member, literal_vector_value};
use super::literal_equality;
use crate::Diagnostic;
use crate::reader::Span;
use crate::types::{ExtraFields, IntegerType, MAX_TUPLE_ITEMS, MapType, Type};

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

    fn exact_set(self, members: &[Type]) -> Type {
        if members.len() < 2 {
            if matches!(self, Self::Rest) {
                Type::empty_list()
            } else {
                self.fixed(members)
            }
        } else {
            let member = Type::union(members.iter().cloned());
            self.open(&member, members.len())
        }
    }

    fn list_result(self, result: Type) -> Type {
        if matches!(self, Self::Head) {
            result
        } else {
            sequence_kind(result, true)
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
    if args.len() > MAX_TUPLE_ITEMS {
        return Value::new(Type::List(Box::new(Type::union(items))));
    }
    let mut result = Value::new(Type::ListTuple(items.collect()));
    let children = args
        .iter()
        .any(|(value, _)| value.literal_container.is_some())
        .then(|| {
            args.iter()
                .map(|(value, _)| value.literal_container.clone())
                .collect()
        });
    result.literal_container = Some(LiteralContainer::List(children));
    result
}

fn conj_exact_set(collection: &Value, items: &[(Value, Span)]) -> Option<Value> {
    let previous = collection.exact_set_members()?;
    if previous.len().saturating_add(items.len()) > MAX_TUPLE_ITEMS
        || items
            .iter()
            .any(|(value, _)| exact_set_member(&value.known).is_none())
    {
        return None;
    }
    let mut members = previous.to_vec();
    for (value, _) in items {
        // PersistentHashSet.cons keeps the original object for an equal member.
        if !members
            .iter()
            .any(|member| literal_equality(member, &value.known) == Some(true))
        {
            members.push(value.known.clone());
        }
    }
    let mut result = Value::literal_set(members);
    result.notes.clone_from(&collection.notes);
    Some(result)
}

fn disj_exact_set(collection: &Value, keys: &[(Value, Span)]) -> Option<Value> {
    let previous = collection.exact_set_members()?;
    if keys.len() > MAX_TUPLE_ITEMS
        || keys
            .iter()
            .any(|(value, _)| exact_set_member(&value.known).is_none())
    {
        return None;
    }
    let mut members = previous.to_vec();
    for (key, _) in keys {
        members.retain(|member| literal_equality(member, &key.known) != Some(true));
    }
    let mut result = Value::literal_set(members);
    result.notes.clone_from(&collection.notes);
    Some(result)
}

impl Checker<'_> {
    pub(super) fn cons(&mut self, args: &[(Value, Span)]) -> Value {
        let (item, _) = &args[0];
        let (tail, span) = &args[1];
        if tail.unproven_tuple_traversal() {
            return self.unchecked_tuple_traversal("clojure.core/cons", *span);
        }
        let result = self.cons_type(&item.known, &tail.known, *span);
        Value::with_notes(result, tail.notes.clone())
    }

    fn cons_type(&mut self, item: &Type, tail: &Type, span: Span) -> Type {
        match tail {
            Type::Nil => return Type::ListTuple(Box::new([item.clone()])),
            Type::Union(parts) => {
                return Type::union(parts.iter().map(|part| self.cons_type(item, part, span)));
            }
            _ => {}
        }
        let sequence = self.seq_type(tail, "clojure.core/cons", span);
        if sequence.contains_unknown() {
            self.may_throw();
        }
        // A non-nil tail produces a Cons, even when the tail is an empty list.
        sequence_kind(
            self.conj_type(&sequence, std::slice::from_ref(item), span),
            false,
        )
    }

    pub(super) fn conj(&mut self, args: &[(Value, Span)]) -> Value {
        let Some(((collection, span), items)) = args.split_first() else {
            return literal_vector_value(std::iter::empty(), 0, true);
        };
        if items.is_empty() {
            return collection.clone();
        }
        if let Some(result) = conj_exact_set(collection, items) {
            return result;
        }
        let added: Vec<_> = items.iter().map(|(value, _)| value.known.clone()).collect();
        let result = self.conj_type(&collection.known, &added, *span);
        let mut result = Value::with_notes(result, collection.notes.clone());
        if let (Type::Tuple(previous), Some(vector @ LiteralContainer::Vector(children))) =
            (&collection.known, &collection.literal_container)
        {
            if matches!(result.known, Type::Tuple(_)) {
                let nested = children.is_some()
                    || items
                        .iter()
                        .any(|(item, _)| item.literal_container.is_some());
                let proofs = nested.then(|| {
                    (0..previous.len())
                        .map(|index| vector.position_child(index).cloned())
                        .chain(items.iter().map(|(item, _)| item.literal_container.clone()))
                        .collect()
                });
                result.literal_container = Some(LiteralContainer::Vector(proofs));
            }
        } else {
            self.may_throw();
        }
        result
    }

    pub(super) fn set_from_collection(&mut self, (input, span): &(Value, Span)) -> Value {
        let items = match &input.known {
            Type::Set(_) if input.is_literal_set() => return input.clone(),
            Type::Nil => &[],
            Type::ListTuple(items)
                if matches!(input.literal_container, Some(LiteralContainer::List(_))) =>
            {
                items.as_ref()
            }
            Type::Tuple(items)
                if matches!(input.literal_container, Some(LiteralContainer::Vector(_))) =>
            {
                items.as_ref()
            }
            Type::Unknown | Type::Never => return input.clone(),
            _ => {
                return self.unsupported(
                    *span,
                    "set analysis currently requires an ordinary vector, a fixed ordinary list, an ordinary set, or Nil",
                );
            }
        };
        if items.len() > MAX_TUPLE_ITEMS
            || items.iter().any(|item| exact_set_member(item).is_none())
        {
            self.may_throw();
            let mut result = Value::literal_set(items.to_vec());
            result.notes.clone_from(&input.notes);
            return result;
        }
        let mut members = Vec::new();
        for item in items {
            if let Some(previous) = members
                .iter()
                .find(|previous| literal_equality(previous, item) == Some(true))
            {
                if previous != item {
                    // Transient construction can keep different objects for seq and lookup.
                    return self.unsupported(
                        *span,
                        "set analysis cannot represent different stored classes for equivalent members",
                    );
                }
            } else {
                members.push(item.clone());
            }
        }
        let mut result = Value::literal_set(members);
        result.notes.clone_from(&input.notes);
        result
    }

    pub(super) fn disj(&mut self, args: &[(Value, Span)]) -> Value {
        let [(collection, span), keys @ ..] = args else {
            unreachable!("disj needs a target");
        };
        if keys.is_empty() {
            return collection.clone();
        }
        if let Some(result) = disj_exact_set(collection, keys) {
            return result;
        }
        let start = self.diagnostics.len();
        let result = self.disj_type(&collection.known, *span);
        self.dedup_diagnostics(start);
        Value::with_notes(result, collection.notes.clone())
    }

    fn disj_type(&mut self, collection: &Type, span: Span) -> Type {
        match collection {
            Type::Nil | Type::Bool(false) => Type::Nil,
            Type::Never => Type::Never,
            Type::Set(_) | Type::Unknown => {
                self.may_throw();
                collection.clone()
            }
            Type::Union(parts) => Type::union(parts.iter().map(|part| self.disj_type(part, span))),
            Type::Bool(true) | Type::Integer(_) | Type::Key(_) | Type::Str(_) | Type::Char(_) => {
                self.class_cast();
                self.diagnostics.push(Diagnostic::type_mismatch(
                    span,
                    "clojure.core/disj",
                    "Set, Nil or false".into(),
                    collection.inferred().to_string(),
                ));
                Type::Never
            }
            _ => {
                self.unsupported(span, "disj analysis requires a known Set, Nil or false")
                    .known
            }
        }
    }

    pub(super) fn peek(&mut self, (collection, span): &(Value, Span)) -> Value {
        if collection.known == Type::Nil {
            return collection.clone();
        }
        if let (Type::Tuple(items), Some(vector @ LiteralContainer::Vector(_))) =
            (&collection.known, &collection.literal_container)
        {
            let known = items.last().cloned().unwrap_or(Type::Nil);
            let mut item = Value::with_notes(known, collection.notes.clone());
            item.literal_container = items
                .len()
                .checked_sub(1)
                .and_then(|index| vector.position_child(index))
                .cloned();
            return item;
        }
        self.unchecked_form(
            *span,
            "peek currently needs a proven ordinary vector or Nil",
        )
    }

    fn conj_type(&mut self, collection: &Type, items: &[Type], span: Span) -> Type {
        let joined = |element: &Type| {
            Box::new(Type::union(
                std::iter::once(element.clone()).chain(items.iter().cloned()),
            ))
        };
        match collection {
            Type::Unknown | Type::Never => collection.clone(),
            Type::Any => {
                self.may_throw();
                Type::Any
            }
            Type::Nil => sequence_kind(conj_seq(&[], items), true),
            Type::ListTuple(previous) => sequence_kind(conj_seq(previous, items), true),
            Type::SeqTuple(previous) => conj_seq(previous, items),
            Type::Seq(element) => Type::Seq(joined(element)),
            Type::List(element) => Type::List(joined(element)),
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
            Type::SpecCollection(_) => {
                self.unsupported(span, "conj analysis requires a known collection kind")
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
        if name == "clojure.core/not-empty" && value.unproven_tuple_traversal() {
            self.may_throw();
            let mut result = value.clone();
            result.narrow(result.known.clone().nilable());
            return result;
        }
        let empty = value
            .exact_set_members()
            .map(|members| Type::Bool(members.is_empty()))
            .or_else(|| collection_emptiness(&value.known));
        let Some(empty) = empty else {
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
        let mut result = value.clone();
        result.narrow(if empty == Type::Bool(false) {
            nonempty
        } else {
            nonempty.nilable()
        });
        result
    }

    pub(in crate::checker) fn sequence_call(
        &mut self,
        name: &str,
        value: &Value,
        span: Span,
    ) -> Value {
        let operation = match name {
            "clojure.core/seq" => ReadSequence::Start,
            "clojure.core/first" => ReadSequence::Head,
            "clojure.core/next" => ReadSequence::Tail,
            "clojure.core/rest" => ReadSequence::Rest,
            _ => unreachable!(),
        };
        if let Some(members) = value.exact_set_members() {
            let mut result = Value::with_notes(operation.exact_set(members), value.notes.clone());
            if matches!(operation, ReadSequence::Rest) && members.len() < 2 {
                result.literal_container = Some(LiteralContainer::List(None));
            }
            return result;
        }
        if matches!(operation, ReadSequence::Head) {
            if let (Type::Tuple(items), Some(LiteralContainer::Vector(_))) =
                (&value.known, &value.literal_container)
            {
                return value.position_value(operation.fixed(items), 0);
            }
            if self.partial_scope.is_some() && !matches!(value.known, Type::Nil | Type::Never) {
                // A structural vector contract can constrain count and nth
                // without constraining the sequence read by first.
                self.may_throw();
                return Value::new(Type::Unknown);
            }
        }
        if value.unproven_tuple_traversal() {
            return self.unchecked_tuple_traversal(name, span);
        }
        if iteration_may_throw(&value.known) {
            self.may_throw();
        }
        let result = self.read_sequence(operation, &value.known, name, span);
        if matches!(operation, ReadSequence::Head) {
            if matches!(value.known, Type::Map(_)) {
                return value.traversal_element(result);
            }
            return value.position_value(result, 0);
        }
        let mut result = Value::with_notes(result, value.notes.clone());
        let children = value
            .literal_container
            .as_ref()
            .and_then(LiteralContainer::child_positions)
            .map(|items| {
                if matches!(operation, ReadSequence::Start) {
                    items.clone()
                } else {
                    Arc::from(items.get(1..).unwrap_or_default())
                }
            });
        let proof = if matches!(value.literal_container, Some(LiteralContainer::List(_))) {
            Some(LiteralContainer::List(children))
        } else {
            children.map(LiteralContainer::Positions)
        };
        result.literal_container = proof.filter(|proof| proof.matches_type(&result.known));
        result
    }

    pub(in crate::checker) fn seq_type(&mut self, ty: &Type, name: &str, span: Span) -> Type {
        self.read_sequence(ReadSequence::Start, ty, name, span)
    }

    pub(in crate::checker) fn seq_value_type(
        &mut self,
        value: &Value,
        name: &str,
        span: Span,
    ) -> Type {
        if value.unproven_tuple_traversal() {
            return self.unchecked_tuple_traversal(name, span).known;
        }
        self.seq_type(&value.known, name, span)
    }

    pub(in crate::checker) fn unchecked_tuple_traversal(
        &mut self,
        name: &str,
        span: Span,
    ) -> Value {
        self.unchecked_form(
            span,
            &format!("{name} needs a proven ordinary vector to use tuple elements for traversal"),
        )
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
            Type::Any | Type::Unknown => {
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
            Type::Any => {
                self.may_throw();
                Type::Unknown
            }
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.read_sequence(op, upper, name, span)
            }
            Type::Nil if matches!(op, ReadSequence::Rest) => Type::empty_list(),
            Type::Nil => op.fixed(&[]),
            Type::Tuple(items) if items.len() < 2 && matches!(op, ReadSequence::Rest) => {
                Type::empty_list()
            }
            Type::Tuple(items) | Type::SeqTuple(items) => op.fixed(items),
            Type::ListTuple(items) => op.list_result(op.fixed(items)),
            Type::MapEntry(items) => op.fixed(items.as_ref()),
            Type::Vector(element) | Type::Set(element) if **element == Type::Never => op.fixed(&[]),
            Type::Vector(element) | Type::Set(element) => op.open(element, 0),
            Type::Seq(element) => op.open(element, 1),
            Type::List(element) => op.list_result(op.open(element, 1)),
            Type::SpecCollection(element) => op.open(element, 0),
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

fn sequence_kind(ty: Type, persistent_list: bool) -> Type {
    match (ty, persistent_list) {
        (Type::SeqTuple(items), true) => Type::ListTuple(items),
        (Type::Seq(item), true) => Type::List(item),
        (Type::ListTuple(items), false) => Type::SeqTuple(items),
        (Type::List(item), false) => Type::Seq(item),
        (Type::Union(parts), _) => Type::union(
            parts
                .into_iter()
                .map(|part| sequence_kind(part, persistent_list)),
        ),
        (ty, _) => ty,
    }
}

pub(in crate::checker) fn iteration_may_throw(ty: &Type) -> bool {
    match ty {
        Type::Seq(_) | Type::Any | Type::Unknown | Type::SpecCollection(_) => true,
        Type::Union(parts) => parts.iter().any(iteration_may_throw),
        _ => false,
    }
}

pub(super) fn collection_emptiness(ty: &Type) -> Option<Type> {
    Some(match ty {
        Type::Nil => Type::Bool(true),
        Type::Never => Type::Never,
        Type::Any | Type::Unknown | Type::SpecCollection(_) => Type::Boolean,
        Type::Parameter(parameter) => return collection_emptiness(parameter.upper.as_ref()?),
        Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => {
            Type::Bool(items.is_empty())
        }
        Type::Seq(_) | Type::List(_) | Type::MapEntry(_) => Type::Bool(false),
        Type::Vector(element) | Type::Set(element) if **element == Type::Never => Type::Bool(true),
        Type::Vector(_) | Type::Set(_) | Type::String => Type::Boolean,
        Type::Str(text) => Type::Bool(text.is_empty()),
        Type::Map(map) => map.emptiness().map_or(Type::Boolean, Type::Bool),
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

pub(in crate::checker) fn narrow_seq_emptiness(ty: &Type, empty: bool) -> Type {
    match ty {
        Type::Tuple(_) | Type::Parameter(_) => ty.clone(),
        Type::Union(parts) => {
            Type::union(parts.iter().map(|part| narrow_seq_emptiness(part, empty)))
        }
        _ => narrow_emptiness(ty, empty),
    }
}

fn counted_type(ty: &Type) -> Option<Type> {
    const JVM_COUNT: Type = Type::Integral(IntegerType::FIXED_INT32);
    let size = match ty {
        Type::Nil => 0,
        Type::Str(text) => text.encode_utf16().count(),
        Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => items.len(),
        Type::MapEntry(_) => 2,
        Type::Vector(element) | Type::Set(element) if **element == Type::Never => 0,
        Type::String
        | Type::Map(_)
        | Type::Vector(_)
        | Type::Seq(_)
        | Type::List(_)
        | Type::Set(_)
        | Type::Unknown => return Some(JVM_COUNT),
        Type::Never => return Some(Type::Never),
        Type::Parameter(parameter) => return counted_type(parameter.upper.as_ref()?),
        Type::Union(parts) => {
            let counts = parts.iter().map(counted_type).collect::<Option<Vec<_>>>()?;
            return Some(Type::union(counts));
        }
        _ => return None,
    };
    Some(i32::try_from(size).map_or(JVM_COUNT, Type::Integer))
}

impl Checker<'_> {
    pub(super) fn count(&mut self, (value, span): &(Value, Span)) -> Value {
        if let Some(members) = value.exact_set_members()
            && let Ok(size) = i32::try_from(members.len())
        {
            return Value::new(Type::Integer(size));
        }
        if let Some(size) = counted_type(&value.known) {
            return Value::new(size);
        }
        self.error(
            "PNX2322",
            *span,
            format!(
                "count expects a collection, string or Nil, got {}",
                value.ty
            ),
        );
        Value::new(Type::Integral(IntegerType::FIXED_INT32))
    }
}
