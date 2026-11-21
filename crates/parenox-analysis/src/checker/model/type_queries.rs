//! Analyze the containing function clause for a focused type query.
use super::{
    Declarations, DeclaredValue, Globals, Model,
    partial::{untyped_clauses, uses_standard_partial_calls},
};
use crate::checker::{CheckedCall, Checker, TypeQuery};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

const MAX_UNTYPED_BINDING_STEPS: usize = 2_048;

impl Declarations {
    pub(crate) fn has_external_function(
        &self,
        globals: &Globals,
        namespace: &str,
        name: &str,
    ) -> bool {
        self.names.loads_library(namespace)
            && !globals.project_namespaces.contains(namespace)
            && matches!(
                globals.declared_value(&format!("{namespace}/{name}")),
                Some(DeclaredValue::Function(_))
            )
    }

    pub(crate) fn binding_types(
        &self,
        forms: &[Form],
        functions: &[usize],
        model: &Model,
        globals: &Globals,
        source: &str,
        bindings: impl Iterator<Item = (Span, bool)>,
    ) -> Vec<(Span, Type)> {
        if !globals
            .exports
            .get(self.namespace())
            .is_some_and(|published| Arc::ptr_eq(published, &model.exports))
        {
            return Vec::new();
        }
        let bindings: BTreeMap<_, _> = bindings
            .map(|(span, literal)| (span.start.offset, (span, literal)))
            .collect();
        if bindings.is_empty() {
            return Vec::new();
        }
        let mut types = Vec::new();
        let query_for = |clause: Span| {
            TypeQuery::bindings(
                clause,
                bindings
                    .range(clause.start.offset..clause.end.offset)
                    .map(|(_, span)| *span),
            )
        };
        let untyped_allowed = uses_standard_partial_calls(globals);
        let mut unchecked = None;
        for &index in functions {
            let form = &forms[index];
            let Kind::List(items) = &form.without_metadata().kind else {
                continue;
            };
            let Some(name) = items.get(1).and_then(Form::symbol) else {
                continue;
            };
            let function = model
                .local_function(name)
                .filter(|function| function.span == form.span);
            if let Some(function) = function
                && let Some(contract) = &function.contract
            {
                for signature in contract.signatures.iter() {
                    let clause = function.arities[&signature.arity()].span;
                    let Some(query) = query_for(clause) else {
                        continue;
                    };
                    let mut checker = Checker::new(model, globals, source);
                    checker.type_query = Some(query);
                    checker.declared_function_body(function, signature);
                    types.extend(checker.binding_query_results());
                }
            } else if untyped_allowed {
                let eligible = match function {
                    Some(function) => function.data_body_eligible,
                    None => {
                        model
                            .unchecked_functions
                            .get(&model.function_name(name))
                            .is_some_and(|function| function.arities.is_some())
                            && unchecked
                                .get_or_insert_with(|| model.unchecked_definition_offsets())
                                .contains(&form.span.start.offset)
                    }
                };
                if !eligible {
                    continue;
                }
                let Some(clauses) = untyped_clauses(form, false) else {
                    continue;
                };
                for clause in clauses.values() {
                    if matches!(clause.parameter_vector.kind, Kind::Metadata(..)) {
                        continue;
                    }
                    let Some(query) = query_for(clause.span) else {
                        continue;
                    };
                    let mut checker = Checker::new(model, globals, source);
                    checker.untyped_binding_budget = Some(MAX_UNTYPED_BINDING_STEPS);
                    checker.literal_binding_query = query.includes_literal_bindings();
                    checker.type_query = Some(query);
                    checker.untyped_clause(clause);
                    types.extend(checker.binding_query_results());
                }
            }
        }
        types.sort_by_key(|(span, _)| span.start.offset);
        types
    }

    pub(crate) fn local_type(
        &self,
        forms: &[Form],
        model: &Model,
        globals: &Globals,
        source: &str,
        site: &Form,
    ) -> Option<Type> {
        self.observe_at(forms, model, globals, source, TypeQuery::local(site)?)?
            .into_value_type()
    }

    pub(crate) fn lookup_type(
        &self,
        forms: &[Form],
        model: &Model,
        globals: &Globals,
        source: &str,
        site: &Form,
    ) -> Option<Type> {
        self.observe_at(forms, model, globals, source, TypeQuery::lookup(site))?
            .into_value_type()
    }

    pub(crate) fn completion_types<'a>(
        &'a self,
        forms: &'a [Form],
        model: &'a Model,
        globals: &'a Globals,
        source: &'a str,
        site: Span,
        names: &[&'a str],
    ) -> Option<BTreeMap<String, String>> {
        self.observe_at(
            forms,
            model,
            globals,
            source,
            TypeQuery::completion(site, names),
        )?
        .into_completion_types()
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
        if self.runtime_alias_gap(symbol).is_some() {
            return None;
        }
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
        &'a self,
        forms: &'a [Form],
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
        if self.static_method_scope
            && self.method_forms.binary_search(&index).is_ok()
            && !matches!(query, TypeQuery::Call(_))
            && uses_standard_partial_calls(globals)
        {
            let clauses = untyped_clauses(&forms[index], true)?;
            let clause = clauses.values().find(|clause| {
                (clause.span.start.offset..clause.span.end.offset).contains(&offset)
            })?;
            let mut checker = Checker::new(model, globals, source);
            checker.type_query = Some(query);
            checker.partial_scope = Some(Cow::Owned(self.partial_shadowed(forms)));
            checker.partial_method_body = true;
            checker.untyped_clause(clause);
            return checker.type_query;
        }
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
            let clauses = untyped_clauses(form, false)?;
            let clause = clauses.values().find(|clause| {
                offset >= clause.span.start.offset && offset < clause.span.end.offset
            })?;
            checker.untyped_clause(clause);
        }
        checker.type_query
    }
}

impl Checker<'_> {
    pub(in crate::checker) fn clean_binding_check(&self) -> bool {
        self.diagnostics.is_empty() && !self.has_unproven_expectation
    }

    pub(in crate::checker) fn can_keep_literal_bindings(&self) -> bool {
        self.literal_binding_query
            && self.diagnostics.iter().all(|finding| {
                matches!(
                    finding.code.as_str(),
                    "PNX9002" | "PNX9003" | "PNX9004" | "PNX9009"
                ) || (finding.code == "PNX9001" && finding.severity == "warning")
            })
    }

    fn binding_query_results(self) -> impl Iterator<Item = (Span, Type)> {
        let clean = self.clean_binding_check();
        let keep = clean || self.can_keep_literal_bindings();
        self.type_query
            .filter(|_| keep)
            .into_iter()
            .flat_map(move |query| query.into_binding_types(!clean))
    }
}
