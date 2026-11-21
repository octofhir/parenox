//! Conform known map fields and collection elements using resolved child specs.
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{MapWrap, SpecOperation, map_wrap};
use crate::checker::{
    Checker, Env, LiteralContainer, MAX_TUPLE_ITEMS, Value,
    calls::literal_equality,
    has_evaluated_metadata,
    maps::map_predicate,
    model::{MapFieldConversion, SpecConversion, StructuredConversion},
    nil_part,
};
use crate::reader::{Form, Kind};
use crate::types::{
    ExtraFields, Field, IntegerClass, IntegerType, MapSlot, MapType, SPEC_INVALID, Type,
};

const MAP_GAP: &str = "The map spec requires a closed map with known keyword fields";
const LITERAL_MAP_GAP: &str =
    "An s/keys extern requires a literal map for exact checking at each nested map";
const LITERAL_VECTOR_GAP: &str =
    "An s/coll-of extern requires a bounded literal vector for exact checking";
const REGISTRY_GAP: &str = "Cannot check an undeclared qualified key against the spec registry";
const REGISTRY_CONVERSION_GAP: &str =
    "Cannot apply a supported conversion to an undeclared qualified key's registered spec";
const REGISTRY_LOAD_GAP: &str =
    "Cannot prove the registered spec for an undeclared qualified key is loaded";
const COLLECTION_GAP: &str = "s/conform cannot reconstruct this collection of maps or strings; use a known ordered collection or a small hash set literal";
const PROJECTION_GAP: &str = "s/conform cannot project this collection; use a known ordered or empty collection or a small hash set literal";
const STRING_KEYED_MAP_GAP: &str =
    "s/conform requires a closed map with known keys for this map spec";
const LITERAL_STRING_KEYED_MAP_GAP: &str =
    "This s/coll-of map extern requires a literal map with known keys";

pub(super) fn conformed_value(input: &Value, output: Type, conversion: &SpecConversion) -> Value {
    let literal_container = conformed_origin(
        &input.known,
        input.literal_container.as_ref(),
        &output,
        conversion,
    );
    Value {
        literal_container,
        ..Value::new(output)
    }
}

fn conformed_origin(
    input: &Type,
    origin: Option<&LiteralContainer>,
    output: &Type,
    conversion: &SpecConversion,
) -> Option<LiteralContainer> {
    if let (Type::MapEntry(items), SpecConversion::TaggedOr(branches)) = (output, conversion)
        && let Type::Key(tag) = &items[0]
    {
        let (_, selected) = branches.iter().find(|(branch, _)| branch == tag)?;
        let result = conformed_origin(input, origin, &items[1], selected)?;
        return Some(LiteralContainer::Positions(Arc::from([None, Some(result)])));
    }
    if matches!(output, Type::Tuple(_))
        && (matches!(input, Type::MapEntry(_))
            || matches!(origin, Some(LiteralContainer::Vector(_))))
    {
        // A modeled collection conversion preserves an ordinary vector or
        // rebuilds a changed map entry as an ordinary vector.
        return Some(LiteralContainer::Vector(None));
    }
    let origin = origin?;
    match conversion {
        SpecConversion::Any
        | SpecConversion::AnyCollection
        | SpecConversion::Identity
        | SpecConversion::IdentityDomain(_) => origin.matches_type(output).then(|| origin.clone()),
        SpecConversion::Structured(shape) => match (shape.as_ref(), input, output, origin) {
            (StructuredConversion::Collection { .. }, Type::MapEntry(_), Type::MapEntry(_), _) => {
                Some(origin.clone())
            }
            (
                StructuredConversion::Map(fields),
                Type::Map(before),
                Type::Map(after),
                LiteralContainer::Map(Some(origins)),
            ) => {
                let children: BTreeMap<_, _> = origins
                    .iter()
                    .filter_map(|(key, proof)| {
                        let output = &after.fields.get(key)?.ty;
                        let child = if let Some(conversion) = &fields.get(key)?.conversion {
                            conformed_origin(
                                &before.fields.get(key)?.ty,
                                Some(proof),
                                output,
                                conversion,
                            )?
                        } else {
                            proof.matches_type(output).then(|| proof.clone())?
                        };
                        Some((key.clone(), child))
                    })
                    .collect();
                (!children.is_empty()).then(|| LiteralContainer::Map(Some(Arc::new(children))))
            }
            _ => None,
        },
        _ => None,
    }
}
const ENTRY_GAP: &str =
    "s/conform cannot prove whether this map entry preserves the identity of both elements";

type MapConversion<'a> = (&'a Type, &'a Field, Cow<'a, SpecConversion>);

impl Checker<'_> {
    pub(super) fn conform_choice<'branch>(
        &mut self,
        input: &Type,
        branches: impl Iterator<Item = (Option<&'branch str>, &'branch SpecConversion)>,
        operation: SpecOperation,
    ) -> Result<Type, &'static str> {
        let mut outcomes = Vec::new();
        for (tag, branch) in branches {
            let outcome = match self.conform_child(input, branch) {
                Err(MAP_GAP | COLLECTION_GAP) if matches!(operation, SpecOperation::Valid) => {
                    self.may_throw();
                    return Ok(Type::Boolean);
                }
                result => result?,
            };
            if outcome.value != Type::Never {
                let value = match (operation, tag) {
                    (SpecOperation::Conform, Some(tag)) => {
                        Type::MapEntry(Box::new([Type::Key(tag.into()), outcome.value]))
                    }
                    _ => outcome.value,
                };
                outcomes.push(operation.success(value));
            }
            // Only the invalid marker advances to the next branch. An
            // exception or a successful result stops this choice.
            if !outcome.invalid {
                return Ok(Type::union(outcomes));
            }
        }
        outcomes.push(operation.invalid());
        Ok(Type::union(outcomes))
    }

    pub(super) fn conform_set_literal(
        &mut self,
        spec: &Form,
        input: &Form,
        operation: SpecOperation,
        env: &Env,
    ) -> Option<Value> {
        let Kind::Set(items) = &input.without_metadata().kind else {
            return None;
        };
        if items.is_empty() || items.len() > MAX_TUPLE_ITEMS {
            return None;
        }
        let key = self.model.validation_key(spec.keyword()?)?;
        if !self.spec_load_is_known(&key) {
            return None;
        }
        let (_, Some(conversion)) = self.read_spec(&key) else {
            return None;
        };
        if conversion.portable_collection().is_some() && self.is_imported_structure(&key) {
            return None;
        }
        let (element, projected) = match &conversion {
            SpecConversion::ProjectedCollection(element) if !has_evaluated_metadata(input) => {
                (element.as_ref(), true)
            }
            _ if matches!(operation, SpecOperation::Conform) => {
                (hash_collection_element(&conversion)?, false)
            }
            _ => return None,
        };
        // The spec argument is a literal keyword. Evaluate the members once,
        // retaining each member's type before ordinary set inference joins them.
        let inputs = self.infer_set_members(items, env);
        if inputs.contains(&Type::Never) {
            return Some(Value::new(Type::Never));
        }
        // Evaluating a set literal checks duplicates before calling spec.
        // Distinct source forms can evaluate to equal values.
        if inputs.iter().enumerate().any(|(index, current)| {
            inputs[..index]
                .iter()
                .any(|previous| !distinct_set_members(previous, current))
        }) {
            self.illegal_argument();
        }
        let singleton = inputs.len() == 1;
        let result = self
            .conform_hash_members(inputs, element)
            .and_then(|mut outcome| {
                if outcome.value != Type::Never && matches!(operation, SpecOperation::Conform) {
                    outcome.value = if projected {
                        let payload = outcome.value.entry_item(1).ok_or(PROJECTION_GAP)?;
                        if singleton {
                            Type::SeqTuple(vec![payload].into_boxed_slice())
                        } else {
                            // Rebuilding the hash set can reorder or deduplicate the entries.
                            Type::Seq(Box::new(payload))
                        }
                    } else {
                        Type::Set(Box::new(outcome.value))
                    };
                }
                Ok(outcome.result_for(operation))
            });
        Some(match result {
            Ok(result) => Value::new(result),
            Err(MAP_GAP | COLLECTION_GAP)
                if projected && matches!(operation, SpecOperation::Valid) =>
            {
                self.may_throw();
                Value::new(Type::Boolean)
            }
            Err(reason) => self.unchecked_form(spec.span, reason),
        })
    }

    fn conform_hash_members(
        &mut self,
        inputs: Vec<Type>,
        element: &SpecConversion,
    ) -> Result<Outcome, &'static str> {
        let mut converted = Vec::with_capacity(inputs.len());
        let mut invalid = false;
        let mut can_succeed = true;
        // Hash iteration order is unknown. Even after finding a definitely
        // invalid member, an overflowing member may have been visited first.
        for input in inputs {
            let outcome = self.conform_child(&input, element)?;
            invalid |= outcome.invalid;
            can_succeed &= outcome.value != Type::Never;
            converted.push(outcome.value);
        }
        let success = if can_succeed {
            Type::union(converted)
        } else {
            Type::Never
        };
        Ok(Outcome {
            value: success,
            invalid,
        })
    }

    pub(super) fn conform_structure(
        &mut self,
        input: &Type,
        conversion: &StructuredConversion,
    ) -> Result<Type, &'static str> {
        match conversion {
            StructuredConversion::Map(fields) => match input {
                Type::Map(map) => self.conform_map(map, fields, SpecOperation::Conform, None),
                Type::Any
                | Type::Unknown
                | Type::SpecValue
                | Type::SpecTruthy
                | Type::SpecCollection(_) => Err(MAP_GAP),
                _ => Ok(SpecOperation::Conform.invalid()),
            },
            StructuredConversion::Collection { element, .. } => {
                self.conform_collection(input, element)
            }
        }
    }

    fn conform_collection(
        &mut self,
        input: &Type,
        element: &SpecConversion,
    ) -> Result<Type, &'static str> {
        match input {
            Type::Tuple(items) => self.conform_ordered(items, element, Type::Tuple),
            // With no :kind or :into, spec rebuilds both lists and seqs
            // as persistent lists in their original encounter order.
            Type::ListTuple(items) | Type::SeqTuple(items) => {
                self.conform_ordered(items, element, Type::ListTuple)
            }
            Type::Set(item) | Type::Vector(item) if **item == Type::Never => Ok(input.clone()),
            Type::MapEntry(items) => self.conform_entry(items, element),
            Type::Map(map) => match map.emptiness() {
                Some(true) => Ok(input.clone()),
                // A nonempty map yields entries. The modeled element domains
                // contain maps and strings.
                Some(false) => Ok(SpecOperation::Conform.invalid()),
                None => Err(COLLECTION_GAP),
            },
            Type::Unknown
            | Type::Any
            | Type::SpecValue
            | Type::SpecTruthy
            | Type::SpecCollection(_)
            | Type::Vector(_)
            | Type::Set(_)
            | Type::List(_)
            | Type::Seq(_) => Err(COLLECTION_GAP),
            _ => Ok(SpecOperation::Conform.invalid()),
        }
    }

    pub(super) fn conform_proven(
        &mut self,
        input: &Type,
        conversion: &SpecConversion,
        operation: SpecOperation,
        proof: Option<&LiteralContainer>,
    ) -> Result<Type, &'static str> {
        match conversion {
            SpecConversion::Structured(structure) => match structure.as_ref() {
                StructuredConversion::Map(fields) => {
                    self.conform_proven_map(input, fields, operation, proof)
                }
                StructuredConversion::Collection { element, .. } => {
                    self.conform_proven_collection(input, element, operation, proof)
                }
            },
            SpecConversion::StringKeyedMap { values, .. } => match proof {
                Some(proof @ LiteralContainer::Map(_)) if matches!(input, Type::Map(_)) => {
                    self.conform_string_keyed_map(input, values, operation, Some(proof))
                }
                Some(LiteralContainer::Vector(_)) => Ok(operation.invalid()),
                _ if map_predicate(input) == Type::Bool(false) => Ok(operation.invalid()),
                _ => Err(LITERAL_STRING_KEYED_MAP_GAP),
            },
            _ => self.convert_spec(input, conversion, operation),
        }
    }

    fn conform_proven_collection(
        &mut self,
        input: &Type,
        element: &SpecConversion,
        operation: SpecOperation,
        proof: Option<&LiteralContainer>,
    ) -> Result<Type, &'static str> {
        if let (Type::Tuple(items), Some(vector @ LiteralContainer::Vector(_))) = (input, proof) {
            let outcome =
                self.conform_ordered_values(items, Type::Tuple, |checker, index, item| {
                    checker.conform_proven_child(item, element, vector.position_child(index))
                })?;
            Ok(outcome.result_for(operation))
        } else if definitely_not_collection(input) {
            Ok(operation.invalid())
        } else {
            Err(LITERAL_VECTOR_GAP)
        }
    }

    pub(super) fn conform_projected_collection(
        &mut self,
        input: &Type,
        element: &SpecConversion,
        operation: SpecOperation,
    ) -> Result<Type, &'static str> {
        let result = match self.conform_collection(input, element) {
            Err(COLLECTION_GAP | MAP_GAP) if matches!(operation, SpecOperation::Valid) => {
                self.may_throw();
                return Ok(Type::Boolean);
            }
            Err(COLLECTION_GAP) => return Err(PROJECTION_GAP),
            result => result?,
        };
        let mut outcome = Outcome::from_type(result);
        if matches!(operation, SpecOperation::Conform) {
            outcome.value = match outcome.value {
                Type::Never => Type::Never,
                Type::Tuple(items) | Type::ListTuple(items) | Type::SeqTuple(items) => {
                    let projected = items
                        .iter()
                        .map(|item| item.entry_item(1))
                        .collect::<Option<_>>();
                    Type::SeqTuple(projected.ok_or(PROJECTION_GAP)?)
                }
                Type::Map(_) | Type::Set(_) | Type::Vector(_) => Type::empty_seq(),
                _ => return Err(PROJECTION_GAP),
            };
        }
        Ok(outcome.result_for(operation))
    }

    pub(super) fn conform_string_keyed_map(
        &mut self,
        input: &Type,
        values: &SpecConversion,
        operation: SpecOperation,
        proof: Option<&LiteralContainer>,
    ) -> Result<Type, &'static str> {
        let map = match input {
            Type::Map(map)
                if map.extra == ExtraFields::Closed
                    && map.fields.iter().all(|(key, field)| {
                        field.required
                            && matches!(key, Type::Str(_) | Type::Key(_) | Type::Integer(_))
                    }) =>
            {
                map
            }
            Type::Map(_)
            | Type::Unknown
            | Type::Any
            | Type::SpecValue
            | Type::SpecTruthy
            | Type::SpecCollection(_) => {
                if proof.is_some() {
                    return Err(LITERAL_STRING_KEYED_MAP_GAP);
                }
                if matches!(operation, SpecOperation::Valid) {
                    self.may_throw();
                    return Ok(Type::Boolean);
                }
                return Err(STRING_KEYED_MAP_GAP);
            }
            _ => return Ok(operation.invalid()),
        };
        // A tuple checks its key before its value. Other entries can run in
        // an unknown map traversal order, even when one key will fail.
        let bad_key = map.fields.keys().any(|key| !matches!(key, Type::Str(_)));
        let conversions = map.fields.iter().filter_map(|(key, field)| {
            matches!(key, Type::Str(_)).then_some((key, field, Cow::Borrowed(values)))
        });
        let outcome = self.conform_map_values(map, conversions, bad_key, proof)?;
        Ok(outcome.result_for(operation))
    }

    pub(super) fn conform_proven_map(
        &mut self,
        input: &Type,
        fields: &BTreeMap<Type, MapFieldConversion>,
        operation: SpecOperation,
        proof: Option<&LiteralContainer>,
    ) -> Result<Type, &'static str> {
        if let (Type::Map(map), Some(proof @ LiteralContainer::Map(_))) = (input, proof) {
            self.conform_map(map, fields, operation, Some(proof))
        } else if matches!(proof, Some(LiteralContainer::Vector(_)))
            || map_predicate(input) == Type::Bool(false)
        {
            Ok(operation.invalid())
        } else {
            Err(LITERAL_MAP_GAP)
        }
    }

    fn conform_map(
        &mut self,
        map: &MapType,
        fields: &BTreeMap<Type, MapFieldConversion>,
        operation: SpecOperation,
        proof: Option<&LiteralContainer>,
    ) -> Result<Type, &'static str> {
        // Required keys are tested before any of their conformers run.
        if fields
            .iter()
            .any(|(key, field)| field.required && matches!(map.slot(key), MapSlot::Absent))
        {
            return Ok(operation.invalid());
        }
        if map.extra != ExtraFields::Closed
            || map
                .fields
                .iter()
                .any(|(key, field)| !field.required || !matches!(key, Type::Key(_)))
        {
            return Err(MAP_GAP);
        }
        let conversions = self.map_field_conversions(map, fields)?;
        let outcome = self.conform_map_values(map, conversions, false, proof)?;
        Ok(outcome.result_for(operation))
    }

    fn conform_map_values<'a>(
        &mut self,
        map: &MapType,
        conversions: impl Iterator<Item = MapConversion<'a>>,
        mut invalid: bool,
        proof: Option<&LiteralContainer>,
    ) -> Result<Outcome, &'static str> {
        let mut conformed = map.clone();
        let mut can_succeed = !invalid;
        // Input map order is unknown. Invalid values and exceptions from
        // different fields may both be observable.
        for (key, input, conversion) in conversions {
            let conversion = conversion.as_ref();
            let outcome = if let Some(proof) = proof {
                self.conform_proven_child(&input.ty, conversion, proof.map_field(key))?
            } else {
                self.conform_child(&input.ty, conversion)?
            };
            invalid |= outcome.invalid;
            can_succeed &= outcome.value != Type::Never;
            let canonical = match conversion {
                SpecConversion::CheckedInt32 => {
                    input.canonical_int() || outcome.value.fits_integer_cache()
                }
                _ => {
                    input.canonical_int()
                        && self.identity(&input.ty, conversion, false) == Identity::Same
                }
            };
            conformed.set_field(
                key.clone(),
                Field::new(outcome.value, true).with_canonical_int(canonical),
            );
        }
        Ok(Outcome {
            value: if can_succeed {
                Type::Map(conformed)
            } else {
                Type::Never
            },
            invalid,
        })
    }

    fn map_field_conversions<'a>(
        &mut self,
        map: &'a MapType,
        fields: &'a BTreeMap<Type, MapFieldConversion>,
    ) -> Result<impl Iterator<Item = MapConversion<'a>> + use<'a>, &'static str> {
        let mut extra = Vec::new();
        for (key, input) in &map.fields {
            let Type::Key(name) = key else { continue };
            let Some((namespace, local)) =
                name.strip_prefix(':').and_then(|name| name.split_once('/'))
            else {
                continue;
            };
            if namespace.is_empty() || local.is_empty() || fields.contains_key(key) {
                continue;
            }
            // A source registration can affect a qualified key absent from
            // the shape. Record its absence as well for incremental checking.
            let (registered, conversion) = self.read_spec(name);
            let conversion = conversion.ok_or(if registered.is_some() {
                REGISTRY_CONVERSION_GAP
            } else {
                REGISTRY_GAP
            })?;
            // A field type alone cannot prove that a nested value has the
            // ordinary persistent map or vector behavior a bundle needs.
            if matches!(conversion, SpecConversion::Structured(_)) {
                return Err(REGISTRY_CONVERSION_GAP);
            }
            let provider = self
                .globals
                .spec_provider(name)
                .filter(|provider| provider.is_self_contained())
                .ok_or(REGISTRY_CONVERSION_GAP)?;
            if !self.model.loads_spec(provider) {
                return Err(REGISTRY_LOAD_GAP);
            }
            extra.push((key, input, Cow::Owned(conversion)));
        }
        let declared = fields.iter().filter_map(move |(key, field)| {
            let input = map.fields.get(key)?;
            Some((key, input, Cow::Borrowed(field.conversion.as_ref()?)))
        });
        Ok(declared.chain(extra))
    }

    fn conform_entry(
        &mut self,
        items: &[Type; 2],
        element: &SpecConversion,
    ) -> Result<Type, &'static str> {
        let left = self.conform_child(&items[0], element)?;
        if left.value == Type::Never {
            return Ok(with_invalid(Type::Never, left.invalid));
        }
        let right = self.conform_child(&items[1], element)?;
        let invalid = left.invalid || right.invalid;
        if right.value == Type::Never {
            return Ok(with_invalid(Type::Never, invalid));
        }
        let values = Box::new([left.value, right.value]);
        let result = match self
            .identity(&items[0], element, false)
            .combine(self.identity(&items[1], element, false))
        {
            Identity::Same => Type::MapEntry(values),
            Identity::Changed => Type::Tuple(values),
            Identity::Unknown => return Err(ENTRY_GAP),
        };
        Ok(with_invalid(result, invalid))
    }

    fn conform_ordered(
        &mut self,
        items: &[Type],
        element: &SpecConversion,
        rebuild: fn(Box<[Type]>) -> Type,
    ) -> Result<Type, &'static str> {
        let outcome = self.conform_ordered_values(items, rebuild, |checker, _, item| {
            checker.conform_child(item, element)
        })?;
        Ok(outcome.result_for(SpecOperation::Conform))
    }

    fn conform_ordered_values(
        &mut self,
        items: &[Type],
        rebuild: fn(Box<[Type]>) -> Type,
        mut conform: impl FnMut(&mut Self, usize, &Type) -> Result<Outcome, &'static str>,
    ) -> Result<Outcome, &'static str> {
        let mut conformed = Vec::with_capacity(items.len());
        let mut invalid = false;
        for (index, item) in items.iter().enumerate() {
            let outcome = conform(self, index, item)?;
            invalid |= outcome.invalid;
            if outcome.value == Type::Never {
                // Later elements cannot run once this one fails.
                return Ok(Outcome {
                    value: Type::Never,
                    invalid,
                });
            }
            conformed.push(outcome.value);
        }
        Ok(Outcome {
            value: rebuild(conformed.into()),
            invalid,
        })
    }

    fn conform_child(
        &mut self,
        ty: &Type,
        conversion: &SpecConversion,
    ) -> Result<Outcome, &'static str> {
        Ok(Outcome::from_type(self.convert_spec(
            ty,
            conversion,
            SpecOperation::Conform,
        )?))
    }

    fn conform_proven_child(
        &mut self,
        input: &Type,
        conversion: &SpecConversion,
        proof: Option<&LiteralContainer>,
    ) -> Result<Outcome, &'static str> {
        let conformed = self.conform_proven(input, conversion, SpecOperation::Conform, proof)?;
        Ok(Outcome::from_type(conformed))
    }

    /// A successful type-preserving conversion can still replace the JVM object.
    fn identity(
        &mut self,
        input: &Type,
        conversion: &SpecConversion,
        canonical_int: bool,
    ) -> Identity {
        match conversion {
            SpecConversion::CheckedInt32 if canonical_int => Identity::Same,
            SpecConversion::CheckedInt32 if input.is_arbitrary_integer() => Identity::Changed,
            SpecConversion::Name if input.subtype_of(&Type::String) => Identity::Same,
            SpecConversion::Name
                if input.subtype_of(&Type::Keyword) || input.subtype_of(&Type::Symbol) =>
            {
                Identity::Changed
            }
            SpecConversion::VectorWrapField(_) | SpecConversion::GuardedMap(_)
                if *input == Type::Nil =>
            {
                Identity::Same
            }
            SpecConversion::VectorWrapField(_) | SpecConversion::GuardedMap(_)
                if nil_part(input) == Type::Never =>
            {
                Identity::Changed
            }
            SpecConversion::MapOrWrapField(_) => match map_wrap(input) {
                MapWrap::Wrap => Identity::Changed,
                MapWrap::Keep => Identity::Same,
                MapWrap::Either => Identity::Unknown,
            },
            SpecConversion::Enum { .. }
            | SpecConversion::WrapField(_)
            | SpecConversion::LiveLookupMap(_)
            | SpecConversion::TaggedOr(_)
            | SpecConversion::ProjectedCollection(_) => Identity::Changed,
            SpecConversion::String
            | SpecConversion::Keyword
            | SpecConversion::Any
            | SpecConversion::IdentityDomain(_)
            | SpecConversion::Identity => Identity::Same,
            SpecConversion::TaggedChoice(branches) => {
                self.choice_identity(input, branches, canonical_int)
            }
            SpecConversion::StringKeyedMap { values, .. } if let Type::Map(map) = input => {
                map.fields.values().fold(Identity::Same, |summary, field| {
                    summary.combine(self.identity(&field.ty, values, field.canonical_int()))
                })
            }
            SpecConversion::Structured(spec)
                if let (Type::Map(map), StructuredConversion::Map(fields)) =
                    (input, spec.as_ref()) =>
            {
                let Ok(conversions) = self.map_field_conversions(map, fields) else {
                    return Identity::Unknown;
                };
                conversions.fold(Identity::Same, |summary, (_, field, conversion)| {
                    summary.combine(self.identity(&field.ty, &conversion, field.canonical_int()))
                })
            }
            SpecConversion::Structured(spec)
                if let StructuredConversion::Collection { element, .. } = spec.as_ref() =>
            {
                match input {
                    Type::Map(map) if map.emptiness() == Some(true) => Identity::Same,
                    Type::Tuple(items) => self.ordered_identity(items, element),
                    Type::MapEntry(items) => self.ordered_identity(items.as_ref(), element),
                    Type::ListTuple(items) | Type::SeqTuple(items) if !items.is_empty() => {
                        Identity::Changed
                    }
                    _ => Identity::Unknown,
                }
            }
            _ => Identity::Unknown,
        }
    }

    fn ordered_identity(&mut self, items: &[Type], element: &SpecConversion) -> Identity {
        items.iter().fold(Identity::Same, |summary, item| {
            summary.combine(self.identity(item, element, false))
        })
    }

    fn choice_identity(
        &mut self,
        input: &Type,
        branches: &[SpecConversion],
        canonical_int: bool,
    ) -> Identity {
        if let Type::Map(map) = input
            && map.emptiness() == Some(true)
        {
            // Every modeled successful branch preserves the empty map.
            return Identity::Same;
        }
        branches
            .iter()
            .find(|branch| match (input, branch) {
                (Type::String | Type::Str(_), SpecConversion::String) => true,
                (Type::Key(_) | Type::Keyword | Type::SpecKeyword, SpecConversion::Enum { .. }) => {
                    true
                }
                (Type::Map(map), SpecConversion::Structured(spec)) => {
                    map.emptiness() == Some(false)
                        && matches!(spec.as_ref(), StructuredConversion::Map(_))
                }
                (
                    Type::Tuple(_) | Type::MapEntry(_) | Type::ListTuple(_) | Type::SeqTuple(_),
                    SpecConversion::Structured(spec),
                ) => matches!(spec.as_ref(), StructuredConversion::Collection { .. }),
                _ => false,
            })
            .map_or(Identity::Unknown, |branch| {
                self.identity(input, branch, canonical_int)
            })
    }
}

fn definitely_not_collection(ty: &Type) -> bool {
    match ty {
        Type::Nil
        | Type::Bool(_)
        | Type::Boolean
        | Type::Key(_)
        | Type::Keyword
        | Type::SpecKeyword
        | Type::Str(_)
        | Type::String
        | Type::Char(_)
        | Type::Character
        | Type::Integer(_)
        | Type::Integral(IntegerType {
            class: IntegerClass::Fixed,
            ..
        }) => true,
        Type::Union(members) => members.iter().all(definitely_not_collection),
        _ => false,
    }
}

fn distinct_set_members(left: &Type, right: &Type) -> bool {
    if literal_equality(left, right) == Some(false) {
        return true;
    }
    let (Type::Map(left), Type::Map(right)) = (left, right) else {
        return false;
    };
    left.fields.iter().any(|(key, field)| {
        field.required
            && right.fields.get(key).is_some_and(|other| {
                other.required && literal_equality(&field.ty, &other.ty) == Some(false)
            })
    })
}

fn hash_collection_element(conversion: &SpecConversion) -> Option<&SpecConversion> {
    match conversion {
        SpecConversion::Structured(spec) => match spec.as_ref() {
            StructuredConversion::Collection { element, .. } => Some(element),
            StructuredConversion::Map(_) => None,
        },
        SpecConversion::TaggedChoice(branches) => {
            let [
                SpecConversion::Structured(first),
                SpecConversion::Structured(second),
            ] = branches.as_ref()
            else {
                return None;
            };
            match (first.as_ref(), second.as_ref()) {
                (
                    StructuredConversion::Collection { element, .. },
                    StructuredConversion::Map(_),
                )
                | (
                    StructuredConversion::Map(_),
                    StructuredConversion::Collection { element, .. },
                ) => Some(element),
                _ => None,
            }
        }
        _ => None,
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Identity {
    Same,
    Changed,
    Unknown,
}

impl Identity {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Changed, _) | (_, Self::Changed) => Self::Changed,
            (Self::Same, Self::Same) => Self::Same,
            _ => Self::Unknown,
        }
    }
}

struct Outcome {
    value: Type,
    invalid: bool,
}

impl Outcome {
    fn from_type(result: Type) -> Self {
        let invalid_key = |ty: &Type| matches!(ty, Type::Key(key) if key == SPEC_INVALID);
        match result {
            Type::Union(mut parts) => {
                let invalid = parts.iter().any(invalid_key);
                parts.retain(|ty| !invalid_key(ty));
                Self {
                    value: Type::union(parts),
                    invalid,
                }
            }
            key if invalid_key(&key) => Self {
                value: Type::Never,
                invalid: true,
            },
            value => Self {
                value,
                invalid: false,
            },
        }
    }

    fn result_for(self, operation: SpecOperation) -> Type {
        let value = if self.value == Type::Never {
            Type::Never
        } else {
            operation.success(self.value)
        };
        if self.invalid {
            Type::union([value, operation.invalid()])
        } else {
            value
        }
    }
}

fn with_invalid(value: Type, invalid: bool) -> Type {
    Outcome { value, invalid }.result_for(SpecOperation::Conform)
}
