//! Body checking for method clauses backed by a local multimethod's function spec.
use super::declarations::SourceDeclaration;
use super::vars::literal;
use super::{Contract, Declarations, Globals, Model};
use crate::checker::{
    Checker, Env, Value,
    function_forms::{function_clauses, split_method_name},
    has_evaluated_metadata, signature_bindings,
};
use crate::reader::{Form, Kind};
use crate::types::{Arity, Type};
use crate::{ContractKind, Diagnostic};
use std::borrow::Cow;
use std::collections::BTreeSet;

#[derive(Default)]
pub(crate) struct CheckedMethodBodies {
    pub(crate) offsets: BTreeSet<usize>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

impl Model {
    pub(super) fn read_multimethod_contract(
        &mut self,
        declarations: &Declarations,
        name: &str,
        parts: &[Form],
        source: &str,
        globals: &Globals,
    ) -> bool {
        let Some((_, local)) = name.split_once('/') else {
            return false;
        };
        let offset = {
            let forms = &self.source_forms;
            let Some(SourceDeclaration::Value([_, target, ..])) =
                declarations.source_declarations(forms, local).next()
            else {
                return false;
            };
            let span = target.without_metadata().span;
            if globals.project_namespaces.contains("clojure.core")
                || !declarations.is_source_multimethod(forms, local, span)
            {
                return false;
            }
            span.start.offset
        };
        if let Some(signatures) = self.parse_fdef(parts, source, globals) {
            let contract = Contract::new(signatures, name, ContractKind::SFdef, &self.path);
            self.multimethod_contracts
                .insert(name.into(), (offset, contract));
        }
        true
    }

    pub(crate) fn check_method_bodies(
        &self,
        declarations: &Declarations,
        source: &str,
        globals: &Globals,
    ) -> CheckedMethodBodies {
        let mut checked = CheckedMethodBodies::default();
        if self.multimethod_contracts.is_empty() || !declarations.all_refers.is_empty() {
            return checked;
        }
        for &index in &declarations.method_forms {
            let form = &self.source_forms[index];
            let Kind::List(items) = &form.without_metadata().kind else {
                continue;
            };
            let [operator, target, dispatch, arms @ ..] = items.as_slice() else {
                continue;
            };
            let Some((defined, contract)) = target
                .symbol()
                .and_then(|name| self.multimethod_contracts.get(&self.function_name(name)))
            else {
                continue;
            };
            if *defined >= form.span.start.offset
                || !literal(dispatch)
                || has_evaluated_metadata(dispatch)
            {
                continue;
            }
            let (local_name, arms) = split_method_name(arms);
            let Ok(clauses) = function_clauses(arms, form.span) else {
                continue;
            };
            let pairs = clauses
                .iter()
                .map(|(arity, clause)| {
                    let mut matching = contract.signatures.iter().filter_map(|signature| {
                        if signature.arity() == *arity {
                            Some(Cow::Borrowed(signature))
                        } else if signature.rest.is_some()
                            && *arity == Arity::Fixed(signature.args.len())
                        {
                            // The fixed clause takes the zero-rest case of this contract.
                            let mut signature = signature.clone();
                            signature.rest = None;
                            Some(Cow::Owned(signature))
                        } else {
                            None
                        }
                    });
                    let signature = matching.next()?;
                    matching.next().is_none().then_some((clause, signature))
                })
                .collect::<Option<Vec<_>>>();
            let Some(pairs) = pairs else { continue };
            let env = method_scope(local_name, || {
                Type::Function(
                    pairs
                        .iter()
                        .map(|(_, signature)| signature.as_ref().clone())
                        .collect(),
                )
            });
            checked.offsets.insert(operator.span.start.offset);
            for (clause, signature) in pairs {
                let mut checker = Checker::new(self, globals, source);
                let fixed = Arity::Fixed(signature.args.len());
                let args = signature_bindings(
                    &signature,
                    clauses.iter().any(|(arity, _)| *arity == fixed),
                );
                let result = checker.function_body(clause, args, &env, None);
                let span = clause.body.last().map_or(clause.span, |form| form.span);
                checker.expect_contract(
                    &result,
                    &signature.ret,
                    span,
                    &format!("Return value of defmethod {}", contract.origin.name),
                    Some(&contract.origin),
                );
                checked.diagnostics.extend(checker.diagnostics);
            }
        }
        checked
    }
}

pub(super) fn method_scope(name: Option<&Form>, own_type: impl FnOnce() -> Type) -> Env {
    let mut env = Env::new();
    if let Some(symbol) = name.and_then(Form::symbol) {
        env.insert(symbol.into(), Value::new(own_type()));
    }
    env
}
