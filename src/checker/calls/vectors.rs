//! Contextual calls over collections that produce eager Clojure vectors.
use super::super::{Checker, Env, Value, flow::narrow_by_type, nil_part};
use super::{
    call_returns, narrow_emptiness, predicate_type,
    sequences::{collection_emptiness, iteration_may_throw},
};
use crate::reader::{Form, Span};
use crate::types::Type;

impl Checker<'_> {
    pub(super) fn vector_transform(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        name: &str,
    ) -> Value {
        let filter = name == "clojure.core/filterv";
        if xs.len() < 3 || (filter && xs.len() != 3) {
            let inputs = if filter {
                "one collection"
            } else {
                "at least one collection"
            };
            self.error(
                "PNX2554",
                span,
                format!("{name} expects a callback and {inputs}"),
            );
            return Value::new(Type::Unknown);
        }

        let callback = &xs[1];
        let collections = &xs[2..];
        let callable = self.prepare_callable(callback, env);
        let inputs: Vec<_> = self
            .eval_args(collections, env, callable.returns())
            .collect();
        if !call_returns(&inputs) {
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return Value::new(Type::Never);
        }
        let nonempty = inputs
            .iter()
            .all(|(value, _)| collection_emptiness(&value.known) == Some(Type::Bool(false)));
        let args: Vec<_> = inputs
            .into_iter()
            .map(|(input, span)| (self.collection_element(input, span, name), span))
            .collect();
        let invoked = args.iter().all(|(element, _)| element.known != Type::Never);
        let filtered = filter.then(|| args[0].0.known.clone());
        let start = self.diagnostics.len();
        let result = self.call_prepared(&callable, args, span, env, invoked);
        for diagnostic in &mut self.diagnostics[start..] {
            if diagnostic.code == "PNX2322"
                && collections.iter().any(|form| diagnostic.span == form.span)
            {
                diagnostic.suggestions.clear();
                diagnostic.help = Some(
                    "Use a callback that accepts these elements or convert the collection's elements before this call."
                        .into(),
                );
            }
        }
        if invoked && nonempty && result.known == Type::Never {
            return result;
        }
        let element = match filtered {
            _ if !invoked => Type::Never,
            Some(_) if result.known.truthy() == Type::Never => Type::Never,
            Some(element) => self.filtered_element(element, callback, env),
            None => result.known,
        };
        Value::new(Type::Vector(Box::new(element)))
    }

    fn collection_element(&mut self, collection: Value, span: Span, name: &str) -> Value {
        if iteration_may_throw(&collection.known) {
            self.may_throw();
        }
        match element_type(collection.known) {
            Ok(element) => Value::with_notes(element, collection.notes),
            Err(CollectionError::OpenMap) => self.unsupported(
                span,
                format!("{name} cannot infer entry types for an open map"),
            ),
            Err(CollectionError::NotSeqable) => {
                self.illegal_argument();
                self.error(
                    "PNX2322",
                    span,
                    format!("{name} expects a seqable collection, got {}", collection.ty),
                );
                Value::new(Type::Unknown)
            }
        }
    }

    fn filtered_element(&self, element: Type, callback: &Form, env: &Env) -> Type {
        let Some(name) = callback.symbol() else {
            return element;
        };
        if env.contains_key(name)
            || self.model.local_function(name).is_some()
            || self.model.declared_value(name, self.globals).is_some()
        {
            return element;
        }
        let name = self.model.resolve(name);
        match name.as_str() {
            "clojure.core/some?" => element.without_nil(),
            "clojure.core/nil?" => nil_part(&element),
            "clojure.core/empty?" => narrow_emptiness(&element, true),
            "clojure.core/not-empty" => narrow_emptiness(&element, false),
            "clojure.core/identity" => element.truthy(),
            _ => match predicate_type(&name) {
                Some(wanted) => narrow_by_type(&element, &wanted, true),
                None => element,
            },
        }
    }
}

pub(super) enum CollectionError {
    OpenMap,
    NotSeqable,
}

pub(super) fn element_type(collection: Type) -> Result<Type, CollectionError> {
    match collection {
        Type::Vector(element) | Type::Set(element) | Type::Seq(element) => Ok(*element),
        Type::Nil | Type::Never => Ok(Type::Never),
        Type::Str(string) if string.is_empty() => Ok(Type::Never),
        Type::String | Type::Str(_) => Ok(Type::Character),
        Type::Unknown => Ok(Type::Unknown),
        Type::Union(parts) => parts
            .into_iter()
            .map(element_type)
            .collect::<Result<Vec<_>, _>>()
            .map(Type::union),
        Type::Map(map) => map.into_entry_type().ok_or(CollectionError::OpenMap),
        Type::MapEntry(items) => Ok(Type::union(*items)),
        Type::Tuple(items) | Type::SeqTuple(items) => Ok(Type::union(items.into_vec())),
        _ => Err(CollectionError::NotSeqable),
    }
}
