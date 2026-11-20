//! Facts from a successful named spec check. A projection need not capture every
//! runtime constraint; it cannot prove success or characterize the false branch.
mod conform;
mod projection;

use crate::Diagnostic;
use crate::checker::{
    Checker, Env, Value,
    calls::{call_returns, iteration_may_throw, vector_predicate},
    maps::{map_predicate, narrow_map_predicate},
    model::{SpecConversion, SpecProvider},
};
use crate::reader::{Form, Span};
use crate::types::{
    Compatibility, ExtraFields, Field, IntegerRange, IntegerType, MapSlot, MapType, SPEC_INVALID,
    Type,
};

const SPEC_LOAD_GAP: &str = "Cannot prove the named spec and its registrations are loaded";

#[derive(Clone, Copy)]
enum SpecOperation {
    Valid,
    Conform,
}

enum MapWrap {
    Wrap,
    Keep,
    Either,
}

fn map_wrap(input: &Type) -> MapWrap {
    match (vector_predicate(input), map_predicate(input)) {
        (Type::Bool(true), _) | (_, Type::Bool(false)) => MapWrap::Wrap,
        (Type::Bool(false), Type::Bool(true)) => MapWrap::Keep,
        _ => MapWrap::Either,
    }
}

impl SpecOperation {
    fn success(self, value: Type) -> Type {
        match self {
            Self::Valid => Type::Bool(true),
            Self::Conform => value,
        }
    }

    fn invalid(self) -> Type {
        match self {
            Self::Valid => Type::Bool(false),
            Self::Conform => Type::Key(SPEC_INVALID.into()),
        }
    }
}

impl Checker<'_> {
    pub(in crate::checker) fn is_spec_validation(&self, name: &str) -> bool {
        matches!(self.spec_operation(name), Some(SpecOperation::Valid))
    }

    fn spec_operation(&self, name: &str) -> Option<SpecOperation> {
        let op = match name {
            "clojure.spec.alpha/valid?" => SpecOperation::Valid,
            "clojure.spec.alpha/conform" => SpecOperation::Conform,
            _ => return None,
        };
        (!self
            .globals
            .project_namespaces
            .contains("clojure.spec.alpha"))
        .then_some(op)
    }

    pub(in crate::checker) fn spec_operation_call(
        &mut self,
        name: &str,
        args: &[Form],
        span: Span,
        env: &Env,
    ) -> Option<Value> {
        let op = self.spec_operation(name)?;
        if let [spec, input] = args
            && let Some(value) = self.conform_set_literal(spec, input, op, env)
        {
            return Some(value);
        }
        let values: Vec<_> = self.eval_args(args, env, true).collect();
        let returns = call_returns(&values);
        // Clojure's Specize implementation for Keyword ignores the optional
        // form when resolving the registry entry. It is still evaluated.
        let spec = match (op, args) {
            (_, [spec, _]) | (SpecOperation::Valid, [spec, _, _]) => spec,
            _ => {
                let count = match op {
                    SpecOperation::Valid => "2 or 3",
                    SpecOperation::Conform => "2",
                };
                self.error(
                    "PNX2554",
                    span,
                    format!("{name} expects {count} arguments, got {}", args.len()),
                );
                if returns {
                    self.may_throw();
                }
                return Some(Value::new(if returns {
                    Type::Unknown
                } else {
                    Type::Never
                }));
            }
        };
        if !returns {
            return Some(Value::new(Type::Never));
        }
        let key = spec
            .keyword()
            .and_then(|key| self.model.validation_key(key));
        let loaded = key
            .as_deref()
            .is_none_or(|key| self.spec_load_is_known(key));
        let input = &values[1].0;
        let provider = key
            .as_deref()
            .and_then(|key| self.globals.spec_provider(key));
        let imported = key
            .as_deref()
            .is_some_and(|key| self.is_imported_structure(key));
        let can_check_map = matches!(op, SpecOperation::Valid)
            && input.is_literal_map()
            && provider.is_some_and(|provider| {
                provider.standalone_registration().is_some() && self.model.loads_spec(provider)
            });
        let (ty, conversion) = key
            .as_deref()
            .map(|key| self.read_spec(key))
            .unwrap_or_default();
        if !loaded {
            return Some(self.unchecked_form(spec.span, SPEC_LOAD_GAP));
        }
        if imported && let Some(conversion) = &conversion {
            return Some(
                match self.conform_proven(
                    &input.known,
                    conversion,
                    op,
                    input.literal_container.as_ref(),
                ) {
                    Ok(ty) => Value::new(ty),
                    Err(reason) => self.unchecked_spec_call(op, spec.span, reason),
                },
            );
        }
        if let Some(fields) = conversion.as_ref().and_then(SpecConversion::map_fields)
            && can_check_map
            && fields.values().all(|field| {
                field
                    .conversion
                    .as_ref()
                    .is_some_and(|child| !matches!(child, SpecConversion::Structured(_)))
            })
        {
            return Some(
                match self.conform_proven_map(
                    &input.known,
                    fields,
                    op,
                    input.literal_container.as_ref(),
                ) {
                    Ok(ty) => Value::new(ty),
                    Err(reason) => self.unchecked_spec_call(op, spec.span, reason),
                },
            );
        }
        if let Some(conversion) = conversion.filter(|conversion| {
            matches!(op, SpecOperation::Conform)
                || !matches!(
                    conversion,
                    SpecConversion::String | SpecConversion::Structured(_)
                )
        }) {
            if input.is_literal_map()
                && let (Type::Map(map), SpecConversion::GuardedMap(projection)) =
                    (&input.known, &conversion)
            {
                return Some(Value::new(projection::conform_literal(map, projection, op)));
            }
            if matches!(op, SpecOperation::Conform) && input.is_literal_map() {
                match conversion {
                    SpecConversion::MapOrWrapField(_) => return Some(input.clone()),
                    SpecConversion::VectorWrapField(_) => return Some(Value::new(Type::Nil)),
                    _ => {}
                }
            }
            let result = self.convert_spec(&input.known, &conversion, op);
            return Some(match result {
                Ok(ty) => Value::new(ty),
                Err(reason) => self.unchecked_form(spec.span, reason),
            });
        }
        if let SpecOperation::Conform = op {
            return Some(self.unchecked_form(
                spec.span,
                "s/conform requires a named spec with a supported conversion",
            ));
        }
        let gap = match (key, ty) {
            (Some(_), Some(ty)) if usable(ty) => None,
            (Some(key), Some(_)) => Some(format!("s/valid? cannot refine unchecked spec {key}")),
            (Some(key), None) => Some(format!("No source declaration for s/valid? spec {key}")),
            (None, _) => Some("s/valid? requires a literal, resolved named spec".into()),
        };
        // Even with a known projection, the runtime registry can impose additional
        // checks (for example on qualified extra keys of an open map).
        self.may_throw();
        if let Some(reason) = gap {
            self.diagnostics
                .push(Diagnostic::analysis_gap("PNX9001", spec.span, reason));
        }
        Some(Value::new(Type::Boolean))
    }

    fn unchecked_spec_call(&mut self, op: SpecOperation, span: Span, reason: &str) -> Value {
        let value = self.unchecked_form(span, reason);
        match op {
            SpecOperation::Valid => Value::new(Type::Boolean),
            SpecOperation::Conform => value,
        }
    }

    fn convert_spec(
        &mut self,
        input: &Type,
        conversion: &SpecConversion,
        op: SpecOperation,
    ) -> Result<Type, &'static str> {
        match input {
            Type::Never => Ok(Type::Never),
            Type::Union(members) => {
                let outcomes = members
                    .iter()
                    .map(|ty| self.convert_spec(ty, conversion, op))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Type::union(outcomes))
            }
            Type::Parameter(parameter)
                if !conversion.is_field_conformer()
                    && !matches!(conversion, SpecConversion::Identity | SpecConversion::Any) =>
            {
                self.convert_spec(
                    parameter.upper.as_ref().unwrap_or(&Type::Unknown),
                    conversion,
                    op,
                )
            }
            ty => Ok(match conversion {
                SpecConversion::CheckedInt32 => self.convert_int32(ty, op),
                SpecConversion::WrapField(key) => match op {
                    SpecOperation::Valid => Type::Bool(true),
                    SpecOperation::Conform => SpecConversion::wrap_field(key, ty.clone()),
                },
                SpecConversion::VectorWrapField(key) => match op {
                    SpecOperation::Valid => Type::Bool(true),
                    SpecOperation::Conform => match vector_predicate(ty) {
                        Type::Bool(true) => SpecConversion::wrap_field(key, ty.clone()),
                        Type::Bool(false) => Type::Nil,
                        _ => SpecConversion::wrap_field(key, Type::Vector(Box::new(Type::Any)))
                            .nilable(),
                    },
                },
                SpecConversion::MapOrWrapField(key) => match op {
                    SpecOperation::Valid => Type::Bool(true),
                    SpecOperation::Conform => match map_wrap(ty) {
                        MapWrap::Wrap => SpecConversion::wrap_field(key, ty.clone()),
                        MapWrap::Keep => ty.clone(),
                        MapWrap::Either => {
                            let map = if map_predicate(ty) == Type::Bool(true) {
                                ty.clone()
                            } else {
                                narrow_map_predicate(&Type::Any, true)
                            };
                            Type::union([map, SpecConversion::wrap_field(key, ty.clone())])
                        }
                    },
                },
                SpecConversion::GuardedMap(projection) => {
                    self.conform_opaque_projection(ty, projection, op)
                }
                SpecConversion::LiveLookupMap(projection) => {
                    self.may_throw();
                    match op {
                        SpecOperation::Conform => projection.map_result(),
                        SpecOperation::Valid => Type::Bool(true),
                    }
                }
                SpecConversion::Name => {
                    let result = self.name_result(ty);
                    if result == Type::Never {
                        Type::Never
                    } else {
                        op.success(result)
                    }
                }
                SpecConversion::Identity
                | SpecConversion::IdentityDomain(_)
                | SpecConversion::Any
                | SpecConversion::String
                | SpecConversion::Keyword => {
                    let accepted = conversion.success_input();
                    match ty.compatibility(&accepted) {
                        Compatibility::Proven => op.success(ty.clone()),
                        Compatibility::Mismatch
                            if matches!(conversion, SpecConversion::IdentityDomain(_))
                                && let Some(overlap) = ty.known_intersection(&accepted)
                                && overlap != Type::Never =>
                        {
                            // A general string can pass a finite set predicate
                            // without meeting a contract for a particular literal.
                            Type::union([op.success(overlap), op.invalid()])
                        }
                        Compatibility::Mismatch => op.invalid(),
                        Compatibility::Unproven => {
                            Type::union([op.success(refine(ty, &accepted)), op.invalid()])
                        }
                    }
                }
                SpecConversion::AnyCollection => self.convert_collection(ty, op),
                SpecConversion::Enum { entries, .. } => convert_enum(ty, entries, op),
                SpecConversion::TaggedChoice(branches) => {
                    return self.conform_choice(
                        ty,
                        branches.iter().map(|branch| (None, branch)),
                        op,
                    );
                }
                SpecConversion::TaggedOr(branches) => {
                    return self.conform_choice(
                        ty,
                        branches
                            .iter()
                            .map(|(tag, branch)| (Some(tag.as_str()), branch)),
                        op,
                    );
                }
                SpecConversion::ProjectedCollection(element) => {
                    return self.conform_projected_collection(ty, element, op);
                }
                SpecConversion::StringKeyedMap { values, .. } => {
                    return self.conform_string_keyed_map(ty, values, op, None);
                }
                SpecConversion::Structured(spec) => {
                    return self.conform_structure(ty, spec);
                }
            }),
        }
    }

    fn convert_collection(&mut self, input: &Type, op: SpecOperation) -> Type {
        if iteration_may_throw(input) || matches!(input, Type::SpecValue | Type::SpecTruthy) {
            self.may_throw();
        }
        let expected = SpecConversion::AnyCollection.success_input();
        let compatibility = input.compatibility(&expected);
        if compatibility == Compatibility::Mismatch && !may_pass_collection(input) {
            return op.invalid();
        }
        // With no :kind, spec can build a persistent list from a queue or
        // another seqable collection. Only retain a shape known to survive.
        let success = if compatibility == Compatibility::Proven
            && matches!(
                input,
                Type::Map(_)
                    | Type::MapEntry(_)
                    | Type::Vector(_)
                    | Type::Tuple(_)
                    | Type::Set(_)
                    | Type::List(_)
                    | Type::ListTuple(_)
            ) {
            input.clone()
        } else {
            expected
        };
        if compatibility == Compatibility::Proven {
            op.success(success)
        } else {
            Type::union([op.success(success), op.invalid()])
        }
    }

    fn convert_int32(&mut self, input: &Type, op: SpecOperation) -> Type {
        let output = Type::Integral(IntegerType::FIXED_INT32);
        let fixed = Type::Integral(IntegerType::FIXED);
        match input {
            Type::Integral(IntegerType {
                range: IntegerRange::ExactLong(_) | IntegerRange::OutsideInt32,
                ..
            }) => {
                self.arithmetic_exception();
                Type::Never
            }
            Type::Integral(IntegerType {
                range: IntegerRange::OutsideInt64,
                ..
            }) => {
                self.illegal_argument();
                Type::Never
            }
            ty if ty.subtype_of(&Type::Integral(IntegerType::INT32)) => {
                // Conversion changes the JVM number class, not the numeric
                // value. Keep a proven in-range literal when available.
                let exact = ty.exact_integer_value();
                if exact.is_none() && !ty.subtype_of(&fixed) {
                    // BigInteger subclasses can override the methods used by int.
                    self.may_throw();
                }
                op.success(exact.map_or(output, Type::Integer))
            }
            ty => match ty.compatibility(&Type::Integral(IntegerType::ANY)) {
                Compatibility::Mismatch => op.invalid(),
                other => {
                    self.arithmetic_exception();
                    if !ty.subtype_of(&fixed) {
                        self.illegal_argument();
                        self.may_throw();
                    }
                    let result = op.success(output);
                    if other == Compatibility::Proven {
                        result
                    } else {
                        Type::union([result, op.invalid()])
                    }
                }
            },
        }
    }

    pub(super) fn validation_projection(&self, form: &Form) -> Option<&Type> {
        let key = self.model.validation_key(form.keyword()?)?;
        if !self.spec_load_is_known(&key) {
            return None;
        }
        if matches!(
            self.globals.spec_provider(&key).map(std::sync::Arc::as_ref),
            Some(SpecProvider::Imported { .. })
        ) && self
            .globals
            .spec_conversion(&key)
            .as_ref()
            .is_some_and(has_checked_int)
        {
            // int can use virtual BigInteger methods. Success guarantees a
            // converted Integer, not a bound on the original field value.
            return None;
        }
        self.globals.spec(&key).filter(|ty| usable(ty))
    }

    fn spec_load_is_known(&self, key: &str) -> bool {
        match self.globals.spec_provider(key).map(std::sync::Arc::as_ref) {
            Some(provider @ (SpecProvider::LoadChecked { .. } | SpecProvider::Imported { .. })) => {
                self.model.loads_spec(provider)
            }
            _ => true,
        }
    }

    fn is_imported_structure(&self, key: &str) -> bool {
        matches!(
            self.globals.spec_provider(key).map(std::sync::Arc::as_ref),
            Some(SpecProvider::Imported {
                structured: true,
                ..
            })
        )
    }
}

fn has_checked_int(conversion: &SpecConversion) -> bool {
    match conversion {
        SpecConversion::CheckedInt32 => true,
        SpecConversion::StringKeyedMap { values, .. } => has_checked_int(values),
        _ => {
            conversion
                .portable_collection()
                .is_some_and(|(_, element)| has_checked_int(element))
                || conversion.map_fields().is_some_and(|fields| {
                    fields
                        .values()
                        .any(|field| field.conversion.as_ref().is_some_and(has_checked_int))
                })
        }
    }
}

fn convert_enum(input: &Type, entries: &[(Type, Type)], op: SpecOperation) -> Type {
    let mut outcomes = vec![op.invalid()];
    for (key, value) in entries {
        match input.compatibility(key) {
            Compatibility::Proven => return op.success(value.clone()),
            Compatibility::Unproven => outcomes.push(op.success(value.clone())),
            Compatibility::Mismatch
                if matches!(input, Type::Keyword | Type::SpecKeyword)
                    && matches!(key, Type::Key(_)) =>
            {
                // A general keyword does not meet a literal contract, but it
                // can still be one of the keys when the enum runs.
                outcomes.push(op.success(value.clone()));
            }
            Compatibility::Mismatch => {}
        }
    }
    Type::union(outcomes)
}

fn usable(ty: &Type) -> bool {
    match ty {
        Type::Never
        | Type::Any
        | Type::SpecValue
        | Type::SpecTruthy
        | Type::Nil
        | Type::Boolean
        | Type::Bool(_)
        | Type::String
        | Type::Str(_)
        | Type::Number
        | Type::Integral(_)
        | Type::Integer(_)
        | Type::Character
        | Type::Char(_)
        | Type::Keyword
        | Type::SpecKeyword
        | Type::Key(_)
        | Type::Symbol
        | Type::SimpleSymbol
        | Type::QualifiedSymbol => true,
        Type::SpecCollection(element) => usable(element),
        Type::Union(parts) => parts.iter().all(usable),
        Type::Map(map) => {
            map.extra == ExtraFields::Open
                && map
                    .fields
                    .values()
                    .all(|field| field.value_constraint().is_none_or(usable))
        }
        _ => false,
    }
}

fn may_pass_collection(ty: &Type) -> bool {
    fn always_invalid(ty: &Type) -> bool {
        match ty {
            Type::Key(key) => key == SPEC_INVALID,
            Type::Union(items) => items.iter().all(always_invalid),
            _ => false,
        }
    }

    match ty {
        // A homogeneous vector or set may be empty even if its possible
        // element types include the sentinel.
        Type::Vector(_) | Type::Set(_) => true,
        Type::Seq(item) | Type::List(item) => !always_invalid(item),
        Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => {
            !items.iter().any(always_invalid)
        }
        Type::MapEntry(items) => !items.iter().any(always_invalid),
        _ => false,
    }
}

pub(super) fn refine(actual: &Type, projection: &Type) -> Type {
    if actual.subtype_of(projection) || matches!(actual, Type::Parameter(_)) {
        return actual.clone();
    }
    if *projection == Type::SpecTruthy {
        return actual.spec_truthy().unwrap_or(Type::SpecTruthy);
    }
    if *projection == Type::SpecValue {
        return actual.spec_value().unwrap_or(Type::SpecValue);
    }
    if *actual == Type::SpecTruthy {
        return projection
            .spec_truthy()
            .unwrap_or_else(|| projection.clone());
    }
    if *actual == Type::SpecValue {
        return projection
            .spec_value()
            .unwrap_or_else(|| projection.clone());
    }
    if let Some(intersection) = actual.integer_intersection(projection) {
        return intersection;
    }
    match (actual, projection) {
        (Type::Key(key), Type::SpecKeyword) if key == SPEC_INVALID => Type::Never,
        (Type::Union(variants), projection) => {
            Type::union(variants.iter().map(|actual| refine(actual, projection)))
        }
        (Type::Key(_), Type::Key(_)) => Type::Never,
        (Type::Key(_), Type::Union(members))
            if members.iter().all(|ty| matches!(ty, Type::Key(_))) =>
        {
            Type::Never
        }
        (_, Type::Union(members)) if members.iter().all(|ty| matches!(ty, Type::Integer(_))) => {
            Type::union(
                members
                    .iter()
                    .filter(|member| actual.compatibility(member) != Compatibility::Mismatch)
                    .cloned(),
            )
        }
        (Type::Map(_), Type::Union(parts))
            if let [Type::String, map @ Type::Map(_)] | [map @ Type::Map(_), Type::String] =
                parts.as_slice() =>
        {
            refine(actual, map)
        }
        (Type::Map(actual_map), Type::Union(parts))
            if let [Type::SpecCollection(element), map @ Type::Map(_)]
            | [map @ Type::Map(_), Type::SpecCollection(element)] = parts.as_slice()
                && element.excludes_map_entry() =>
        {
            match actual_map.emptiness() {
                Some(true) => actual.clone(),
                Some(false) => refine(actual, map),
                None => Type::union([
                    Type::Map(MapType::dictionary(Type::Never, Type::Never)),
                    refine(actual, map),
                ]),
            }
        }
        (_, Type::Union(_)) if let Some(overlap) = actual.known_intersection(projection) => overlap,
        (Type::Map(actual), Type::Map(projection)) => {
            let mut shape = actual.clone();
            for (key, field) in &projection.fields {
                let constraint = field.value_constraint();
                let value = |current: &Type| {
                    constraint.map_or_else(|| current.clone(), |ty| refine(current, ty))
                };
                let (ty, required) = match actual.slot(key) {
                    MapSlot::Present(current) => (value(current), true),
                    MapSlot::Optional(current) => (value(current), field.required),
                    MapSlot::Absent if field.required => return Type::Never,
                    MapSlot::Absent => continue,
                    MapSlot::Undeclared => (field.ty.clone(), field.required),
                };
                if required && ty == Type::Never {
                    return Type::Never;
                }
                let field = if constraint.is_none() && ty == Type::Unknown {
                    Field::presence_only(required)
                } else {
                    Field::new(ty, required)
                };
                shape.set_field(key.clone(), field);
            }
            Type::Map(shape)
        }
        // The validated value is in the projection, even when the type system
        // cannot represent its intersection with the previous type.
        (_, projection) => projection.clone(),
    }
}
