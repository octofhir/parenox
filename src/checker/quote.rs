//! Clojure's quoted literals keep symbols and lists as data.
use super::{Checker, Env, Value, collection_element, vector_type};
use crate::reader::{Form, Kind, Span};
use crate::types::{MAX_TUPLE_ITEMS, Type};

impl Checker<'_> {
    pub(super) fn quote_form(&mut self, forms: &[Form], span: Span) -> Value {
        let [target] = forms else {
            self.error("PNX2554", span, "quote expects exactly one form");
            return Value::new(Type::Unknown);
        };
        self.quoted_data(target)
    }

    pub(super) fn quoted_data(&mut self, form: &Form) -> Value {
        let form = form.without_metadata();
        let ty = match &form.kind {
            Kind::Symbol(_) | Kind::AnonymousArgument(_) => Type::Symbol,
            Kind::List(items) => {
                let values = items
                    .iter()
                    .map(|item| self.quoted_data(item).known)
                    .collect();
                sequence_type(values)
            }
            Kind::Vector(items) => vector_type(
                items
                    .iter()
                    .map(|item| self.quoted_data(item).known)
                    .collect(),
            ),
            Kind::Map(entries) => {
                return self.map_literal(entries, |checker, form| checker.quoted_data(form));
            }
            Kind::Set(items) => Type::Set(collection_element(
                items
                    .iter()
                    .map(|item| self.quoted_data(item).known)
                    .collect(),
            )),
            Kind::Quote(item) => sequence_type(vec![Type::Symbol, self.quoted_data(item).known]),
            Kind::AnonymousFunction(_) => {
                return self.unsupported(
                    form.span,
                    "Quoted anonymous function forms are not supported yet",
                );
            }
            _ => return self.infer(form, &Env::new()),
        };
        Value::new(ty)
    }
}

fn sequence_type(items: Vec<Type>) -> Type {
    if items.len() <= MAX_TUPLE_ITEMS {
        Type::SeqTuple(items.into_boxed_slice())
    } else {
        Type::Seq(collection_element(items))
    }
}
