//! Infer one local occurrence for an editor query.
use super::{Declarations, Globals, Model, contracts::function_parts};
use crate::checker::{Checker, Env, LocalRead, Value, function_forms::function_clauses};
use crate::reader::{Form, Kind};
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
        let published = globals.exports.get(self.namespace())?;
        if !Arc::ptr_eq(published, &model.exports) {
            return None;
        }
        let offset = site.span.start.offset;
        let index = forms.partition_point(|form| form.span.end.offset <= offset);
        self.functions.binary_search(&index).ok()?;
        let form = &forms[index];
        let Kind::List(items) = &form.without_metadata().kind else {
            return None;
        };
        let name = items.get(1)?.symbol()?;
        let mut checker = Checker::new(model, globals, source);
        checker.local_read = Some(LocalRead {
            name: site.symbol()?,
            span: site.span,
            ty: Type::Never,
        });
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
        let ty = checker.local_read?.ty;
        (!matches!(ty, Type::Never | Type::Unknown)).then_some(ty)
    }
}
