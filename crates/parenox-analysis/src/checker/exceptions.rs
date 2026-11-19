//! Explicit exceptional exits and Clojure's structured exception data.
mod effects;
mod handlers;

pub(super) use effects::Throws;

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
        if let Some(throws) = &mut self.throws {
            throws.throw_value(&value.known);
        }
        Value::new(Type::Never)
    }

    pub(super) fn ex_info(&mut self, args: &[(Value, Span)]) -> Value {
        let (message, message_span) = &args[0];
        let (data, data_span) = &args[1];
        let message_type = Type::String.nilable();
        let data_type = map(ExtraFields::Open).nilable();
        let cause_type = Type::Throwable.nilable();
        if self.throws.is_some()
            && (!message.known.subtype_of(&message_type)
                || !data.known.subtype_of(&data_type)
                || args
                    .get(2)
                    .is_some_and(|(cause, _)| !cause.known.subtype_of(&cause_type)))
        {
            self.may_throw();
        }
        self.expect(message, &message_type, *message_span, "ex-info message");
        self.expect(data, &data_type, *data_span, "ex-info data");
        if let Some((cause, span)) = args.get(2) {
            self.expect(cause, &cause_type, *span, "ex-info cause");
        }
        Value::with_notes(
            Type::ExceptionInfo(Box::new(exception_data(&data.known))),
            data.notes.clone(),
        )
    }

    pub(super) fn ex_data(&mut self, value: &Value) -> Value {
        fn unknown_constructor(ty: &Type) -> bool {
            match ty {
                Type::Throwable | Type::Unknown | Type::Parameter(_) => true,
                Type::Union(parts) => parts.iter().any(unknown_constructor),
                _ => false,
            }
        }
        fn data(ty: &Type) -> Type {
            match ty {
                Type::ExceptionInfo(data) => data.as_ref().clone(),
                Type::Throwable | Type::Parameter(_) => map(ExtraFields::Open).nilable(),
                Type::Unknown | Type::Never => ty.clone(),
                Type::Union(parts) => Type::union(parts.iter().map(data)),
                _ => Type::Nil,
            }
        }
        if self.throws.is_some() && unknown_constructor(&value.known) {
            self.may_throw();
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
