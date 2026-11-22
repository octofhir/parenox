//! Infer input from an argument or a supported projection, then output from one callback.
use std::borrow::Cow;

use super::{
    CallArguments, CallOperands, ContextualArgument, EvaluatedArguments,
    inline::{CallbackReturn, ContextualCallback},
};
use crate::Diagnostic;
use crate::checker::{Checker, Env, Value};
use crate::reader::Span;
use crate::types::{
    CallbackWitness, Compatibility, Parameter, Parameters, Signature, Type, TypeArguments,
};

fn output_evidence<'a>(
    witness: &CallbackWitness<'_>,
    callback: &'a Type,
    parameters: &Parameters,
    evidence: &TypeArguments<'_>,
) -> Result<Cow<'a, Type>, Compatibility> {
    if let Type::Union(alternatives) = callback {
        let mut candidates = Vec::with_capacity(alternatives.len());
        let status =
            Compatibility::all(alternatives.iter().map(|alternative| {
                match output_evidence(witness, alternative, parameters, evidence) {
                    Ok(candidate) => {
                        candidates.push(candidate.into_owned());
                        Compatibility::Proven
                    }
                    Err(status) => status,
                }
            }));
        if status != Compatibility::Proven {
            return Err(status);
        }
        return Ok(Cow::Owned(Type::union(candidates)));
    }
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
        forms: CallOperands<'_>,
        call: Span,
        env: &Env,
    ) -> EvaluatedArguments {
        let mut values = Vec::with_capacity(forms.len());
        let mut callbacks = Vec::new();
        let mut reachable = env.reachable();
        for (index, (form, expected)) in forms.iter().zip(&signature.args).enumerate() {
            if let Some((context, argument)) =
                self.prepare_contextual_argument(form, expected, env, reachable, None)
            {
                callbacks.push((index, context, reachable));
                reachable = !argument.stops_evaluation();
                values.push((argument, form.span));
            } else if matches!(expected, Type::Function(_))
                && self.contextual_callback_form(form, env)
            {
                // An overloaded expectation still defers the closure body.
                callbacks.push((
                    index,
                    ContextualArgument {
                        callback: form,
                        expected,
                        choice: None,
                        map_key: None,
                        map_ancestors: Box::new([]),
                    },
                    reachable,
                ));
                let argument = self.contextual_callback_argument(form, env, reachable);
                reachable = !argument.stops_evaluation();
                values.push((argument, form.span));
            } else {
                let mut value = self
                    .contextual_map_argument(form, expected, env, reachable, |callback, here| {
                        callbacks.push((index, callback, here));
                    })
                    .unwrap_or_else(|| self.infer_path(form, env, reachable, false));
                reachable = !value.stops_evaluation();
                if !reachable {
                    value.narrow(Type::Never);
                }
                values.push((value, form.span));
            }
        }
        let tail_outcome = forms.metadata_outcome(self, env, reachable);
        reachable &= tail_outcome.is_none();
        let mut output_gap_explained = false;
        let mut output_callback_span = None;
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
            for (index, mut contextual, reachable_at_argument) in callbacks {
                let form = contextual.callback;
                let template = contextual.expected;
                let expected = contextual
                    .expected_in(&instantiated.args[index])
                    .expect("generic instantiation preserves contextual fields");
                // A missing argument stays Unknown. Known context lets us
                // check another input without pretending the call was proved.
                let has_context = !binders.needs_missing_input(template, &missing)
                    || binders.inputs().any(|(index, param)| {
                        !missing[index] && template.mentions_parameter(param)
                    });
                let check_body = reachable && reachable_at_argument && has_context;
                let output = output_callback.as_ref().filter(|(_, witness)| {
                    witness.index == index
                        && witness.matches_path(&contextual.map_ancestors, contextual.map_key)
                });
                let mode = if output.is_some() {
                    CallbackReturn::Infer
                } else {
                    CallbackReturn::Check
                };
                let origin = function.contract.as_deref();
                let checked = if let Some(choice) = contextual.choice.take() {
                    self.check_callback_choice(*choice, expected, call, origin, check_body, mode)
                } else {
                    self.inspect_path(check_body, |checker| {
                        checker
                            .contextual_callback(form, expected, call, origin, env, mode)
                            .unwrap_or_else(|| {
                                ContextualCallback::from_value(checker.infer(form, env))
                            })
                    })
                };
                if check_body {
                    if let Some((parameter, witness)) = output {
                        output_callback_span = Some(form.without_metadata().span);
                        output_gap_explained = checked.rejected_return.as_ref().is_some_and(|ty| {
                            *ty == Type::Never
                                || witness.evidence(ty).is_some_and(|evidence| {
                                    parameter.check_bound(&evidence) == Compatibility::Proven
                                })
                        });
                    }
                    contextual.apply(&mut values[index].0, checked.value);
                }
                // A skipped or unproved body does not make closure creation
                // diverge. Keep the placeholder for the argument's reachability.
            }
        }
        EvaluatedArguments {
            values,
            tail_outcome,
            output_gap_explained,
            output_callback_span,
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
            if let Some(ty) = ty {
                let source = evidence.input_source(index).expect("evidenced argument");
                let span = args[source].1;
                valid &= self.generic_bound(function, parameter, ty, span);
            } else {
                let witness = signature
                    .input_witness(parameter)
                    .expect("declared input argument");
                let span = args[witness.index].1;
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
                let candidate = witness
                    .callback_type(&value.ty)
                    .ok_or(Compatibility::Unproven)
                    .and_then(|ty| output_evidence(&witness, ty, binders, &evidence));
                match candidate {
                    Ok(candidate) => {
                        let span = arguments.output_callback_span.unwrap_or(*span);
                        output_valid = self.generic_bound(function, parameter, &candidate, span);
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
