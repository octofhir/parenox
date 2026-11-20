//! Infer input from an argument or a supported projection, then output from one callback.
use std::borrow::Cow;

use super::{CallArguments, inline::CallbackReturn};
use crate::Diagnostic;
use crate::checker::{Checker, Env, Value};
use crate::reader::{Form, Span};
use crate::types::{
    CallbackWitness, Compatibility, Parameter, Parameters, Signature, Type, TypeArguments,
};

fn output_evidence<'a>(
    witness: &CallbackWitness<'_>,
    callback: &'a Type,
    parameters: &Parameters,
    evidence: &TypeArguments<'_>,
) -> Result<Cow<'a, Type>, Compatibility> {
    let expected = witness.signature;
    let selected = callback
        .signature_at(expected.arity())
        .ok_or(Compatibility::Unproven)?;
    if selected.parameters.is_some() {
        // Only a generic named callback needs its input context substituted.
        let context = expected.with_type_arguments(parameters, evidence, None);
        let selected = selected.specialize_callback(&context)?;
        witness
            .evidence(&selected.ret)
            .map(|ty| Cow::Owned(ty.into_owned()))
            .ok_or(Compatibility::Unproven)
    } else {
        witness
            .evidence(&selected.ret)
            .ok_or(Compatibility::Unproven)
    }
}

impl Checker<'_> {
    pub(super) fn generic_arguments(
        &mut self,
        function: &Value,
        signature: &Signature,
        forms: &[Form],
        call: Span,
        env: &Env,
    ) -> Vec<(Value, Span)> {
        let mut values = Vec::with_capacity(forms.len());
        let mut callbacks = Vec::new();
        let mut reachable = env.reachable();
        for (index, (form, expected)) in forms.iter().zip(&signature.args).enumerate() {
            if matches!(expected, Type::Function(_)) && self.inline_callback_literal(form, env) {
                // Delay the static analysis of the body, not the evaluation of
                // an argument expression. A literal fn or #() only creates a closure.
                callbacks.push((index, reachable));
                values.push((
                    Value::new(if reachable {
                        Type::Unknown
                    } else {
                        Type::Never
                    }),
                    form.span,
                ));
            } else {
                let value = self.infer_path(form, env, reachable, false);
                reachable = value.known != Type::Never;
                values.push((value, form.span));
            }
        }
        if !callbacks.is_empty() {
            let binders = signature.parameters.as_ref().expect("generic signature");
            let output_callback = binders
                .output()
                .and_then(|(_, parameter)| signature.output_callback(parameter))
                .map(|witness| witness.index);
            let (missing, instantiated) = {
                let inputs = signature.input_types(binders, |index| &values[index].0.ty);
                let missing = inputs.missing();
                let instantiated =
                    signature.with_type_arguments(binders, &inputs, Some(&Type::Unknown));
                (missing, instantiated)
            };
            for (index, reachable_at_argument) in callbacks {
                let form = &forms[index];
                let template = &signature.args[index];
                let expected = &instantiated.args[index];
                // A missing argument stays Unknown. Known context lets us
                // check another input without pretending the call was proved.
                let has_context = !binders.needs_missing_input(template, &missing)
                    || binders.inputs().any(|(index, param)| {
                        !missing[index] && template.mentions_parameter(param)
                    });
                let check_body = reachable && reachable_at_argument && has_context;
                let mode = if Some(index) == output_callback {
                    CallbackReturn::Infer
                } else {
                    CallbackReturn::Check
                };
                let body_checked = self.check_path(check_body, |checker| {
                    checker
                        .contextual_callback(
                            form,
                            expected,
                            call,
                            function.contract.as_deref(),
                            env,
                            mode,
                        )
                        .unwrap_or_else(|| checker.infer(form, env))
                });
                if check_body {
                    values[index].0 = body_checked;
                }
                // A skipped or unproved body does not make closure creation
                // diverge. Keep the placeholder for the argument's reachability.
            }
        }
        values
    }

    pub(super) fn call_generic(
        &mut self,
        signature: &Signature,
        function: &Value,
        arguments: CallArguments<'_>,
    ) -> Value {
        let args = arguments.values;
        let binders = signature.parameters.as_ref().expect("generic signature");
        let mut evidence = signature.input_types(binders, |index| &args[index].0.ty);
        let missing = evidence.missing();
        let mut valid = true;
        for (index, parameter) in binders.inputs() {
            let ty = evidence.get(index);
            if ty.is_some() && parameter.upper.is_none() {
                continue;
            }
            let witness = signature
                .input_witness(parameter)
                .expect("declared input argument");
            let span = args[witness.index].1;
            if let Some(ty) = ty {
                valid &= self.generic_bound(function, parameter, ty, span);
            } else {
                self.missing_parameter(function, parameter, span, &witness.evidence_site());
                valid = false;
            }
        }
        if let Some((index, parameter)) = binders.output() {
            let witness = signature
                .output_callback(parameter)
                .expect("declared result callback");
            let (value, span) = &args[witness.index];
            // A missing input can prevent contextual checking. In that case
            // do not report the consequence as a second missing parameter.
            let mut output_valid = false;
            if !witness
                .signature
                .args
                .iter()
                .any(|ty| binders.needs_missing_input(ty, &missing))
            {
                match output_evidence(&witness, &value.ty, binders, &evidence) {
                    Ok(candidate) => {
                        output_valid = self.generic_bound(function, parameter, &candidate, *span);
                        evidence.insert(index, candidate);
                    }
                    Err(Compatibility::Mismatch) => {} // The argument check below reports it.
                    Err(_) => {
                        self.missing_parameter(function, parameter, *span, "this callback's result")
                    }
                }
            }
            valid &= output_valid;
        }
        // Unknown stands in for a missing binder only while checking the
        // remaining arguments. It cannot hide an independent mismatch.
        let instantiated = signature.with_type_arguments(binders, &evidence, Some(&Type::Unknown));
        self.check_call_arguments(&instantiated, function, arguments);
        Value::new(if valid {
            instantiated.ret
        } else {
            Type::Unknown
        })
    }

    fn generic_bound(
        &mut self,
        function: &Value,
        parameter: &Parameter,
        actual: &Type,
        span: Span,
    ) -> bool {
        let Some(upper) = &parameter.upper else {
            return true;
        };
        match parameter.check_bound(actual) {
            Compatibility::Proven => true,
            Compatibility::Mismatch => {
                self.expect_contract(
                    &Value::new(actual.clone()),
                    upper,
                    span,
                    &format!("Bound for type parameter {}", parameter.name),
                    function.contract.as_deref(),
                );
                false
            }
            Compatibility::Unproven => {
                self.missing_parameter(
                    function,
                    parameter,
                    span,
                    "a type that satisfies its bound",
                );
                false
            }
        }
    }

    fn missing_parameter(
        &mut self,
        function: &Value,
        parameter: &Parameter,
        span: Span,
        site: &str,
    ) {
        let mut diagnostic = Diagnostic::new(
            "PNX9010",
            span,
            format!("Cannot infer type parameter {} from {site}", parameter.name),
        );
        diagnostic.contract = function.contract.as_deref().cloned();
        self.diagnostics.push(diagnostic);
    }
}
