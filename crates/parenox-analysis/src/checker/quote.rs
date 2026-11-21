//! Clojure's quoted literals keep symbols and lists as data.
use super::{Checker, Env, Value, collection_element, literal_vector_value, maps::MapLiteral};
use crate::reader::{Form, Kind, Span};
use crate::types::{MAX_TUPLE_ITEMS, Type};

pub(super) fn quoted_symbol(form: &Form) -> Option<&str> {
    let target = match &form.without_metadata().kind {
        Kind::Quote(target) => target.as_ref(),
        Kind::List(forms) => match forms.as_slice() {
            [head, target] if head.symbol() == Some("quote") => target,
            _ => return None,
        },
        _ => return None,
    };
    match &target.without_metadata().kind {
        Kind::Symbol(symbol) => Some(symbol),
        _ => None,
    }
}

impl Checker<'_> {
    pub(super) fn quote_form(&mut self, forms: &[Form], span: Span) -> Value {
        let [target] = forms else {
            self.error("PNX2554", span, "quote expects exactly one form");
            return Value::new(Type::Unknown);
        };
        self.quoted_data(target)
    }

    pub(super) fn quoted_data(&mut self, form: &Form) -> Value {
        self.quoted_data_with_symbols(form, true)
    }

    pub(super) fn quoted_data_with_symbols(&mut self, form: &Form, precise_symbols: bool) -> Value {
        let form = form.without_metadata();
        let ty = match &form.kind {
            Kind::Symbol(symbol) if precise_symbols => Type::Sym(symbol.clone()),
            Kind::Symbol(symbol) if symbol != "/" && symbol.contains('/') => Type::QualifiedSymbol,
            // The JVM reader generates a fresh symbol even in quoted #() data.
            Kind::Symbol(_) | Kind::AnonymousArgument(_) => Type::SimpleSymbol,
            Kind::List(items) => {
                let precise_symbols = precise_symbols && items.len() <= MAX_TUPLE_ITEMS;
                let values = items
                    .iter()
                    .map(|item| self.quoted_data_with_symbols(item, precise_symbols).known)
                    .collect();
                sequence_type(values)
            }
            Kind::Vector(items) => {
                let precise_symbols = precise_symbols && items.len() <= MAX_TUPLE_ITEMS;
                return literal_vector_value(
                    items
                        .iter()
                        .map(|item| self.quoted_data_with_symbols(item, precise_symbols)),
                    items.len(),
                    true,
                );
            }
            Kind::Map(entries) => {
                let precise_symbols = precise_symbols && entries.len() / 2 <= MAX_TUPLE_ITEMS;
                return self.map_literal(entries, MapLiteral::Quoted { precise_symbols });
            }
            Kind::Set(items) => {
                // Large inventories use a shared element type. Keeping a
                // separate shape for every symbol also duplicates nested maps.
                let precise_symbols = precise_symbols && items.len() <= MAX_TUPLE_ITEMS;
                Type::Set(collection_element(
                    items
                        .iter()
                        .map(|item| self.quoted_data_with_symbols(item, precise_symbols).known)
                        .collect(),
                ))
            }
            Kind::Quote(item) => {
                let quote = if precise_symbols {
                    Type::Sym("quote".into())
                } else {
                    Type::SimpleSymbol
                };
                sequence_type(vec![
                    quote,
                    self.quoted_data_with_symbols(item, precise_symbols).known,
                ])
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
