//! Vector and sequence traversal and accumulation.
mod flatten;

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use super::super::{
    Checker, Env, LiteralContainer, Value, exact_set_member, flow::has_closed_scalar_class,
    literal_vector_value, nil_part,
};
use super::{arrays::boxed_array_element, literal_equality, vectors::element_type};
use crate::Diagnostic;
use crate::reader::{Form, Span};
use crate::types::{
    Compatibility, ExtraFields, IntegerClass, IntegerType, LengthBounds, MAX_TUPLE_ITEMS, MapType,
    Type,
};

#[derive(Clone, Copy)]
enum ReadSequence {
    Start,
    Head,
    Tail,
    Rest,
}

fn reads_array(name: &str) -> bool {
    matches!(
        name,
        "clojure.core/seq"
            | "clojure.core/first"
            | "clojure.core/next"
            | "clojure.core/rest"
            | "clojure.core/distinct"
            | "clojure.core/frequencies"
    )
}

fn fixed_range(start: i64, end: i64, step: i64) -> Type {
    if start == end {
        return Type::empty_list();
    }
    if step == 0 {
        return infinite_sequence(Type::exact_long(start, IntegerClass::IntOrLong));
    }
    // Inputs are i32 values widened to i64, so even the full distance fits.
    let distance = if step > 0 { end - start } else { start - end };
    if distance <= 0 {
        return Type::empty_list();
    }
    let length = usize::try_from(1 + (distance - 1) / step.abs())
        .expect("the distance between i32 endpoints fits usize");
    if length > MAX_TUPLE_ITEMS {
        return Type::BoundedCollection(
            Box::new(Type::Seq(Box::new(Type::Integral(IntegerType::FIXED)))),
            LengthBounds::exact(length),
        );
    }
    Type::SeqTuple(
        (0..length)
            .map(|index| Type::exact_long(start + step * index as i64, IntegerClass::IntOrLong))
            .collect(),
    )
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

    fn bounded(self, element: Type, bounds: LengthBounds, preserve_bounds: bool) -> Type {
        if bounds.max == Some(bounds.min) && bounds.min <= MAX_TUPLE_ITEMS {
            return self.fixed(&vec![element; bounds.min]);
        }
        if !preserve_bounds || matches!(self, Self::Head) {
            return self.open(&element, bounds.min);
        }
        let remaining = match self {
            Self::Tail | Self::Rest => bounds.after_consuming(1),
            _ => bounds,
        };
        if remaining.max == Some(0) {
            return self.fixed(&[]);
        }
        let result = nonempty_sequence(element, remaining);
        match self {
            Self::Rest if remaining.min == 0 => Type::union([Type::empty_seq(), result]),
            _ if remaining.min == 0 => result.nilable(),
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

pub(super) fn nonempty_sequence(element: Type, lengths: LengthBounds) -> Type {
    let nonempty = LengthBounds {
        min: lengths.min.max(1),
        max: lengths.max,
    };
    let sequence = Type::Seq(Box::new(element));
    if nonempty == LengthBounds::NONEMPTY {
        sequence
    } else {
        Type::BoundedCollection(Box::new(sequence), nonempty)
    }
}

fn infinite_sequence(element: Type) -> Type {
    // Cover every lower bound expressible in a collection contract.
    nonempty_sequence(
        element,
        LengthBounds {
            min: i32::MAX as usize,
            max: None,
        },
    )
}

#[derive(Clone, Copy)]
pub(super) enum SequenceSlice {
    Take(Option<usize>),
    Drop(Option<usize>),
    DropLast(Option<usize>),
    TakeLast(Option<usize>),
    ButLast,
}

impl SequenceSlice {
    pub(super) fn new(name: &str, count: &Type) -> Self {
        match name {
            "clojure.core/take" => Self::Take(nonnegative_count(count)),
            "clojure.core/drop" => Self::Drop(exact_drop_count(count)),
            "clojure.core/drop-last" => Self::DropLast(exact_drop_count(count)),
            "clojure.core/take-last" => Self::TakeLast(exact_drop_count(count)),
            _ => unreachable!("the slice operation was resolved before slicing"),
        }
    }

    pub(super) fn positions(self, length: usize) -> Option<Range<usize>> {
        match self {
            Self::Take(Some(limit)) => Some(0..limit.min(length)),
            Self::Drop(Some(count)) => Some(count.min(length)..length),
            Self::DropLast(Some(count)) => Some(0..length.saturating_sub(count)),
            Self::TakeLast(Some(count)) => Some(length.saturating_sub(count)..length),
            Self::ButLast => Some(0..length.saturating_sub(1)),
            Self::Take(None) | Self::Drop(None) | Self::DropLast(None) | Self::TakeLast(None) => {
                None
            }
        }
    }

    pub(super) fn lengths(self, source: LengthBounds) -> LengthBounds {
        match self {
            Self::Take(Some(limit)) | Self::TakeLast(Some(limit)) => LengthBounds {
                min: limit.min(source.min),
                max: Some(source.max.map_or(limit, |max| max.min(limit))),
            },
            Self::Drop(Some(count)) | Self::DropLast(Some(count)) => source.after_consuming(count),
            Self::ButLast => source.after_consuming(1),
            Self::Take(None) | Self::Drop(None) | Self::DropLast(None) | Self::TakeLast(None) => {
                LengthBounds {
                    min: 0,
                    max: source.max,
                }
            }
        }
    }

    fn empty(self) -> Type {
        match self {
            Self::TakeLast(_) | Self::ButLast => Type::Nil,
            _ => Type::empty_seq(),
        }
    }
}

fn nonnegative_count(count: &Type) -> Option<usize> {
    count
        .exact_long_value()
        .map(|value| usize::try_from(value.max(0)).unwrap_or(usize::MAX))
}

fn exact_drop_count(count: &Type) -> Option<usize> {
    // IDrop can convert non-fixed counts through double before skipping.
    let value = count.exact_long_value()?;
    (value <= (1_i64 << 53) || count.subtype_of(&Type::Integral(IntegerType::FIXED)))
        .then(|| usize::try_from(value.max(0)).unwrap_or(usize::MAX))
}

fn slice_sequence_type(sequence: Type, slice: SequenceSlice) -> Type {
    match sequence {
        Type::Nil => slice.empty(),
        Type::Never | Type::Unknown => Type::Unknown,
        Type::Union(parts) => Type::union(
            parts
                .into_iter()
                .map(|part| slice_sequence_type(part, slice)),
        ),
        Type::SeqTuple(items) | Type::ListTuple(items)
            if let Some(positions) = slice.positions(items.len()) =>
        {
            if positions.is_empty() {
                slice.empty()
            } else {
                Type::SeqTuple(items[positions].into())
            }
        }
        shape => {
            let lengths = slice.lengths(shape.length_bounds());
            if lengths.max == Some(0) {
                return slice.empty();
            }
            let element = element_type(shape)
                .unwrap_or_else(|_| unreachable!("a sequence has an element type"));
            let sequence = nonempty_sequence(element, lengths);
            if lengths.min == 0 {
                Type::union([slice.empty(), sequence])
            } else {
                sequence
            }
        }
    }
}

fn slice_sequence_value(sequence: &Value, slice: SequenceSlice) -> Value {
    if matches!(slice, SequenceSlice::Take(Some(0))) {
        return Value::new(slice.empty());
    }
    let mut result = Value::with_notes(
        slice_sequence_type(sequence.known.clone(), slice),
        sequence.notes.clone(),
    );
    if let Type::SeqTuple(items) = &result.known
        && let Some(positions) = sequence
            .literal_container
            .as_ref()
            .and_then(LiteralContainer::child_positions)
        && let Some(range) = slice.positions(positions.len())
        && range.len() == items.len()
    {
        result.literal_container = Some(LiteralContainer::Positions(Arc::from(&positions[range])));
    }
    result
}

fn nonempty_seq(items: &[Type]) -> Type {
    if items.is_empty() {
        Type::Nil
    } else {
        Type::SeqTuple(items.into())
    }
}

pub(in crate::checker) fn sequence_lengths(source: &Type, sequence: &Type) -> LengthBounds {
    let source = source.length_bounds();
    let sequence = sequence.length_bounds();
    LengthBounds {
        min: source.min.max(sequence.min),
        max: source.max.into_iter().chain(sequence.max).min(),
    }
}

fn spaced_lengths(source: LengthBounds, spacing: usize) -> LengthBounds {
    LengthBounds {
        min: source.min.div_ceil(spacing),
        max: source.max.map(|size| size.div_ceil(spacing)),
    }
}

fn full_partitions(length: usize, size: usize, step: usize) -> usize {
    length.checked_sub(size).map_or(0, |rest| 1 + rest / step)
}

fn partial_partition(length: usize, size: usize, step: usize) -> Option<usize> {
    full_partitions(length, size, step)
        .checked_mul(step)
        .filter(|&start| start < length)
}

fn may_use_padding(lengths: LengthBounds, size: usize, step: usize) -> bool {
    if size <= 1 {
        return false;
    }
    if size > step {
        return true;
    }
    // Without overlap, a final partial group exists exactly at remainders 1..size.
    let length = lengths.min.max(1);
    let remainder = length % step;
    if (1..size).contains(&remainder) {
        return true;
    }
    let distance = if remainder == 0 {
        1
    } else {
        step - remainder + 1
    };
    length
        .checked_add(distance)
        .is_some_and(|next| lengths.includes(next))
}

fn sequence_item_equality(left: &Type, right: &Type) -> Option<bool> {
    literal_equality(left, right).or_else(|| {
        ((left.is_vector() && has_closed_scalar_class(right))
            || (right.is_vector() && has_closed_scalar_class(left)))
        .then_some(false)
    })
}

fn dedupe_skips_initial(item: &Type) -> Option<bool> {
    // Clojure 1.13's transducer initializes the previous value to ::none.
    let initial = Type::Key(":clojure.core/none".into());
    sequence_item_equality(&initial, item)
        .or_else(|| (item.known_intersection(&initial) == Some(Type::Never)).then_some(false))
}

fn bounded_sequence_value(element: Type, lengths: LengthBounds, notes: Vec<String>) -> Value {
    let result = if lengths.max == Some(0) {
        Type::empty_seq()
    } else if lengths.min == 0 {
        Type::union([Type::empty_seq(), nonempty_sequence(element, lengths)])
    } else {
        nonempty_sequence(element, lengths)
    };
    Value::with_notes(result, notes)
}

fn modeled_distinct_source(ty: &Type) -> bool {
    match ty {
        Type::Nil | Type::Never | Type::Unknown | Type::JvmArray(_) => true,
        Type::Union(parts) => parts.iter().all(modeled_distinct_source),
        Type::Parameter(parameter) => parameter
            .upper
            .as_ref()
            .is_some_and(modeled_distinct_source),
        ty => ty.nth_item(0).is_some() || definitely_not_seqable(ty),
    }
}

fn repeated_values(item: &Value, length: usize) -> Value {
    if length <= MAX_TUPLE_ITEMS {
        Value::positional_sequence(vec![item.clone(); length])
    } else {
        Value::with_notes(
            nonempty_sequence(item.known.clone(), LengthBounds::exact(length)),
            item.notes.clone(),
        )
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
    pub(super) fn range(&mut self, args: &[(Value, Span)]) -> Value {
        for (arg, span) in args {
            self.expect(arg, &Type::Number, *span, "clojure.core/range");
        }
        if args.is_empty() {
            return Value::new(infinite_sequence(Type::Integral(IntegerType::ANY)));
        }
        let fixed = Type::Integral(IntegerType::FIXED);
        let integer = Type::Integral(IntegerType::ANY);
        if args.iter().all(|(arg, _)| arg.known.subtype_of(&fixed))
            && let Some(values) = args
                .iter()
                .map(|(arg, _)| arg.known.exact_integer_value().map(i64::from))
                .collect::<Option<Vec<_>>>()
        {
            let (start, end, step) = match values.as_slice() {
                [end] => (0, *end, 1),
                [start, end] => (*start, *end, 1),
                [start, end, step] => (*start, *end, *step),
                _ => unreachable!("range arities are validated before inference"),
            };
            return Value::new(fixed_range(start, end, step));
        }
        self.may_throw();
        let element = if args.iter().all(|(arg, _)| arg.known.subtype_of(&fixed)) {
            fixed
        } else if args.iter().all(|(arg, _)| arg.known.subtype_of(&integer)) {
            integer
        } else {
            Type::Number
        };
        Value::new(Type::union([
            Type::empty_list(),
            Type::Seq(Box::new(element)),
        ]))
    }

    pub(super) fn slice_sequence(
        &mut self,
        name: &str,
        args: &[(Value, Span)],
        span: Span,
    ) -> Value {
        let default;
        let ((count, count_span), (source, source_span)) = match args {
            [count, source] => (count, source),
            [source] if name == "clojure.core/drop-last" => {
                default = (Value::new(Type::Integer(1)), span);
                (&default, source)
            }
            _ => {
                return self.unsupported(
                    span,
                    format!("Type analysis for an unapplied {name} transducer is unavailable"),
                );
            }
        };
        self.expect(count, &Type::Number, *count_span, name);
        let split_at = name == "clojure.core/split-at";
        let operation = if split_at { "clojure.core/drop" } else { name };
        let slice = SequenceSlice::new(operation, &count.known);
        if matches!(slice, SequenceSlice::Take(Some(0))) {
            return Value::new(Type::empty_seq());
        }
        if matches!(
            slice,
            SequenceSlice::Drop(None) | SequenceSlice::DropLast(None)
        ) && !has_closed_scalar_class(&source.known)
            && !matches!(source.literal_container, Some(LiteralContainer::List(_)))
        {
            // IDrop collections can check the argument during construction.
            if matches!(source.literal_container, Some(LiteralContainer::Vector(_)))
                && count.known.compatibility(&Type::Number) == Compatibility::Mismatch
            {
                if nil_part(&count.known) != Type::Never {
                    self.null_pointer();
                }
                if !count.known.subtype_of(&Type::Nil) {
                    self.class_cast();
                }
                return Value::new(Type::Never);
            }
            self.may_throw();
        }

        // Deferred traversal can still be checked for types.
        let enclosing = self.throws.take();
        let sequence = self.sequence_call(name, source, *source_span);
        self.throws = enclosing;
        let result = slice_sequence_value(&sequence, slice);
        if split_at && let SequenceSlice::Drop(count) = slice {
            let prefix = slice_sequence_value(&sequence, SequenceSlice::Take(count));
            return literal_vector_value([prefix, result].into_iter(), 2, true);
        }
        result
    }

    pub(super) fn concat(&mut self, args: &[(Value, Span)]) -> Value {
        let sequences = self.read_lazy_sources("clojure.core/concat", args);
        if sequences
            .iter()
            .any(|sequence| matches!(sequence.known, Type::Unknown | Type::Never))
        {
            return Value::new(Type::Unknown);
        }
        let lengths = args
            .iter()
            .zip(&sequences)
            .map(|((source, _), sequence)| sequence_lengths(&source.known, &sequence.known))
            .fold(LengthBounds::exact(0), |sum, input| LengthBounds {
                min: sum.min.saturating_add(input.min),
                max: sum.max.zip(input.max).and_then(|(a, b)| a.checked_add(b)),
            });
        if lengths.max == Some(0) {
            return Value::new(Type::empty_seq());
        }
        if lengths.max.is_some_and(|max| max <= MAX_TUPLE_ITEMS) {
            let mut values = Vec::new();
            let fixed = sequences.iter().all(|sequence| {
                let items = match &sequence.known {
                    Type::Nil => return true,
                    Type::SeqTuple(items) | Type::ListTuple(items) => items,
                    _ => return false,
                };
                values.extend(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| sequence.position_value(item.clone(), index)),
                );
                true
            });
            if fixed {
                return Value::positional_sequence(values);
            }
        }
        let element = Type::union(sequences.into_iter().map(|sequence| {
            element_type(sequence.known)
                .unwrap_or_else(|_| unreachable!("a sequence has an element type"))
        }));
        let result = nonempty_sequence(element, lengths);
        Value::new(if lengths.min == 0 {
            Type::union([Type::empty_seq(), result])
        } else {
            result
        })
    }

    pub(super) fn interpose(&mut self, args: &[(Value, Span)], span: Span) -> Value {
        let [(separator, _), (source, source_span)] = args else {
            return self.unsupported(
                span,
                "Type analysis for an interpose transducer is unavailable",
            );
        };
        let enclosing = self.throws.take();
        let sequence = self.sequence_call("clojure.core/interpose", source, *source_span);
        self.throws = enclosing;
        if matches!(sequence.known, Type::Unknown | Type::Never) {
            return Value::new(Type::Unknown);
        }
        let source_lengths = sequence_lengths(&source.known, &sequence.known);
        if source_lengths.max == Some(0) {
            return Value::new(Type::empty_seq());
        }
        let with_separators = |n: usize| n.checked_add(n.saturating_sub(1));
        let lengths = LengthBounds {
            min: with_separators(source_lengths.min).unwrap_or(usize::MAX),
            max: source_lengths.max.and_then(with_separators),
        };
        if let Some(max) = lengths.max.filter(|&max| max <= MAX_TUPLE_ITEMS)
            && let Type::SeqTuple(items) | Type::ListTuple(items) = &sequence.known
        {
            let mut values = Vec::with_capacity(max);
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    values.push(separator.clone());
                }
                values.push(sequence.position_value(item.clone(), index));
            }
            return Value::positional_sequence(values);
        }
        let mut element = element_type(sequence.known)
            .unwrap_or_else(|_| unreachable!("a sequence has an element type"));
        if source_lengths.max != Some(1) {
            element = Type::union([element, separator.known.clone()]);
        }
        let result = nonempty_sequence(element, lengths);
        Value::new(if lengths.min == 0 {
            Type::union([Type::empty_seq(), result])
        } else {
            result
        })
    }

    pub(super) fn interleave(&mut self, args: &[(Value, Span)]) -> Value {
        let sequences = self.read_lazy_sources("clojure.core/interleave", args);
        if sequences
            .iter()
            .any(|sequence| matches!(sequence.known, Type::Unknown | Type::Never))
        {
            return Value::new(Type::Unknown);
        }
        let shortest = args
            .iter()
            .zip(&sequences)
            .map(|((source, _), sequence)| sequence_lengths(&source.known, &sequence.known))
            .reduce(|left, right| LengthBounds {
                min: left.min.min(right.min),
                max: left.max.into_iter().chain(right.max).min(),
            })
            .unwrap_or(LengthBounds::exact(0));
        if shortest.max == Some(0) {
            return Value::new(Type::empty_seq());
        }
        let lengths = LengthBounds {
            min: shortest.min.saturating_mul(args.len()),
            max: shortest.max.and_then(|max| max.checked_mul(args.len())),
        };
        if let Some(max) = lengths.max.filter(|&max| max <= MAX_TUPLE_ITEMS)
            && let Some(fixed) = sequences
                .iter()
                .map(|sequence| match &sequence.known {
                    Type::SeqTuple(items) | Type::ListTuple(items) => Some(items.as_ref()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
        {
            let mut values = Vec::with_capacity(max);
            for index in 0..shortest.min {
                for (sequence, items) in sequences.iter().zip(&fixed) {
                    values.push(sequence.position_value(items[index].clone(), index));
                }
            }
            return Value::positional_sequence(values);
        }
        let element = Type::union(sequences.into_iter().map(|sequence| {
            element_type(sequence.known)
                .unwrap_or_else(|_| unreachable!("a sequence has an element type"))
        }));
        let result = nonempty_sequence(element, lengths);
        Value::new(if lengths.min == 0 {
            Type::union([Type::empty_seq(), result])
        } else {
            result
        })
    }

    pub(super) fn repeat(&mut self, args: &[(Value, Span)]) -> Value {
        let [(count, span), (item, _)] = args else {
            return Value::new(infinite_sequence(args[0].0.known.clone()));
        };
        self.expect(count, &Type::Number, *span, "clojure.core/repeat");
        if count.known.compatibility(&Type::Number) == Compatibility::Mismatch {
            if nil_part(&count.known) != Type::Never {
                self.null_pointer();
            }
            if !count.known.subtype_of(&Type::Nil) {
                self.class_cast();
            }
            return Value::new(Type::Never);
        }
        if count
            .known
            .subtype_of(&Type::Integral(IntegerType::OUTSIDE_INT64))
        {
            self.illegal_argument();
            return Value::new(Type::Never);
        }
        let Some(count) = count.known.exact_long_value() else {
            self.may_throw();
            return Value::new(Type::union([
                Type::empty_list(),
                Type::Seq(Box::new(item.known.clone())),
            ]));
        };
        if count <= 0 {
            return Value::new(Type::empty_list());
        }
        let Ok(length) = usize::try_from(count) else {
            return Value::new(Type::Seq(Box::new(item.known.clone())));
        };
        repeated_values(item, length)
    }

    pub(super) fn repeatedly(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        const NAME: &str = "clojure.core/repeatedly";
        if self.standard_core_name(&forms[0], env).as_deref() != Some(NAME) {
            return self.unchecked_form(
                forms[0].span,
                "Standard clojure.core/repeatedly is unconfirmed",
            );
        }
        let (count, callback) = match &forms[1..] {
            [callback] => (None, callback),
            [count, callback] => (Some((self.infer(count, env), count.span)), callback),
            other => {
                return self.reject_runtime_arity(
                    other,
                    span,
                    env,
                    "repeatedly expects a callback and an optional leading count",
                );
            }
        };
        let ready = count
            .as_ref()
            .is_none_or(|(count, _)| !count.stops_evaluation());
        self.inspect_path(ready, |checker| {
            checker.with_callback_choice(callback, env, |checker, selected, scope| {
                let callable = checker.prepare_callable(selected, scope);
                if !ready || !callable.returns() {
                    checker.call_prepared(&callable, Vec::new(), span, scope, false);
                    return Value::new(Type::Never);
                }
                let limit = if let Some((count, count_span)) = &count {
                    let start = checker.diagnostics.len();
                    checker.expect(count, &Type::Number, *count_span, NAME);
                    checker.note_builtin_contract(start, NAME);
                    if count.known.compatibility(&Type::Number) == Compatibility::Mismatch {
                        checker.call_prepared(&callable, Vec::new(), span, scope, false);
                        return Value::new(Type::Seq(Box::new(Type::Never)));
                    }
                    nonnegative_count(&count.known)
                } else {
                    None
                };
                if limit == Some(0) {
                    checker.call_prepared(&callable, Vec::new(), span, scope, false);
                    return Value::new(Type::empty_seq());
                }
                let enclosing = checker.throws.take();
                let item = checker.call_prepared(&callable, Vec::new(), span, scope, true);
                checker.throws = enclosing;
                match limit {
                    Some(length) => repeated_values(&item, length),
                    None if count.is_none() => {
                        Value::with_notes(infinite_sequence(item.known), item.notes)
                    }
                    None => Value::with_notes(
                        Type::union([Type::empty_seq(), Type::Seq(Box::new(item.known))]),
                        item.notes,
                    ),
                }
            })
        })
    }

    pub(super) fn cycle(&mut self, (source, span): &(Value, Span)) -> Value {
        // Cycle/create receives the result of seq during construction.
        let sequence = self.sequence_call("clojure.core/cycle", source, *span);
        if matches!(sequence.known, Type::Never | Type::Unknown) {
            if sequence.known == Type::Unknown {
                self.may_throw();
            }
            return sequence;
        }
        let lengths = sequence_lengths(&source.known, &sequence.known);
        if lengths.max == Some(0) {
            return Value::new(Type::empty_list());
        }
        let element = element_type(sequence.known)
            .unwrap_or_else(|_| unreachable!("a sequence has an element type"));
        let repeated = infinite_sequence(element);
        let result = if lengths.min == 0 {
            Type::union([Type::empty_list(), repeated])
        } else {
            repeated
        };
        Value::with_notes(result, sequence.notes)
    }

    pub(super) fn take_nth(&mut self, args: &[(Value, Span)], span: Span) -> Value {
        const NAME: &str = "clojure.core/take-nth";
        let [(step, step_span), (source, source_span)] = args else {
            return self.unsupported(
                span,
                "Type analysis for a take-nth transducer is unavailable",
            );
        };
        let enclosing = self.throws.take();
        let sequence = self.sequence_call(NAME, source, *source_span);
        self.throws = enclosing;
        if matches!(sequence.known, Type::Unknown | Type::Never) {
            return Value::new(Type::Unknown);
        }
        let source_lengths = sequence_lengths(&source.known, &sequence.known);
        if source_lengths.max == Some(0) {
            return Value::new(Type::empty_seq());
        }
        self.expect(step, &Type::Number, *step_span, NAME);
        let spacing = exact_drop_count(&step.known);
        let fixed = match &sequence.known {
            Type::SeqTuple(items) | Type::ListTuple(items) => Some(items.as_ref()),
            _ => None,
        };
        if let (Some(spacing @ 1..), Some(items)) = (spacing, fixed) {
            let values = items
                .iter()
                .enumerate()
                .step_by(spacing)
                .map(|(index, item)| sequence.position_value(item.clone(), index))
                .collect();
            return Value::positional_sequence(values);
        }
        let first = if spacing == Some(0) {
            fixed.and_then(|items| items.first()).cloned()
        } else {
            None
        };
        let element = first.unwrap_or_else(|| {
            element_type(sequence.known)
                .unwrap_or_else(|_| unreachable!("a sequence has an element type"))
        });
        let result = match spacing {
            Some(0) => infinite_sequence(element),
            Some(spacing) => nonempty_sequence(element, spaced_lengths(source_lengths, spacing)),
            None => Type::Seq(Box::new(element)),
        };
        Value::with_notes(
            if source_lengths.min == 0 {
                Type::union([Type::empty_seq(), result])
            } else {
                result
            },
            sequence.notes,
        )
    }

    pub(super) fn dedupe(&mut self, args: &[(Value, Span)], span: Span) -> Value {
        const NAME: &str = "clojure.core/dedupe";
        let [(source, source_span)] = args else {
            return self.unsupported(span, "Type analysis for a dedupe transducer is unavailable");
        };
        // sequence with a transducer probes for an initial output at construction.
        let sequence = self.eager_sequence(NAME, source, *source_span);
        if matches!(sequence.known, Type::Unknown | Type::Never) {
            return sequence;
        }
        let source_lengths = sequence_lengths(&source.known, &sequence.known);
        if source_lengths.max == Some(0) {
            return Value::with_notes(Type::empty_seq(), sequence.notes);
        }
        if let Type::SeqTuple(items) | Type::ListTuple(items) = &sequence.known {
            let mut values = Vec::new();
            let mut minimum = 0;
            for (index, item) in items.iter().enumerate() {
                let skip = if index == 0 {
                    dedupe_skips_initial(item)
                } else {
                    sequence_item_equality(&items[index - 1], item)
                };
                if skip != Some(true) {
                    minimum += usize::from(skip == Some(false));
                    values.push(sequence.position_value(item.clone(), index));
                }
            }
            if minimum == values.len() {
                let mut result = Value::positional_sequence(values);
                result.notes = sequence.notes;
                return result;
            }
            self.may_throw();
            let lengths = LengthBounds {
                min: minimum,
                max: Some(values.len()),
            };
            let element = Type::union(values.into_iter().map(|item| item.known));
            return bounded_sequence_value(element, lengths, sequence.notes);
        }
        let element = element_type(sequence.known.clone())
            .unwrap_or_else(|_| unreachable!("a sequence has an element type"));
        let skips_initial = dedupe_skips_initial(&element);
        let uniform = literal_equality(&element, &element) == Some(true);
        let lengths = LengthBounds {
            min: usize::from(source_lengths.min > 0 && skips_initial == Some(false)),
            max: if uniform {
                Some(usize::from(skips_initial != Some(true)))
            } else {
                source_lengths.max
            },
        };
        bounded_sequence_value(element, lengths, sequence.notes)
    }

    pub(super) fn distinct(&mut self, args: &[(Value, Span)], span: Span) -> Value {
        const NAME: &str = "clojure.core/distinct";
        let [(source, source_span)] = args else {
            return self.unsupported(
                span,
                "Type analysis for a distinct transducer is unavailable",
            );
        };
        // The first lazy step destructures the original collection using nth.
        // Later steps destructure the seq returned by rest.
        if source.is_literal_map() || source.is_literal_set() {
            self.diagnostics.push(Diagnostic::type_mismatch(
                *source_span,
                NAME,
                "an input supporting nth".into(),
                source.ty.to_string(),
            ));
            return Value::new(Type::Unknown);
        }
        if !modeled_distinct_source(&source.known) {
            return self.unchecked_form(
                *source_span,
                "Type analysis for distinct cannot verify nth on this source type",
            );
        }
        let sequence = self.read_lazy_source(NAME, source, *source_span);
        if matches!(sequence.known, Type::Never | Type::Unknown) {
            return Value::with_notes(Type::Unknown, sequence.notes);
        }
        let source_lengths = sequence_lengths(&source.known, &sequence.known);
        if source_lengths.max == Some(0) {
            return Value::with_notes(Type::empty_seq(), sequence.notes);
        }
        if let Type::SeqTuple(items) | Type::ListTuple(items) = &sequence.known
            && items.len() <= MAX_TUPLE_ITEMS
        {
            let mut values = Vec::new();
            let mut minimum = 0;
            'positions: for (index, item) in items.iter().enumerate() {
                let mut first_occurrence = true;
                for previous in &items[..index] {
                    match sequence_item_equality(previous, item) {
                        Some(true) => continue 'positions,
                        Some(false) => {}
                        None => first_occurrence = false,
                    }
                }
                minimum += usize::from(first_occurrence);
                values.push(sequence.position_value(item.clone(), index));
            }
            if minimum == values.len() {
                let mut result = Value::positional_sequence(values);
                result.notes = sequence.notes;
                return result;
            }
            let lengths = LengthBounds {
                min: minimum,
                max: Some(values.len()),
            };
            let element = Type::union(values.into_iter().map(|item| item.known));
            return bounded_sequence_value(element, lengths, sequence.notes);
        }
        let element = element_type(sequence.known.clone())
            .unwrap_or_else(|_| unreachable!("a sequence has an element type"));
        let lengths = LengthBounds {
            min: usize::from(source_lengths.min > 0),
            max: if literal_equality(&element, &element) == Some(true) {
                Some(1)
            } else {
                source_lengths.max
            },
        };
        bounded_sequence_value(element, lengths, sequence.notes)
    }

    pub(super) fn partition_all(&mut self, args: &[(Value, Span)], span: Span) -> Value {
        const NAME: &str = "clojure.core/partition-all";
        let ((size, size_span), step, (source, source_span)) = match args {
            [size, source] => (size, None, source),
            [size, step, source] => (size, Some(step), source),
            _ => {
                return self.unsupported(
                    span,
                    "Type analysis for a partition-all transducer is unavailable",
                );
            }
        };
        let mut sequence = self.read_lazy_source(NAME, source, *source_span);
        if matches!(sequence.known, Type::Unknown | Type::Never) {
            return Value::new(Type::Unknown);
        }
        let lengths = sequence_lengths(&source.known, &sequence.known);
        if lengths.max == Some(0) {
            return Value::new(Type::empty_seq());
        }
        self.expect(size, &Type::Number, *size_span, "partition-all size");
        let step = step.map_or(size, |(step, span)| {
            self.expect(step, &Type::Number, *span, "partition-all step");
            step
        });
        let size = nonnegative_count(&size.known);
        let spacing = exact_drop_count(&step.known);

        let result = if spacing == Some(0) {
            sequence.narrow(sequence.known.without_nil());
            infinite_sequence(slice_sequence_value(&sequence, SequenceSlice::Take(size)).known)
        } else {
            if let (Some(size), Some(spacing)) = (size, spacing)
                && let Type::SeqTuple(items) | Type::ListTuple(items) = &sequence.known
                && items
                    .len()
                    .div_ceil(spacing)
                    .saturating_mul(size.min(items.len()))
                    <= MAX_TUPLE_ITEMS
            {
                let groups = (0..items.len()).step_by(spacing).map(|first| {
                    let end = first.saturating_add(size).min(items.len());
                    Value::positional_sequence(
                        (first..end)
                            .map(|index| sequence.position_value(items[index].clone(), index))
                            .collect(),
                    )
                });
                return Value::positional_sequence(groups.collect());
            }
            let last_size = spacing.map_or(1, |spacing| {
                if lengths.max == Some(lengths.min) {
                    1 + (lengths.min - 1) % spacing
                } else {
                    1
                }
            });
            let group_lengths = LengthBounds {
                min: size.map_or(0, |size| size.min(last_size)),
                max: size.into_iter().chain(lengths.max).min(),
            };
            let group = if group_lengths.max == Some(0) {
                Type::empty_seq()
            } else {
                let item = element_type(sequence.known)
                    .unwrap_or_else(|_| unreachable!("a sequence has an element type"));
                let group = nonempty_sequence(item, group_lengths);
                if group_lengths.min == 0 {
                    Type::union([Type::empty_seq(), group])
                } else {
                    group
                }
            };
            let outer = spacing.map_or(LengthBounds::UNBOUNDED, |spacing| {
                spaced_lengths(lengths, spacing)
            });
            nonempty_sequence(group, outer)
        };
        Value::with_notes(
            if lengths.min == 0 {
                Type::union([Type::empty_seq(), result])
            } else {
                result
            },
            sequence.notes,
        )
    }

    pub(super) fn partition(&mut self, args: &[(Value, Span)], span: Span) -> Value {
        const NAME: &str = "clojure.core/partition";
        let (width, stride, padding, (source, source_span)) = match args {
            [width, source] => (width, None, None, source),
            [width, step, source] => (width, Some(step), None, source),
            [width, step, pad, source] => (width, Some(step), Some(pad), source),
            _ => unreachable!("partition arity was checked"),
        };
        let source_sequence = self.read_lazy_source(NAME, source, *source_span);
        if matches!(source_sequence.known, Type::Never | Type::Unknown) {
            return Value::new(Type::Unknown);
        }
        let lengths = sequence_lengths(&source.known, &source_sequence.known);
        if lengths.max == Some(0) {
            return Value::new(Type::empty_seq());
        }
        self.expect(&width.0, &Type::Number, width.1, "partition size");
        if let Some((step, step_span)) = stride {
            self.expect(step, &Type::Number, *step_span, "partition step");
        }
        let stride = &stride.unwrap_or(width).0;
        if [&width.0, stride]
            .into_iter()
            .any(|value| value.known.compatibility(&Type::Number) == Compatibility::Mismatch)
        {
            return Value::new(Type::Unknown);
        }
        let (Some(size @ 1..), Some(step @ 1..)) = (
            nonnegative_count(&width.0.known),
            exact_drop_count(&stride.known),
        ) else {
            return self.unchecked_form(
                span,
                "partition needs a known positive integer size and step",
            );
        };
        if size > i32::MAX as usize {
            return self.unchecked_form(
                width.1,
                "partition size exceeds the JVM collection count range",
            );
        }
        let padding = padding.filter(|_| may_use_padding(lengths, size, step));
        let pad_sequence = padding.map_or_else(
            || Value::new(Type::Nil),
            |(pad, span)| self.read_lazy_source(NAME, pad, *span),
        );
        if matches!(pad_sequence.known, Type::Never | Type::Unknown) {
            return Value::new(Type::Unknown);
        }
        let groups = |length| {
            full_partitions(length, size, step)
                + usize::from(padding.is_some() && partial_partition(length, size, step).is_some())
        };
        let outer_lengths = LengthBounds {
            min: groups(lengths.min),
            max: lengths.max.map(groups),
        };
        if outer_lengths.max == Some(0) {
            return Value::new(Type::empty_seq());
        }
        if let Type::SeqTuple(items) | Type::ListTuple(items) = &source_sequence.known
            && let Some(pad_items) = match &pad_sequence.known {
                Type::Nil => Some(&[][..]),
                Type::SeqTuple(items) | Type::ListTuple(items) => Some(items.as_ref()),
                _ => None,
            }
            && groups(items.len()).saturating_mul(size) <= MAX_TUPLE_ITEMS
        {
            let result = (0..groups(items.len())).map(|group| {
                let start = group * step;
                let end = start.saturating_add(size).min(items.len());
                let source = (start..end)
                    .map(|index| source_sequence.position_value(items[index].clone(), index));
                let padding = pad_items
                    .iter()
                    .take(size - (end - start))
                    .enumerate()
                    .map(|(index, item)| pad_sequence.position_value(item.clone(), index));
                Value::positional_sequence(source.chain(padding).collect())
            });
            return Value::positional_sequence(result.collect());
        }
        let shape_item = |sequence: Value| {
            element_type(sequence.known)
                .unwrap_or_else(|_| unreachable!("a sequence has an element type"))
        };
        let pad_lengths = pad_sequence.known.length_bounds();
        let element = Type::union([shape_item(source_sequence), shape_item(pad_sequence)]);
        let group_lengths = LengthBounds {
            min: if padding.is_some() {
                size.min(1_usize.saturating_add(pad_lengths.min))
            } else {
                size
            },
            max: Some(size),
        };
        let result = nonempty_sequence(nonempty_sequence(element, group_lengths), outer_lengths);
        Value::new(if outer_lengths.min == 0 {
            Type::union([Type::empty_seq(), result])
        } else {
            result
        })
    }

    pub(super) fn take_last(&mut self, args: &[(Value, Span)]) -> Value {
        const NAME: &str = "clojure.core/take-last";
        let [(count, count_span), (source, source_span)] = args else {
            unreachable!("take-last arity was checked");
        };
        let sequence = self.sequence_call(NAME, source, *source_span);
        if sequence.stops_evaluation() {
            return sequence;
        }
        self.expect(count, &Type::Number, *count_span, NAME);
        if count.known.compatibility(&Type::Number) == Compatibility::Mismatch {
            if nil_part(&count.known) != Type::Never {
                self.null_pointer();
            }
            if !count.known.subtype_of(&Type::Nil) {
                self.class_cast();
            }
            return Value::new(Type::Never);
        }
        let slice = SequenceSlice::new(NAME, &count.known);
        if matches!(slice, SequenceSlice::TakeLast(None))
            || !matches!(
                source.literal_container,
                Some(
                    LiteralContainer::Vector(_)
                        | LiteralContainer::List(_)
                        | LiteralContainer::Set(_)
                )
            ) && !has_closed_scalar_class(&source.known)
        {
            self.may_throw();
        }
        if matches!(slice, SequenceSlice::TakeLast(Some(0))) {
            return Value::new(Type::Nil);
        }
        slice_sequence_value(&sequence, slice)
    }

    pub(super) fn reverse(&mut self, (source, span): &(Value, Span)) -> Value {
        let sequence = self.eager_sequence("clojure.core/reverse", source, *span);
        if sequence.stops_evaluation() || sequence.known == Type::Unknown {
            return sequence;
        }
        if let Type::SeqTuple(items) | Type::ListTuple(items) = &sequence.known {
            let args: Vec<_> = items
                .iter()
                .enumerate()
                .rev()
                .map(|(index, item)| (sequence.position_value(item.clone(), index), *span))
                .collect();
            let mut result = list_value(&args);
            result.notes = sequence.notes;
            return result;
        }
        let lengths = sequence_lengths(&source.known, &sequence.known);
        if lengths.max == Some(0) {
            return list_value(&[]);
        }
        let element = element_type(sequence.known)
            .unwrap_or_else(|_| unreachable!("a sequence has an element type"));
        let result = sequence_kind(nonempty_sequence(element, lengths), true);
        Value::with_notes(
            if lengths.min == 0 {
                Type::union([Type::empty_list(), result])
            } else {
                result
            },
            sequence.notes,
        )
    }

    pub(super) fn butlast(&mut self, (source, span): &(Value, Span)) -> Value {
        let sequence = self.eager_sequence("clojure.core/butlast", source, *span);
        if sequence.stops_evaluation() {
            return sequence;
        }
        slice_sequence_value(&sequence, SequenceSlice::ButLast)
    }

    pub(in crate::checker) fn eager_sequence(
        &mut self,
        name: &str,
        source: &Value,
        span: Span,
    ) -> Value {
        let sequence = self.sequence_call(name, source, span);
        // Traversing the entire source can force deferred sequence steps.
        if !sequence.stops_evaluation()
            && !matches!(
                source.literal_container,
                Some(
                    LiteralContainer::Vector(_)
                        | LiteralContainer::List(_)
                        | LiteralContainer::Set(_)
                )
            )
            && !has_closed_scalar_class(&source.known)
        {
            self.may_throw();
        }
        if matches!(&sequence.known, Type::SeqTuple(items) | Type::ListTuple(items)
            if items.contains(&Type::Never))
        {
            return Value::new(Type::Never);
        }
        sequence
    }

    fn read_lazy_source(&mut self, name: &str, source: &Value, span: Span) -> Value {
        let enclosing = self.throws.take();
        let sequence = self.sequence_call(name, source, span);
        self.throws = enclosing;
        sequence
    }

    fn read_lazy_sources(&mut self, name: &str, args: &[(Value, Span)]) -> Vec<Value> {
        let enclosing = self.throws.take();
        let mut sequences = Vec::with_capacity(args.len());
        for (index, (source, span)) in args.iter().enumerate() {
            let sequence = self.sequence_call(name, source, *span);
            // The variadic interleave checks the first two seqs through
            // unchunked cons cells, so an empty one skips later traversals.
            let finished = name == "clojure.core/interleave"
                && args.len() > 2
                && index < 2
                && sequence.known == Type::Nil;
            sequences.push(sequence);
            if finished {
                break;
            }
        }
        self.throws = enclosing;
        sequences
    }

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
            Type::BoundedCollection(collection, _)
                if collection.is_vector()
                    || matches!(collection.as_ref(), Type::Seq(_) | Type::List(_)) =>
            {
                self.conj_type(collection, items, span)
            }
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
            .or_else(|| {
                value
                    .known_array_length()
                    .map(|length| Type::Bool(length == 0))
            })
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
        if empty == Type::Bool(true) {
            return Value::with_notes(Type::Nil, value.notes.clone());
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
            "clojure.core/seq"
            | "clojure.core/take"
            | "clojure.core/drop"
            | "clojure.core/split-at"
            | "clojure.core/concat"
            | "clojure.core/interpose"
            | "clojure.core/interleave"
            | "clojure.core/cycle"
            | "clojure.core/take-nth"
            | "clojure.core/take-last"
            | "clojure.core/drop-last"
            | "clojure.core/reverse"
            | "clojure.core/butlast"
            | "clojure.core/dedupe"
            | "clojure.core/distinct"
            | "clojure.core/frequencies"
            | "clojure.core/partition-all"
            | "clojure.core/partition" => ReadSequence::Start,
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
        let result = match (&value.known, value.known_array_length()) {
            (Type::JvmArray(class), Some(length)) if reads_array(name) => operation.bounded(
                boxed_array_element(*class),
                LengthBounds::exact(length),
                true,
            ),
            _ => self.read_sequence(operation, &value.known, name, span, true),
        };
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
        // Traversal consumers dispatch on nil, fixed positions, or a homogeneous sequence.
        self.read_sequence(ReadSequence::Start, ty, name, span, false)
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
        let mut result = Value::with_notes(result, value.notes.clone());
        let proof = LiteralContainer::GroupBucketSequence;
        if name == "clojure.core/vals"
            && matches!(
                value.literal_container,
                Some(LiteralContainer::GroupBuckets)
            )
            && proof.matches_type(&result.known)
        {
            result.literal_container = Some(proof);
        }
        result
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

    fn read_sequence(
        &mut self,
        op: ReadSequence,
        ty: &Type,
        name: &str,
        span: Span,
        preserve_bounds: bool,
    ) -> Type {
        match ty {
            Type::Never | Type::Unknown => ty.clone(),
            Type::Any => {
                self.may_throw();
                Type::Unknown
            }
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.read_sequence(op, upper, name, span, preserve_bounds)
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
            Type::BoundedCollection(collection, bounds) => {
                let element = element_type(collection.as_ref().clone())
                    .unwrap_or_else(|_| unreachable!("collection has an element type"));
                let result = op.bounded(element, *bounds, preserve_bounds);
                if matches!(collection.as_ref(), Type::List(_)) {
                    op.list_result(result)
                } else {
                    result
                }
            }
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
            Type::JvmArray(class) if reads_array(name) => op.open(&boxed_array_element(*class), 0),
            Type::Map(map) => self.read_map_sequence(op, map, name, span),
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.read_sequence(op, part, name, span, preserve_bounds)),
            ),
            actual
                if matches!(
                    name,
                    "clojure.core/concat"
                        | "clojure.core/interpose"
                        | "clojure.core/interleave"
                        | "clojure.core/cycle"
                        | "clojure.core/take-nth"
                        | "clojure.core/take-last"
                        | "clojure.core/drop-last"
                        | "clojure.core/reverse"
                        | "clojure.core/butlast"
                        | "clojure.core/dedupe"
                        | "clojure.core/frequencies"
                        | "clojure.core/partition-all"
                        | "clojure.core/partition"
                ) && !definitely_not_seqable(actual) =>
            {
                self.unchecked_form(
                    span,
                    &format!("{name} traversal for this source type is unchecked"),
                )
                .known
            }
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
        (Type::BoundedCollection(inner, bounds), kind) => {
            Type::BoundedCollection(Box::new(sequence_kind(*inner, kind)), bounds)
        }
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
        Type::BoundedCollection(collection, _) => iteration_may_throw(collection),
        Type::Union(parts) => parts.iter().any(iteration_may_throw),
        _ => false,
    }
}

pub(super) fn definitely_not_seqable(ty: &Type) -> bool {
    match ty {
        Type::Union(parts) => parts.iter().all(definitely_not_seqable),
        Type::Bool(_)
        | Type::Boolean
        | Type::Char(_)
        | Type::Character
        | Type::Key(_)
        | Type::Keyword
        | Type::SpecKeyword
        | Type::SimpleSymbol
        | Type::QualifiedSymbol
        | Type::Symbol
        | Type::Sym(_)
        | Type::Integer(_)
        | Type::Integral(IntegerType {
            class: IntegerClass::Fixed | IntegerClass::IntOrLong,
            ..
        }) => true,
        // Custom Number and BigInteger subclasses may implement Seqable.
        _ => false,
    }
}

pub(super) fn collection_emptiness(ty: &Type) -> Option<Type> {
    Some(match ty {
        Type::Nil => Type::Bool(true),
        Type::Never => Type::Never,
        Type::Any | Type::Unknown | Type::SpecCollection(_) => Type::Boolean,
        Type::BoundedCollection(_, bounds) if bounds.max == Some(0) => Type::Bool(true),
        Type::BoundedCollection(_, bounds) if bounds.min > 0 => Type::Bool(false),
        Type::BoundedCollection(..) => Type::Boolean,
        Type::Parameter(parameter) => return collection_emptiness(parameter.upper.as_ref()?),
        Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => {
            Type::Bool(items.is_empty())
        }
        Type::Seq(_) | Type::List(_) | Type::MapEntry(_) => Type::Bool(false),
        Type::Vector(element) | Type::Set(element) if **element == Type::Never => Type::Bool(true),
        Type::Vector(_) | Type::Set(_) | Type::String | Type::JvmArray(_) => Type::Boolean,
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
        Type::BoundedCollection(_, bounds) if bounds.max == Some(bounds.min) => bounds.min,
        Type::Vector(element) | Type::Set(element) if **element == Type::Never => 0,
        Type::String
        | Type::Map(_)
        | Type::Vector(_)
        | Type::Seq(_)
        | Type::List(_)
        | Type::Set(_)
        | Type::SpecCollection(_)
        | Type::BoundedCollection(..)
        | Type::JvmArray(_)
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
        if let Some(size) = value
            .known_array_length()
            .and_then(|size| i32::try_from(size).ok())
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
