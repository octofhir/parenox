use crate::checker::{Checker, model::GuardedMapProjection};
use crate::types::{IntegerClass, IntegerType, MapSlot, MapType, Type};

use super::SpecOperation;

fn lookup(map: &MapType, key: &Type, default: Type) -> Type {
    match map.slot(key) {
        MapSlot::Present(value) => value.clone(),
        MapSlot::Optional(value) => Type::union([value.clone(), default]),
        MapSlot::Absent => default,
        MapSlot::Undeclared => Type::Any,
    }
}

pub(super) fn conform_literal(
    map: &MapType,
    projection: &GuardedMapProjection,
    operation: SpecOperation,
) -> Type {
    let mut may_skip = false;
    for key in &projection.guards {
        let value = lookup(map, key, Type::Nil);
        if value.truthy() == Type::Never {
            return operation.success(Type::Nil);
        }
        may_skip |= value.falsey() != Type::Never;
    }
    match operation {
        SpecOperation::Valid => Type::Bool(true),
        SpecOperation::Conform => {
            let result = projection.map_result(|key, default| {
                let value = lookup(map, key, default);
                if projection.guards.contains(key) {
                    value.truthy()
                } else {
                    value
                }
            });
            if may_skip { result.nilable() } else { result }
        }
    }
}

impl Checker<'_> {
    pub(super) fn conform_opaque_projection(
        &mut self,
        input: &Type,
        projection: &GuardedMapProjection,
        operation: SpecOperation,
    ) -> Type {
        if has_no_keyword_lookup(input) {
            return operation.success(Type::Nil);
        }
        // Java Map, sorted maps and ILookup may throw or change their answers
        // between guard and body. A shape alone cannot establish stability.
        self.may_throw();
        match operation {
            SpecOperation::Valid => Type::Bool(true),
            SpecOperation::Conform => projection.map_result(|_, _| Type::Any).nilable(),
        }
    }
}

fn has_no_keyword_lookup(input: &Type) -> bool {
    matches!(
        input,
        Type::Nil
            | Type::Bool(_)
            | Type::Boolean
            | Type::Char(_)
            | Type::Character
            | Type::Str(_)
            | Type::String
            | Type::Key(_)
            | Type::Keyword
            | Type::SpecKeyword
            | Type::Symbol
            | Type::SimpleSymbol
            | Type::QualifiedSymbol
            | Type::Integer(_)
            | Type::Integral(IntegerType {
                class: IntegerClass::Fixed,
                ..
            })
            | Type::Pattern
    )
}
