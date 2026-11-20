use crate::types::{ExtraFields, Field, MapType, Type};

/// A guarded map literal whose leaves read fields from a single argument.
/// Preserve source order for inputs with custom lookups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GuardedMapProjection {
    pub guards: Box<[Type]>,
    pub fields: ProjectedFields,
}

pub(crate) type ProjectedFields = Box<[(Type, ProjectedValue)]>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectedValue {
    Lookup(Type),
    BooleanLookup(Type),
    Map(ProjectedFields),
}

impl GuardedMapProjection {
    pub fn map_result(&self, mut lookup: impl FnMut(&Type, Type) -> Type) -> Type {
        map_result(&self.fields, &mut lookup)
    }
}

fn map_result(fields: &ProjectedFields, lookup: &mut impl FnMut(&Type, Type) -> Type) -> Type {
    Type::Map(MapType {
        fields: fields
            .iter()
            .map(|(key, value)| {
                let ty = match value {
                    ProjectedValue::Lookup(key) => lookup(key, Type::Nil),
                    ProjectedValue::BooleanLookup(key) => {
                        let value = lookup(key, Type::Bool(false));
                        if value.truthy() == Type::Never {
                            Type::Bool(false)
                        } else if value.falsey() == Type::Never {
                            Type::Bool(true)
                        } else {
                            Type::Boolean
                        }
                    }
                    ProjectedValue::Map(fields) => map_result(fields, lookup),
                };
                (key.clone(), Field::new(ty, true))
            })
            .collect(),
        extra: ExtraFields::Closed,
    })
}
