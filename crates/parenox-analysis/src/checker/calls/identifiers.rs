//! Results and exceptions of JVM identifier operations.
use crate::checker::Checker;
use crate::types::{IntegerClass, IntegerType, Type};

impl Checker<'_> {
    pub(in crate::checker) fn name_result(&mut self, input: &Type) -> Type {
        match input {
            Type::Never | Type::Str(_) | Type::String => input.clone(),
            Type::Key(identifier) | Type::Sym(identifier) => {
                let spelling = identifier.strip_prefix(':').unwrap_or(identifier);
                let name = spelling
                    .split_once('/')
                    .filter(|(namespace, _)| !namespace.is_empty())
                    .map_or(spelling, |(_, name)| name);
                Type::Str(name.to_owned())
            }
            Type::Keyword | Type::SpecKeyword => Type::String,
            Type::Symbol | Type::SimpleSymbol | Type::QualifiedSymbol => Type::String.nilable(),
            Type::Nil => {
                self.null_pointer();
                Type::Never
            }
            Type::Boolean
            | Type::Bool(_)
            | Type::Character
            | Type::Char(_)
            | Type::Integer(_)
            | Type::Integral(IntegerType {
                class: IntegerClass::Fixed,
                ..
            }) => {
                self.class_cast();
                Type::Never
            }
            Type::Union(members) => Type::union(members.iter().map(|ty| self.name_result(ty))),
            Type::Parameter(parameter) if let Some(upper) = parameter.upper.as_ref() => {
                self.name_result(upper)
            }
            // Other types may implement Named as well. Its JVM method returns
            // a nullable String and can throw.
            _ => {
                self.may_throw();
                Type::String.nilable()
            }
        }
    }
}
