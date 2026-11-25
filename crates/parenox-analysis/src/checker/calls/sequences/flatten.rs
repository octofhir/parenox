use super::{bounded_sequence_value, iteration_may_throw, positional_sequence};
use crate::checker::{Checker, LiteralContainer, Value, flow::has_closed_scalar_class};
use crate::reader::Span;
use crate::types::{LengthBounds, MAX_TUPLE_ITEMS, Type};

const MAX_FLATTEN_DEPTH: usize = 32;
const MAX_FLATTEN_NODES: usize = 256;

struct Flattened {
    element: Type,
    lengths: LengthBounds,
    positions: Option<Vec<Value>>,
}

impl Flattened {
    fn empty() -> Self {
        Self {
            element: Type::Never,
            lengths: LengthBounds::exact(0),
            positions: Some(Vec::new()),
        }
    }

    fn leaf(ty: &Type, proof: Option<&LiteralContainer>) -> Self {
        let mut value = Value::new(ty.clone());
        value.literal_container = proof.cloned();
        Self {
            element: ty.clone(),
            lengths: LengthBounds::exact(1),
            positions: Some(vec![value]),
        }
    }

    fn append(self, other: Self) -> Self {
        let positions = self
            .positions
            .zip(other.positions)
            .and_then(|(mut left, right)| {
                (left.len() + right.len() <= MAX_TUPLE_ITEMS).then(|| {
                    left.extend(right);
                    left
                })
            });
        Self {
            element: Type::union([self.element, other.element]),
            lengths: LengthBounds {
                min: self.lengths.min.saturating_add(other.lengths.min),
                max: self
                    .lengths
                    .max
                    .zip(other.lengths.max)
                    .and_then(|(a, b)| a.checked_add(b)),
            },
            positions,
        }
    }

    fn cover(self, other: Self) -> Self {
        Self {
            element: Type::union([self.element, other.element]),
            lengths: LengthBounds {
                min: self.lengths.min.min(other.lengths.min),
                max: self
                    .lengths
                    .max
                    .zip(other.lengths.max)
                    .map(|(a, b)| a.max(b)),
            },
            positions: None,
        }
    }

    fn repeat(self, source: LengthBounds) -> Self {
        if self.lengths.max == Some(0) || source.max == Some(0) {
            return Self::empty();
        }
        if source == LengthBounds::exact(1) {
            return self;
        }
        Self {
            element: self.element,
            lengths: LengthBounds {
                min: self.lengths.min.saturating_mul(source.min),
                max: self
                    .lengths
                    .max
                    .zip(source.max)
                    .and_then(|(a, b)| a.checked_mul(b)),
            },
            positions: None,
        }
    }
}

impl Checker<'_> {
    pub(in crate::checker::calls) fn flatten(&mut self, (source, span): &(Value, Span)) -> Value {
        if matches!(source.known, Type::Never | Type::Unknown) {
            return source.clone();
        }
        // Dropping the root of tree-seq can force the source's initial seq.
        if iteration_may_throw(&source.known) || matches!(source.known, Type::SeqTuple(_)) {
            self.may_throw();
        }
        let mut budget = MAX_FLATTEN_NODES;
        let result = flatten_shape(
            &source.known,
            source.literal_container.as_ref(),
            true,
            0,
            &mut budget,
        );
        match result {
            Ok(flat) => {
                if let Some(positions) = flat.positions {
                    let mut result = positional_sequence(positions);
                    result.notes.clone_from(&source.notes);
                    result
                } else {
                    bounded_sequence_value(flat.element, flat.lengths, source.notes.clone())
                }
            }
            Err(message) => self.unchecked_form(*span, message),
        }
    }
}

fn flatten_shape(
    ty: &Type,
    proof: Option<&LiteralContainer>,
    root: bool,
    depth: usize,
    budget: &mut usize,
) -> Result<Flattened, &'static str> {
    if depth > MAX_FLATTEN_DEPTH || *budget == 0 {
        return Err("Type analysis for flatten exceeded its nested shape limit");
    }
    *budget -= 1;
    let (kind, lengths) = match ty {
        Type::BoundedCollection(collection, lengths) => (collection.as_ref(), Some(*lengths)),
        _ => (ty, None),
    };
    match kind {
        Type::Never => Ok(Flattened::empty()),
        Type::Union(parts) => {
            let mut branches = parts
                .iter()
                .map(|part| flatten_shape(part, proof, root, depth + 1, budget));
            let first = branches
                .next()
                .transpose()?
                .unwrap_or_else(Flattened::empty);
            branches.try_fold(first, |result, branch| Ok(result.cover(branch?)))
        }
        Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
            flatten_shape(upper, proof, root, depth + 1, budget)
        }
        Type::Tuple(_)
            if !matches!(
                proof,
                Some(LiteralContainer::Vector(_) | LiteralContainer::NonemptyVector)
            ) =>
        {
            Err(
                "Type analysis for flatten needs a proven ordinary vector to traverse tuple positions",
            )
        }
        Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => {
            flatten_positions(items, proof, depth, budget)
        }
        Type::MapEntry(items) => flatten_positions(items.as_ref(), proof, depth, budget),
        Type::Vector(item) | Type::Seq(item) | Type::List(item) => {
            let lengths = lengths.unwrap_or_else(|| kind.length_bounds());
            if lengths.max == Some(0) {
                return Ok(Flattened::empty());
            }
            Ok(flatten_shape(item, None, false, depth + 1, budget)?.repeat(lengths))
        }
        kind if has_closed_scalar_class(kind)
            || matches!(kind, Type::JvmArray(_) | Type::JvmIterator)
            || matches!(
                proof,
                Some(
                    LiteralContainer::Map(_)
                        | LiteralContainer::Set(_)
                        | LiteralContainer::GroupBuckets
                )
            ) =>
        {
            Ok(if root {
                Flattened::empty()
            } else {
                Flattened::leaf(ty, proof)
            })
        }
        _ => Err("Type analysis for flatten cannot determine whether this value is sequential"),
    }
}

fn flatten_positions(
    items: &[Type],
    proof: Option<&LiteralContainer>,
    depth: usize,
    budget: &mut usize,
) -> Result<Flattened, &'static str> {
    items
        .iter()
        .enumerate()
        .try_fold(Flattened::empty(), |result, (index, item)| {
            let proof = proof.and_then(|proof| proof.position_child(index));
            Ok(result.append(flatten_shape(item, proof, false, depth + 1, budget)?))
        })
}
