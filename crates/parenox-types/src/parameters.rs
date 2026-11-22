//! Rigid parameters and substitution in the supported generic signature positions.
use std::{borrow::Cow, sync::Arc};

use super::{Arity, Compatibility, ExtraFields, MapSlot, Signature, Type};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Parameter {
    // Two definitions may both spell their binder a without sharing a type.
    pub owner: String,
    pub name: String,
    pub upper: Option<Type>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Parameters {
    pub binders: Box<[Arc<Parameter>]>,
    pub output: Option<usize>,
}

pub const MAX_TYPE_PARAMETERS: usize = 3;

#[derive(Default)]
pub struct TypeArguments<'a>([Option<Cow<'a, Type>>; MAX_TYPE_PARAMETERS]);

impl<'a> TypeArguments<'a> {
    pub fn get(&self, index: usize) -> Option<&Type> {
        self.0[index].as_deref()
    }

    pub fn insert(&mut self, index: usize, ty: Cow<'a, Type>) {
        self.0[index] = Some(ty);
    }

    pub fn missing(&self) -> [bool; MAX_TYPE_PARAMETERS] {
        std::array::from_fn(|index| self.0[index].is_none())
    }
}

#[derive(Clone, Copy)]
enum ParameterSlot<'a> {
    Direct,
    VectorElement,
    DictionaryKey,
    DictionaryValue,
    RequiredField(&'a Type),
}

pub struct InputWitness<'a> {
    pub index: usize,
    slot: ParameterSlot<'a>,
}

pub struct CallbackWitness<'a> {
    pub index: usize,
    pub map_key: Option<&'a Type>,
    pub signature: &'a Signature,
    slot: ParameterSlot<'a>,
}

impl CallbackWitness<'_> {
    pub fn callback_type<'a>(&self, argument: &'a Type) -> Option<&'a Type> {
        let Some(key) = self.map_key else {
            return Some(argument);
        };
        let Type::Map(map) = argument else {
            return None;
        };
        let field = map.fields.get(key)?;
        field.value_constraint().filter(|_| field.required)
    }

    pub fn evidence<'a>(&self, result: &'a Type) -> Option<Cow<'a, Type>> {
        self.slot.evidence(result)
    }
}

impl InputWitness<'_> {
    pub fn evidence<'a>(&self, argument: &'a Type) -> Option<Cow<'a, Type>> {
        self.slot.evidence(argument)
    }

    pub fn evidence_site(&self) -> String {
        match self.slot {
            ParameterSlot::Direct => "this argument".into(),
            ParameterSlot::VectorElement => "this vector's elements".into(),
            ParameterSlot::DictionaryKey => "this map's keys".into(),
            ParameterSlot::DictionaryValue => "this map's values".into(),
            ParameterSlot::RequiredField(key) => format!("required field {key} of this map"),
        }
    }
}

impl ParameterSlot<'_> {
    fn evidence(self, ty: &Type) -> Option<Cow<'_, Type>> {
        let candidate = self.project(ty)?;
        candidate.is_generic_evidence().then_some(candidate)
    }

    fn project(self, ty: &Type) -> Option<Cow<'_, Type>> {
        if matches!(self, Self::Direct) {
            return Some(Cow::Borrowed(ty));
        }
        if let Type::Union(parts) = ty {
            let elements = parts
                .iter()
                .map(|ty| self.project(ty))
                .collect::<Option<Vec<_>>>()?;
            return Some(Cow::Owned(Type::union(
                elements.into_iter().map(Cow::into_owned),
            )));
        }
        match (self, ty) {
            (Self::VectorElement, Type::Vector(element)) => Some(Cow::Borrowed(element)),
            (Self::VectorElement, Type::Tuple(items)) => {
                Some(Cow::Owned(Type::union(items.iter().cloned())))
            }
            (Self::VectorElement, Type::MapEntry(items)) => {
                Some(Cow::Owned(Type::union(items.iter().cloned())))
            }
            (Self::DictionaryKey | Self::DictionaryValue, Type::Map(map)) => {
                let key_slot = matches!(self, Self::DictionaryKey);
                let additional = match &map.extra {
                    ExtraFields::Open => return None,
                    ExtraFields::Closed => None,
                    ExtraFields::Typed(types) if types.contains(&Type::Never) => None,
                    ExtraFields::Typed(types) => Some(if key_slot { &types[0] } else { &types[1] }),
                };
                if map.fields.is_empty() {
                    return additional
                        .map(Cow::Borrowed)
                        .or(Some(Cow::Owned(Type::Never)));
                }
                let types = map
                    .fields
                    .iter()
                    .filter(|(_, field)| field.ty != Type::Never)
                    .map(|(key, field)| if key_slot { key } else { &field.ty })
                    .chain(additional);
                Some(Cow::Owned(Type::union(types.cloned())))
            }
            (Self::RequiredField(key), Type::Map(map)) => match map.slot(key) {
                MapSlot::Present(value) => Some(Cow::Borrowed(value)),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Parameters {
    pub fn iter(&self) -> impl Iterator<Item = &Arc<Parameter>> {
        self.binders.iter()
    }

    pub fn inputs(&self) -> impl Iterator<Item = (usize, &Parameter)> {
        self.binders
            .iter()
            .enumerate()
            .filter_map(|(index, parameter)| {
                (Some(index) != self.output).then_some((index, parameter.as_ref()))
            })
    }

    pub fn output(&self) -> Option<(usize, &Parameter)> {
        self.output
            .map(|index| (index, self.binders[index].as_ref()))
    }

    pub fn needs_missing_input(&self, ty: &Type, missing: &[bool; MAX_TYPE_PARAMETERS]) -> bool {
        self.inputs()
            .any(|(index, parameter)| missing[index] && ty.mentions_parameter(parameter))
    }
}

impl Parameter {
    pub fn check_bound(&self, evidence: &Type) -> Compatibility {
        self.upper
            .as_ref()
            .map_or(Compatibility::Proven, |upper| evidence.compatibility(upper))
    }
}

type Replacements<'a> = [Option<(&'a Parameter, &'a Type)>; MAX_TYPE_PARAMETERS];

impl Type {
    pub fn signature_at(&self, arity: Arity) -> Option<&Signature> {
        let Self::Function(signatures) = self else {
            return None;
        };
        signatures.iter().find(|sig| sig.arity() == arity)
    }

    pub(crate) fn is_generic_evidence(&self) -> bool {
        self != &Self::Never
            && !self.contains_unknown()
            && (!self.contains_parameter() || matches!(self, Self::Parameter(_)))
    }

    pub fn contains_parameter(&self) -> bool {
        self.any_component(|ty| matches!(ty, Type::Parameter(_)))
    }

    pub fn mentions_parameter(&self, parameter: &Parameter) -> bool {
        self.any_component(|ty| ty.is_parameter(parameter))
    }

    pub(crate) fn is_parameter(&self, parameter: &Parameter) -> bool {
        matches!(self, Self::Parameter(found) if found.as_ref() == parameter)
    }

    pub fn has_parameter_slot(&self, parameter: &Parameter) -> bool {
        self.parameter_slot(parameter, false).is_some()
    }

    pub fn has_nullable_parameter(&self, parameter: &Parameter) -> bool {
        self.nullable_parameter()
            .is_some_and(|ty| ty.is_parameter(parameter))
    }

    fn nullable_parameter(&self) -> Option<&Type> {
        let Self::Union(parts) = self else {
            return None;
        };
        match parts.as_slice() {
            [ty @ Self::Parameter(_), Self::Nil] | [Self::Nil, ty @ Self::Parameter(_)] => Some(ty),
            _ => None,
        }
    }

    fn parameter_slot(
        &self,
        parameter: &Parameter,
        allow_callback_fields: bool,
    ) -> Option<ParameterSlot<'_>> {
        match self {
            Self::Parameter(found) if found.as_ref() == parameter => Some(ParameterSlot::Direct),
            Self::Vector(element) if element.is_parameter(parameter) => {
                Some(ParameterSlot::VectorElement)
            }
            Self::Map(map) => match &map.extra {
                ExtraFields::Typed(types)
                    if map.fields.is_empty()
                        && types.iter().all(|ty| {
                            matches!(ty, Self::Parameter(_)) || !ty.contains_parameter()
                        }) =>
                {
                    // A shared binder uses the value's inferred type. A closed
                    // shape can retain a literal key when its values widen.
                    if types[1].is_parameter(parameter) {
                        Some(ParameterSlot::DictionaryValue)
                    } else {
                        types[0]
                            .is_parameter(parameter)
                            .then_some(ParameterSlot::DictionaryKey)
                    }
                }
                ExtraFields::Closed
                    if map.fields.values().all(|field| {
                        field.required
                            && (matches!(field.ty, Self::Parameter(_))
                                || (allow_callback_fields && matches!(field.ty, Self::Function(_)))
                                || !field.ty.contains_parameter())
                    }) =>
                {
                    map.fields.iter().find_map(|(key, field)| {
                        field
                            .ty
                            .is_parameter(parameter)
                            .then_some(ParameterSlot::RequiredField(key))
                    })
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn substitute(&self, replacements: &Replacements<'_>) -> Self {
        match self {
            Self::Parameter(found) => replacements
                .iter()
                .flatten()
                .find_map(|(parameter, replacement)| {
                    (*parameter == found.as_ref()).then_some(*replacement)
                })
                .unwrap_or(self)
                .clone(),
            Self::Vector(element) if matches!(element.as_ref(), Self::Parameter(_)) => {
                Self::Vector(Box::new(element.substitute(replacements)))
            }
            Self::Union(_) if let Some(parameter) = self.nullable_parameter() => {
                parameter.substitute(replacements).nilable()
            }
            Self::Map(map) if self.contains_parameter() => {
                let mut map = map.clone();
                if let ExtraFields::Typed(types) = &mut map.extra {
                    for ty in types
                        .iter_mut()
                        .filter(|ty| matches!(ty, Self::Parameter(_)))
                    {
                        *ty = ty.substitute(replacements);
                    }
                }
                for field in map.fields.values_mut() {
                    if matches!(field.ty, Self::Parameter(_) | Self::Function(_)) {
                        field.ty = field.ty.substitute(replacements);
                    }
                }
                Self::Map(map)
            }
            Self::Function(signatures) if self.contains_parameter() => Self::Function(
                signatures
                    .iter()
                    .map(|signature| {
                        if signature.parameters.is_none() {
                            signature.substitute(replacements)
                        } else {
                            signature.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .into(),
            ),
            // The parser rejects deeper occurrences, unions and other containers.
            ty => ty.clone(),
        }
    }
}

impl Signature {
    /// Compare declarations up to renaming, not up to specialization or subtyping.
    pub fn same_type(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        let (Some(ours), Some(theirs)) = (&self.parameters, &other.parameters) else {
            return false;
        };
        if self.arity() != other.arity()
            || ours.binders.len() != theirs.binders.len()
            || ours.output != theirs.output
            || !ours
                .iter()
                .zip(theirs.iter())
                .all(|(a, b)| a.upper == b.upper)
        {
            return false;
        }
        // Compare under the same rigid binders. A simultaneous replacement
        // matters when a declaration swaps the spelling of its two binders.
        let mut arguments = TypeArguments::default();
        for (index, parameter) in theirs.iter().enumerate() {
            arguments.insert(index, Cow::Owned(Type::Parameter(parameter.clone())));
        }
        let renamed = self.with_type_arguments(ours, &arguments, None);
        renamed.args == other.args && renamed.rest == other.rest && renamed.ret == other.ret
    }

    /// Choose one instantiation from a callback's declared inputs, never from its expected return.
    pub fn specialize_callback(&self, expected: &Self) -> Result<Self, Compatibility> {
        let binders = self.parameters.as_ref().ok_or(Compatibility::Unproven)?;
        if self.arity() != expected.arity() {
            return Err(Compatibility::Mismatch);
        }
        if expected.parameters.is_some() {
            return Err(Compatibility::Unproven);
        }
        let mut arguments = self.input_types(binders, |index| &expected.args[index]);
        let mut result = Compatibility::all(binders.inputs().map(|(index, parameter)| {
            arguments
                .get(index)
                .map_or(Compatibility::Unproven, |ty| parameter.check_bound(ty))
        }));
        if let Some((index, output_parameter)) = binders.output() {
            let output_source = self
                .output_callback(output_parameter)
                .ok_or(Compatibility::Unproven)?;
            // A fixed, monomorphic callback argument can witness the output.
            // Do not recursively infer a generic callback inside another.
            let nested = output_source
                .callback_type(&expected.args[output_source.index])
                .and_then(|ty| ty.signature_at(output_source.signature.arity()))
                .filter(|sig| sig.parameters.is_none());
            if let Some(evidence) = nested.and_then(|sig| output_source.evidence(&sig.ret)) {
                result = result.min(output_parameter.check_bound(&evidence));
                arguments.insert(index, evidence);
            } else {
                result = result.min(Compatibility::Unproven);
            }
        }
        if result == Compatibility::Proven {
            Ok(self.with_type_arguments(binders, &arguments, None))
        } else {
            Err(result)
        }
    }

    pub fn input_types<'a>(
        &self,
        binders: &Parameters,
        argument_at: impl Fn(usize) -> &'a Type,
    ) -> TypeArguments<'a> {
        let mut arguments = TypeArguments::default();
        for (index, parameter) in binders.inputs() {
            if let Some(witness) = self.input_witness(parameter)
                && let Some(evidence) = witness.evidence(argument_at(witness.index))
            {
                arguments.insert(index, evidence);
            }
        }
        arguments
    }

    pub fn input_witness(&self, parameter: &Parameter) -> Option<InputWitness<'_>> {
        self.args
            .iter()
            .position(|ty| ty.is_parameter(parameter))
            .map(|index| InputWitness {
                index,
                slot: ParameterSlot::Direct,
            })
            .or_else(|| {
                self.args.iter().enumerate().find_map(|(index, ty)| {
                    ty.parameter_slot(parameter, true)
                        .map(|slot| InputWitness { index, slot })
                })
            })
    }

    pub fn output_callback(&self, output: &Parameter) -> Option<CallbackWitness<'_>> {
        self.args.iter().enumerate().find_map(|(index, ty)| {
            let (callback, map_key) = match ty {
                Type::Map(map) if matches!(map.extra, ExtraFields::Closed) => {
                    let mut fields = map
                        .fields
                        .iter()
                        .filter(|(_, field)| field.ty.mentions_parameter(output));
                    let (key, field) = fields.next()?;
                    if !field.required || fields.next().is_some() {
                        return None;
                    }
                    (field.value_constraint()?, Some(key))
                }
                _ => (ty, None),
            };
            let Type::Function(signatures) = callback else {
                return None;
            };
            let [callback] = signatures.as_ref() else {
                return None;
            };
            if callback.rest.is_some() {
                return None;
            }
            let slot = callback.ret.parameter_slot(output, false)?;
            Some(CallbackWitness {
                index,
                map_key,
                signature: callback,
                slot,
            })
        })
    }

    pub fn with_type_arguments(
        &self,
        binders: &Parameters,
        arguments: &TypeArguments<'_>,
        fallback: Option<&Type>,
    ) -> Self {
        self.substitute(&std::array::from_fn(|index| {
            let parameter = binders.binders.get(index)?;
            let ty = arguments.get(index).or(fallback)?;
            Some((parameter.as_ref(), ty))
        }))
    }

    fn substitute(&self, replacements: &Replacements<'_>) -> Self {
        Self {
            args: self
                .args
                .iter()
                .map(|ty| ty.substitute(replacements))
                .collect(),
            rest: self.rest.as_ref().map(|ty| ty.substitute(replacements)),
            ret: self.ret.substitute(replacements),
            parameters: None,
            nil_preserving: self.nil_preserving,
        }
    }
}
