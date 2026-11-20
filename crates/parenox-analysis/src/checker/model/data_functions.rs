//! Body checks for private functions that construct literal data.
use std::{borrow::Cow, collections::BTreeSet, sync::Arc};

use super::{Function, Globals, Model};
use crate::checker::{Checker, Env, Value};
use crate::reader::{Form, Kind};
use crate::types::{Arity, MAX_TUPLE_ITEMS, Parameter, Type};

impl Model {
    pub(super) fn inert_function_metadata(
        &self,
        mut name: &Form,
        attrs: [Option<&Form>; 2],
    ) -> bool {
        while let Kind::Metadata(metadata, target) = &name.kind {
            if !matches!(metadata.kind, Kind::Map(_)) || !self.data_expression(metadata, &[]) {
                return false;
            }
            name = target;
        }
        attrs
            .into_iter()
            .flatten()
            .all(|form| self.data_expression(form, &[]))
    }

    pub(super) fn check_data_functions(&mut self, source: &str, globals: &Globals) {
        self.checked_data_functions = self
            .functions
            .values()
            .filter(|function| self.check_data_function(function, source, globals))
            .map(|function| function.name.clone())
            .collect();
    }

    fn check_data_function(&self, function: &Function, source: &str, globals: &Globals) -> bool {
        if !function.data_body_eligible || function.arities.len() != 1 {
            return false;
        }
        let Some((Arity::Fixed(count), arity)) = function.arities.first_key_value() else {
            return false;
        };
        let [body] = arity.body.as_slice() else {
            return false;
        };
        let mut names = Vec::with_capacity(*count);
        for param in &arity.params {
            let Kind::Symbol(name) = &param.kind else {
                return false;
            };
            if names.contains(&name.as_str()) {
                return false;
            }
            names.push(name.as_str());
        }
        if !self.data_expression(body, &names) {
            return false;
        }

        let args = names
            .into_iter()
            .map(|name| {
                Value::new(Type::Parameter(Arc::new(Parameter {
                    owner: function.name.clone(),
                    name: name.into(),
                    upper: None,
                })))
            })
            .collect();
        let mut checker = Checker::new(self, globals, source);
        let result = checker.function_body(
            &arity.params,
            arity.conditions(),
            &arity.body,
            args,
            &Env::new(),
            arity.span,
        );
        checker.diagnostics.is_empty()
            && result.known != Type::Never
            && !result.known.contains_unknown()
    }

    fn data_expression(&self, form: &Form, params: &[&str]) -> bool {
        match &form.kind {
            Kind::Nil
            | Kind::Bool(_)
            | Kind::Number
            | Kind::SymbolicNumber(_)
            | Kind::Character(_)
            | Kind::String(_)
            | Kind::Utf16String(_) => true,
            Kind::Keyword(key) => self.data_keyword(key).is_some(),
            Kind::Symbol(name) => params.contains(&name.as_str()),
            Kind::Vector(items) => {
                items.len() <= MAX_TUPLE_ITEMS
                    && items.iter().all(|item| self.data_expression(item, params))
            }
            Kind::Map(entries) => {
                let (pairs, remainder) = entries.as_chunks::<2>();
                let mut keys = BTreeSet::new();
                remainder.is_empty()
                    && pairs.iter().all(|[key, value]| {
                        let Kind::Keyword(key) = &key.kind else {
                            return false;
                        };
                        self.data_keyword(key).is_some_and(|key| keys.insert(key))
                            && self.data_expression(value, params)
                    })
            }
            _ => false,
        }
    }

    fn data_keyword<'a>(&self, key: &'a str) -> Option<Cow<'a, str>> {
        if key.starts_with("::") {
            self.validation_key(key).map(Cow::Owned)
        } else {
            Some(Cow::Borrowed(key))
        }
    }
}
