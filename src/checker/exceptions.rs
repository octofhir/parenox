//! Explicit exceptional exits and Clojure's structured exception data.
use std::collections::BTreeMap;

use super::{Checker, Env, Value};
use crate::reader::{Form, Span};
use crate::types::{ExtraFields, MapType, Type};

impl Checker<'_> {
    pub(super) fn throw_form(&mut self, args: &[Form], span: Span, env: &Env) -> Value {
        let [exception] = args else {
            self.error("PNX2554", span, "throw requires one argument");
            return Value::new(Type::Unknown);
        };
        let value = self.infer(exception, env);
        self.expect(&value, &Type::Throwable.nilable(), exception.span, "throw");
        Value::new(Type::Never)
    }

    pub(super) fn ex_info(&mut self, args: &[(Value, Span)]) -> Value {
        let (message, message_span) = &args[0];
        let (data, data_span) = &args[1];
        self.expect(
            message,
            &Type::String.nilable(),
            *message_span,
            "ex-info message",
        );
        self.expect(
            data,
            &map(ExtraFields::Open).nilable(),
            *data_span,
            "ex-info data",
        );
        if let Some((cause, span)) = args.get(2) {
            self.expect(cause, &Type::Throwable.nilable(), *span, "ex-info cause");
        }
        Value::with_notes(
            Type::ExceptionInfo(Box::new(exception_data(&data.known))),
            data.notes.clone(),
        )
    }

    pub(super) fn ex_data(&self, value: &Value) -> Value {
        fn data(ty: &Type) -> Type {
            match ty {
                Type::ExceptionInfo(data) => data.as_ref().clone(),
                Type::Throwable => map(ExtraFields::Open).nilable(),
                Type::Unknown | Type::Never => ty.clone(),
                Type::Union(parts) => Type::union(parts.iter().map(data)),
                _ => Type::Nil,
            }
        }
        Value::with_notes(data(&value.known), value.notes.clone())
    }
}

fn map(extra: ExtraFields) -> Type {
    Type::Map(MapType {
        fields: BTreeMap::new(),
        extra,
    })
}

fn exception_data(ty: &Type) -> Type {
    match ty {
        Type::Nil => map(ExtraFields::Closed),
        Type::Union(parts) => Type::union(parts.iter().map(exception_data)),
        other => other.clone(),
    }
}
