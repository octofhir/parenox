//! Analyze the containing function clause for a focused type query.
use super::{Declarations, DeclaredValue, Globals, Model, contracts::function_parts};
use crate::checker::{
    CheckedCall, Checker, Env, TypeQuery, Value, function_forms::function_clauses,
};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;
use std::sync::Arc;

impl Declarations {
    pub(crate) fn local_read_type(
        &self,
        forms: &[Form],
        model: &Model,
        globals: &Globals,
        source: &str,
        site: &Form,
    ) -> Option<Type> {
        self.observe_at(forms, model, globals, source, TypeQuery::local(site)?)?
            .into_local_type()
    }

    pub(crate) fn checked_call(
        &self,
        forms: &[Form],
        model: &Model,
        globals: &Globals,
        source: &str,
        span: Span,
        written: &[Form],
    ) -> Option<CheckedCall> {
        self.observe_at(
            forms,
            model,
            globals,
            source,
            TypeQuery::call(span, written),
        )?
        .into_call()
    }

    pub(crate) fn qualified_call_declaration(
        &self,
        globals: &Globals,
        symbol: &str,
        wanted: usize,
    ) -> Option<CheckedCall> {
        let (namespace, name) = self.qualified_reference(symbol)?;
        let DeclaredValue::Function(contract) =
            globals.declared_value(&format!("{namespace}/{name}"))?
        else {
            return None;
        };
        let signature = contract
            .signatures
            .iter()
            .find(|signature| signature.arity().accepts(wanted))
            .or_else(|| {
                contract
                    .signatures
                    .iter()
                    .find(|sig| sig.arity().required() >= wanted)
            })
            .or_else(|| contract.signatures.last())?;
        Some(CheckedCall::declaration(signature, &contract.origin))
    }

    fn observe_at<'a>(
        &self,
        forms: &[Form],
        model: &'a Model,
        globals: &'a Globals,
        source: &'a str,
        query: TypeQuery<'a>,
    ) -> Option<TypeQuery<'a>> {
        let published = globals.exports.get(self.namespace())?;
        if !Arc::ptr_eq(published, &model.exports) {
            return None;
        }
        let offset = query.span().start.offset;
        let index = forms.partition_point(|form| form.span.end.offset <= offset);
        self.functions.binary_search(&index).ok()?;
        let form = &forms[index];
        let Kind::List(items) = &form.without_metadata().kind else {
            return None;
        };
        let name = items.get(1)?.symbol()?;
        let mut checker = Checker::new(model, globals, source);
        checker.type_query = Some(query);
        if let Some(function) = model.local_function(name)
            && function.span == form.span
            && let Some(contract) = &function.contract
        {
            let signature = contract.signatures.iter().find(|signature| {
                let span = function.arities[&signature.arity()].span;
                offset >= span.start.offset && offset < span.end.offset
            })?;
            checker.declared_function_body(function, signature);
        } else {
            let clauses = function_clauses(function_parts(&items[2..]).0, form.span).ok()?;
            let clause = clauses.values().find(|clause| {
                offset >= clause.span.start.offset && offset < clause.span.end.offset
            })?;
            let params = clause.bindings();
            let args = vec![Value::new(Type::Unknown); params.len()];
            checker.function_body(
                &params,
                clause.conditions,
                clause.body,
                args,
                &Env::new(),
                clause.span,
            );
        }
        checker.type_query
    }
}
