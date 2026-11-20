//! Clojure's quoted literals keep symbols and lists as data.
use super::{Checker, Env, Value, collection_element, literal_vector_value, maps::MapLiteral};
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
            Kind::Symbol(symbol) if symbol != "/" && symbol.contains('/') => Type::QualifiedSymbol,
            Kind::Symbol(_) | Kind::AnonymousArgument(_) => Type::SimpleSymbol,
            Kind::List(items) => {
                let values = items
                    .iter()
                    .map(|item| self.quoted_data(item).known)
                    .collect();
                sequence_type(values)
            }
            Kind::Vector(items) => {
                return literal_vector_value(
                    items.iter().map(|item| self.quoted_data(item)),
                    items.len(),
                    true,
                );
            }
            Kind::Map(entries) => {
                return self.map_literal(entries, MapLiteral::Quoted);
            }
            Kind::Set(items) => Type::Set(collection_element(
                items
                    .iter()
                    .map(|item| self.quoted_data(item).known)
                    .collect(),
            )),
            Kind::Quote(item) => {
                sequence_type(vec![Type::SimpleSymbol, self.quoted_data(item).known])
            }
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
        Type::ListTuple(items.into_boxed_slice())
    } else {
        Type::List(collection_element(items))
    }
}
