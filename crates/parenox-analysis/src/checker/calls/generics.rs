//! Infer input from an argument or a supported projection, then output from one callback.
use std::borrow::Cow;

use super::{CallArguments, EvaluatedArguments, inline::CallbackReturn};
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
    if selected.parameters.is_some() || selected.nil_preserving {
        let context = expected.with_type_arguments(parameters, evidence, None);
        let specialized = if selected.parameters.is_some() {
            Some(selected.specialize_callback(&context)?)
        } else {
            context
                .args
                .first()
                .and_then(|input| selected.specialize_nil_result(input))
        };
        if let Some(specialized) = specialized {
            return witness
                .evidence(&specialized.ret)
                .map(|ty| Cow::Owned(ty.into_owned()))
                .ok_or(Compatibility::Unproven);
        }
    }
    witness
        .evidence(&selected.ret)
        .ok_or(Compatibility::Unproven)
}

impl Checker<'_> {
    pub(super) fn generic_arguments(
        &mut self,
        function: &Value,
        signature: &Signature,
        forms: &[Form],
        call: Span,
        env: &Env,
    ) -> EvaluatedArguments {
        let mut values = Vec::with_capacity(forms.len());
        let mut callbacks = Vec::new();
        let mut reachable = env.reachable();
        for (index, (form, expected)) in forms.iter().zip(&signature.args).enumerate() {
            if matches!(expected, Type::Function(_)) && self.contextual_callback_form(form, env) {
                // A literal fn creates a closure; a saved local only reads one.
                // The contextual body can wait for the other arguments' types.
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
        let mut output_gap_explained = false;
        if !callbacks.is_empty() {
            let binders = signature.parameters.as_ref().expect("generic signature");
            let output_callback = binders.output().and_then(|(_, parameter)| {
                Some((parameter, signature.output_callback(parameter)?))
            });
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
                let output = output_callback
                    .as_ref()
                    .filter(|(_, witness)| witness.index == index);
                let mode = if output.is_some() {
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
                        .map(|callback| {
                            if check_body && let Some((parameter, witness)) = output {
                                output_gap_explained =
                                    callback.rejected_return.as_ref().is_some_and(|ty| {
                                        *ty == Type::Never
                                            || witness.evidence(ty).is_some_and(|evidence| {
                                                parameter.check_bound(&evidence)
                                                    == Compatibility::Proven
                                            })
                                    });
                            }
                            callback.value
                        })
                        .unwrap_or_else(|| checker.infer(form, env))
                });
                if check_body {
                    values[index].0 = body_checked;
                }
                // A skipped or unproved body does not make closure creation
                // diverge. Keep the placeholder for the argument's reachability.
            }
        }
        EvaluatedArguments {
            values,
            output_gap_explained,
        }
    }

    pub(super) fn call_generic(
        &mut self,
        signature: &Signature,
        function: &Value,
        arguments: CallArguments<'_>,
        call: Span,
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
        let input_valid = valid;
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
                    Err(_) if !arguments.output_gap_explained => {
                        self.missing_parameter(function, parameter, *span, "this callback's result")
                    }
                    Err(_) => {}
                }
            }
            valid &= output_valid;
        }
        // Unknown stands in for a missing binder only while checking the
        // remaining arguments. It cannot hide an independent mismatch.
        let instantiated = signature.with_type_arguments(binders, &evidence, Some(&Type::Unknown));
        if self.type_query.is_some() {
            let specialize = input_valid
                && (valid
                    || binders
                        .output()
                        .is_none_or(|(_, parameter)| parameter.upper.is_none()));
            self.observe_call(
                call,
                arguments,
                signature,
                if specialize { &instantiated } else { signature },
                function.contract.as_ref(),
            );
        }
        let mismatch = self.check_call_arguments(&instantiated, function, arguments);
        if mismatch {
            return Value::rejected_call();
        }
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
