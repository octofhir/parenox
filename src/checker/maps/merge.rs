//! Right-biased map merging, including optional and dictionary entries.
use super::{Checker, Value};
use crate::reader::Span;
use crate::types::Type;

impl Checker<'_> {
    pub(in crate::checker) fn merge(&mut self, args: &[(Value, Span)], span: Span) -> Value {
        let mut result = Type::Nil;
        for (value, arg_span) in args {
            let right = alternatives(&value.known);
            if right.contains(&Type::Unknown) {
                return self.unsupported(*arg_span, "Cannot determine the map type for merge");
            }
            if !right
                .iter()
                .all(|ty| matches!(ty, Type::Nil | Type::Map(_)))
            {
                return self.unsupported(
                    *arg_span,
                    format!(
                        "merge currently supports only map or Nil operands, got {}",
                        value.ty
                    ),
                );
            }
            let left = alternatives(&result);
            if left.len().saturating_mul(right.len()) > 64 {
                return self.unsupported(span, "Too many map alternatives to analyze merge");
            }
            result = Type::union(left.iter().flat_map(|a| {
                right.iter().map(move |b| match (a, b) {
                    (Type::Map(a), Type::Map(b)) => Type::Map(a.merge(b)),
                    (a, Type::Nil) => a.clone(),
                    (Type::Nil, b) => b.clone(),
                    _ => unreachable!("merge operands were checked"),
                })
            }));
        }
        for ty in alternatives(&result) {
            if let Type::Map(map) = ty
                && let Some((key, _)) = map
                    .fields
                    .iter()
                    .find(|(_, field)| field.ty == Type::Unknown)
            {
                self.error(
                    "PNX9003",
                    span,
                    format!("Cannot determine the type of {key} after merge"),
                );
                break;
            }
        }
        Value::new(result)
    }
}

fn alternatives(ty: &Type) -> &[Type] {
    match ty {
        Type::Union(parts) => parts,
        ty => std::slice::from_ref(ty),
    }
}
