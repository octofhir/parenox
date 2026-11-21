//! Contextual calls over collections that produce eager Clojure vectors.
use super::super::{
    Checker, Env, LiteralContainer, Value, flow::narrow_by_type, literal_vector_value, nil_part,
};
use super::{
    ArgumentForms, Callable, PreparedCallable, call_returns, narrow_emptiness, predicate_type,
    sequences::{collection_emptiness, iteration_may_throw},
};
use crate::reader::{Form, Span};
use crate::types::{IntegerClass, IntegerType, Type};

const MAX_VECTOR_TRANSFORM_STEPS: usize = 128;

struct FixedVector<'a> {
    items: &'a [Type],
    notes: &'a [String],
    container: &'a LiteralContainer,
    span: Span,
    literal: Option<&'a [Form]>,
}

pub(in crate::checker) fn vector_predicate(input: &Type) -> Type {
    match input {
        Type::Never => Type::Never,
        Type::Vector(_) | Type::Tuple(_) | Type::MapEntry(_) => Type::Bool(true),
        Type::Nil
        | Type::Bool(_)
        | Type::Boolean
        | Type::String
        | Type::Str(_)
        | Type::Character
        | Type::Char(_)
        | Type::Keyword
        | Type::SpecKeyword
        | Type::Key(_)
        | Type::Symbol
        | Type::SimpleSymbol
        | Type::QualifiedSymbol
        | Type::Sym(_)
        | Type::Pattern
        | Type::Reduced(_)
        | Type::ListTuple(_)
        | Type::SeqTuple(_)
        | Type::Integer(_)
        | Type::Integral(IntegerType {
            class: IntegerClass::Fixed,
            ..
        }) => Type::Bool(false),
        Type::Union(parts) => Type::union(parts.iter().map(vector_predicate)),
        Type::Parameter(parameter) => parameter
            .upper
            .as_ref()
            .map_or(Type::Boolean, vector_predicate),
        // These wider domains can contain a user-defined IPersistentVector.
        _ => Type::Boolean,
    }
}

pub(in crate::checker) fn narrow_vector_predicate(input: &Type, truth: bool) -> Type {
    if let Type::Union(parts) = input {
        return Type::union(
            parts
                .iter()
                .map(|part| narrow_vector_predicate(part, truth)),
        );
    }
    match vector_predicate(input) {
        Type::Bool(result) if result != truth => Type::Never,
        Type::Boolean if truth && !matches!(input, Type::Parameter(_)) => {
            Type::Vector(Box::new(Type::Any))
        }
        _ => input.clone(),
    }
}

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
        if self
            .model
            .can_use_standard_core(xs[0].symbol().unwrap(), self.globals)
        {
            let start = self.diagnostics.len();
            if let Some(result) = self.transform_fixed_vectors(
                &callable,
                &inputs,
                collections,
                filter.then_some(callback),
                span,
                env,
            ) {
                self.explain_projected_arguments(start, collections);
                self.dedup_diagnostics(start);
                return result;
            }
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
        self.explain_projected_arguments(start, collections);
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

    fn transform_fixed_vectors(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        inputs: &[(Value, Span)],
        collections: &[Form],
        predicate: Option<&Form>,
        span: Span,
        env: &Env,
    ) -> Option<Value> {
        let vectors = inputs
            .iter()
            .zip(collections)
            .map(|((input, _), form)| {
                // s/tuple observes count and nth. A custom vector can expose
                // different elements via seq or IReduceInit.
                let (Type::Tuple(items), Some(container @ LiteralContainer::Vector(_))) =
                    (&input.known, &input.literal_container)
                else {
                    return None;
                };
                Some(FixedVector {
                    items,
                    notes: &input.notes,
                    container,
                    span: form.span,
                    literal: form.vector().filter(|forms| forms.len() == items.len()),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        // All operands are already evaluated. A zip of these finite vectors
        // visits one position at a time and ends at the shortest input.
        let count = vectors
            .iter()
            .map(|vector| vector.items.len())
            .min()
            .unwrap();
        if !self.path_reachable {
            return Some(self.call_prepared(callable, Vec::new(), span, env, false));
        }
        if count == 0 {
            self.call_prepared(callable, Vec::new(), span, env, false);
        }
        let check_arguments = matches!(&callable.kind, Callable::Value(value)
            if matches!(value.known, Type::Function(_) | Type::Union(_)))
            && vectors.iter().any(|vector| vector.literal.is_some());
        let mut projected = check_arguments.then(|| Vec::with_capacity(vectors.len()));
        let mut output = Vec::with_capacity(count);
        let mut exact_positions = true;
        for index in 0..count {
            if self.vector_transform_steps >= MAX_VECTOR_TRANSFORM_STEPS {
                self.type_query = None;
                self.call_prepared(callable, Vec::new(), span, env, false);
                let name = if predicate.is_some() {
                    "filterv"
                } else {
                    "mapv"
                };
                return Some(self.unchecked_form(
                    span,
                    &format!("{name} step analysis limit ({MAX_VECTOR_TRANSFORM_STEPS}) exceeded"),
                ));
            }
            self.vector_transform_steps += 1;
            if let Some(projected) = &mut projected {
                projected.clear();
            }
            let args: Vec<_> = vectors
                .iter()
                .map(|vector| {
                    let mut value =
                        Value::with_notes(vector.items[index].clone(), vector.notes.to_vec());
                    value.literal_container = vector.container.vector_item(index).cloned();
                    let form = vector.literal.map(|forms| &forms[index]);
                    if let Some(projected) = &mut projected {
                        projected.push(form);
                    }
                    let span = form.map_or(vector.span, |form| form.span);
                    (value, span)
                })
                .collect();
            let selection = predicate.map(|form| (form, args[0].0.clone()));
            let forms = projected
                .as_deref()
                .map_or(ArgumentForms::Unavailable, ArgumentForms::Projected);
            let result = self.call_prepared_with_forms(callable, args, forms, span, env, true);
            if result.known == Type::Never {
                return Some(result);
            }
            if let Some((form, mut item)) = selection {
                if result.known.truthy() == Type::Never {
                    continue;
                }
                exact_positions &= result.known.falsey() == Type::Never;
                let kept = self.filtered_element(item.known.clone(), form, env);
                item.narrow(kept);
                output.push(item);
            } else {
                output.push(result);
            }
        }
        let count = output.len();
        Some(if exact_positions {
            literal_vector_value(output.into_iter(), count, true)
        } else {
            let element = Type::union(output.into_iter().map(|item| item.known));
            Value::new(Type::Vector(Box::new(element)))
        })
    }

    fn explain_projected_arguments(&mut self, start: usize, collections: &[Form]) {
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
            || self
                .model
                .declared_value(name, self.globals, callback.span.start.offset)
                .is_some()
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
            "clojure.core/vector?" if !self.globals.project_namespaces.contains("clojure.core") => {
                narrow_vector_predicate(&element, true)
            }
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
        Type::Vector(element) | Type::Set(element) | Type::Seq(element) | Type::List(element) => {
            Ok(*element)
        }
        Type::Parameter(parameter) if parameter.upper.as_ref() == Some(&Type::String) => {
            Ok(Type::Character)
        }
        Type::Nil | Type::Never => Ok(Type::Never),
        Type::Str(string) if string.is_empty() => Ok(Type::Never),
        Type::String | Type::Str(_) => Ok(Type::Character),
        Type::Any | Type::Unknown => Ok(collection),
        Type::SpecCollection(element) => Ok(*element),
        Type::Union(parts) => parts
            .into_iter()
            .map(element_type)
            .collect::<Result<Vec<_>, _>>()
            .map(Type::union),
        Type::Map(map) => map.into_entry_type().ok_or(CollectionError::OpenMap),
        Type::MapEntry(items) => Ok(Type::union(*items)),
        Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => {
            Ok(Type::union(items.into_vec()))
        }
        _ => Err(CollectionError::NotSeqable),
    }
}
