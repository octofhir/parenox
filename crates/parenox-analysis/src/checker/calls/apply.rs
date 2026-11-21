//! Calls whose final input supplies zero or more positional arguments.
use std::collections::BTreeSet;

use super::super::LiteralContainer;
use super::{
    ArgumentForms, Callable, Checker, Env, PreparedCallable, Value, arity_counts, builtin_arities,
    call_returns, vectors::CollectionError, vectors::element_type,
};
use crate::reader::{Form, Kind, Span};
use crate::types::{Arity, Signature, Type};

const MAX_SPREAD_ALTERNATIVES: usize = 16;

struct OpenSpread<'a> {
    element: &'a Type,
    minimum: usize,
    span: Span,
    notes: &'a [String],
}

impl Checker<'_> {
    pub(super) fn apply_form(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        let [apply, callee, prefix @ .., spread] = forms else {
            self.error(
                "PNX2554",
                span,
                "apply expects a function and a final collection",
            );
            return Value::new(Type::Unknown);
        };
        let callable = self.prepare_callable(callee, env);
        let mut args: Vec<_> = self.eval_args(prefix, env, callable.returns()).collect();
        let reachable = callable.returns() && call_returns(&args);
        if let Some(items) = spread.vector() {
            // The literal's syntax supplies every position even for large vectors.
            args.extend(self.eval_args(items, env, reachable));
            let invoked = reachable && call_returns(&args);
            let written: Vec<_> = prefix.iter().chain(items).map(Some).collect();
            return self.call_prepared_with_forms(
                &callable,
                args,
                ArgumentForms::Projected(&written),
                span,
                env,
                invoked,
            );
        }
        if apply
            .symbol()
            .is_some_and(|name| self.model.can_use_standard_core(name, self.globals))
            && let Some(operands) = self.concat_operands(spread, env)
        {
            // concat constructs a lazy result only after evaluating all its
            // operands. Traversing the proved vectors adds no effects.
            let inputs: Vec<_> = self.eval_args(operands, env, reachable).collect();
            if !reachable || !call_returns(&inputs) {
                return self.call_prepared(&callable, args, span, env, false);
            }
            let mut projected: Vec<_> = prefix.iter().map(Some).collect();
            if let Err(unproven) =
                append_concat_arguments(&mut args, &mut projected, inputs, operands)
            {
                self.withhold_callback_facts(&callable, env);
                self.call_prepared(&callable, Vec::new(), span, env, false);
                return self.unchecked_form(
                    unproven,
                    "apply through concat currently needs proven ordinary vector or Nil operands",
                );
            }
            let start = self.diagnostics.len();
            let result = self.call_prepared_with_forms(
                &callable,
                args,
                ArgumentForms::Projected(&projected),
                span,
                env,
                true,
            );
            for form in operands.iter().filter(|form| form.vector().is_none()) {
                self.explain_spread_elements(start, form.span);
            }
            return result;
        }
        let collection = self.infer_path(spread, env, reachable, false);
        if collection.known == Type::Never {
            return self.call_prepared(&callable, args, span, env, false);
        }
        if collection.unproven_tuple_traversal() {
            self.withhold_callback_facts(&callable, env);
            self.call_prepared(&callable, Vec::new(), span, env, false);
            return self.unchecked_tuple_traversal("clojure.core/apply", spread.span);
        }
        let types = match &collection.known {
            Type::Union(types) => types.as_slice(),
            ty => std::slice::from_ref(ty),
        };
        if types.len() > MAX_SPREAD_ALTERNATIVES {
            return self.unsupported(spread.span, "apply has too many possible collection shapes");
        }
        let mut result = Value::new(Type::Never);
        for ty in types {
            let elements = match ty {
                Type::Nil => &[][..],
                Type::Vector(element) if **element == Type::Never => &[][..],
                Type::Tuple(elements) | Type::SeqTuple(elements) | Type::ListTuple(elements) => {
                    elements
                }
                Type::MapEntry(elements) => elements.as_slice(),
                Type::Vector(element) | Type::Seq(element) | Type::List(element) => {
                    let open = OpenSpread {
                        element,
                        minimum: usize::from(matches!(ty, Type::Seq(_) | Type::List(_))),
                        span: spread.span,
                        notes: &collection.notes,
                    };
                    result = result.join(self.apply_open(&callable, &args, open, span));
                    continue;
                }
                Type::Unknown => return Value::new(Type::Unknown),
                ty => {
                    if matches!(element_type(ty.clone()), Err(CollectionError::NotSeqable)) {
                        self.error(
                            "PNX2322",
                            spread.span,
                            format!(
                                "apply expects a seqable final argument, got {}",
                                ty.inferred()
                            ),
                        );
                        self.may_throw();
                        return Value::new(Type::Unknown);
                    }
                    return self.unsupported(
                        spread.span,
                        "apply needs a final collection with a known argument count",
                    );
                }
            };
            let mut actual = Vec::with_capacity(args.len() + elements.len());
            actual.extend(args.iter().cloned());
            actual.extend(
                elements
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| (collection.position_value(ty.clone(), index), spread.span)),
            );
            let start = self.diagnostics.len();
            let value = self.call_prepared(&callable, actual, span, env, true);
            self.explain_spread_elements(start, spread.span);
            result = result.join(value);
        }
        result
    }

    fn concat_operands<'form>(&self, form: &'form Form, env: &Env) -> Option<&'form [Form]> {
        let Kind::List(forms) = &form.kind else {
            return None;
        };
        let (head, operands) = forms.split_first()?;
        (self.standard_core_name(head, env)?.as_str() == "clojure.core/concat").then_some(operands)
    }

    fn apply_open(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        prefix: &[(Value, Span)],
        spread: OpenSpread<'_>,
        span: Span,
    ) -> Value {
        let minimum = prefix.len() + spread.minimum;
        let element = Value::with_notes(spread.element.clone(), spread.notes.to_vec());
        let start = self.diagnostics.len();
        let result = match &callable.kind {
            Callable::Partial(wrapper) => {
                let arguments: Vec<_> = wrapper.prefix.iter().chain(prefix).cloned().collect();
                return self.apply_open(&wrapper.target, &arguments, spread, span);
            }
            Callable::Value(function) => match &function.known {
                Type::Function(signatures)
                    if signatures.iter().any(|sig| sig.parameters.is_some()) =>
                {
                    self.unsupported(
                        spread.span,
                        "apply cannot infer a type parameter from an open spread",
                    )
                }
                Type::Function(signatures) => {
                    let arities: Vec<_> = signatures.iter().map(Signature::arity).collect();
                    self.check_spread_arities(&arities, minimum, spread.span);
                    let value = self.check_spread_signatures(
                        signatures,
                        prefix,
                        minimum,
                        &element,
                        spread.span,
                    );
                    if let Some(contract) = &function.contract {
                        for diagnostic in &mut self.diagnostics[start..] {
                            diagnostic.contract = Some(contract.as_ref().clone());
                        }
                    }
                    value
                }
                Type::Map(_) | Type::Key(_) | Type::Keyword | Type::SpecKeyword => {
                    self.check_spread_arities(
                        &[Arity::Fixed(1), Arity::Fixed(2)],
                        minimum,
                        spread.span,
                    );
                    self.may_throw();
                    Value::new(Type::Unknown)
                }
                Type::Set(_) | Type::Vector(_) | Type::Tuple(_) | Type::MapEntry(_) => {
                    self.check_spread_arities(&[Arity::Fixed(1)], minimum, spread.span);
                    self.may_throw();
                    Value::new(Type::Unknown)
                }
                Type::Unknown => Value::new(Type::Unknown),
                _ => self.unsupported(
                    callable.span,
                    format!("Calling {} is not supported", function.ty),
                ),
            },
            Callable::Builtin(name) if name == "clojure.core/list" => {
                let items = prefix.iter().map(|(value, _)| value.known.clone());
                let sequence = Type::List(Box::new(Type::union(
                    items.chain(std::iter::once(element.known.clone())),
                )));
                Value::new(if minimum == 0 {
                    Type::union([Type::empty_list(), sequence])
                } else {
                    sequence
                })
            }
            Callable::Builtin(name) if name == "clojure.core/str" => {
                self.may_throw();
                Value::new(Type::String)
            }
            Callable::Builtin(name)
                if matches!(name.as_str(), "clojure.core/min" | "clojure.core/max") =>
            {
                self.check_spread_arities(builtin_arities(name), minimum, spread.span);
                let result = self.open_extremum(name, prefix, &element, minimum);
                self.note_builtin_contract(start, name);
                result
            }
            Callable::Builtin(name)
                if !matches!(
                    name.as_str(),
                    "clojure.core/conj" | "clojure.core/merge" | "clojure.core/dissoc"
                ) =>
            {
                let arities = builtin_arities(name);
                self.check_spread_arities(arities, minimum, spread.span);
                self.note_builtin_contract(start, name);
                if let [Arity::Variadic(required)] = arities {
                    // These built-ins have the same result type for every accepted count.
                    let mut actual = prefix.to_vec();
                    actual.resize(
                        minimum.max(prefix.len() + 1).max(*required),
                        (element, spread.span),
                    );
                    self.builtin(name, &actual, span)
                } else {
                    self.may_throw();
                    Value::new(Type::Unknown)
                }
            }
            _ => self.unsupported(
                spread.span,
                "apply cannot infer every argument count for this callback and collection",
            ),
        };
        self.explain_spread_elements(start, spread.span);
        result
    }

    fn check_spread_arities(&mut self, arities: &[Arity], minimum: usize, span: Span) {
        // One count above the largest fixed arity represents the unbounded tail.
        // For a variadic arm all counts at or above its required count are covered.
        let boundary = arities
            .iter()
            .map(|arity| match *arity {
                Arity::Fixed(count) => count + 1,
                Arity::Variadic(required) => required,
            })
            .max()
            .unwrap_or(minimum);
        if let Some(missing) = (minimum..=boundary.max(minimum))
            .find(|&count| !arities.iter().any(|arity| arity.accepts(count)))
        {
            self.error(
                "PNX2554",
                span,
                format!(
                    "apply may pass {missing} arguments, but the function expects {}",
                    arity_counts(arities.iter().copied())
                ),
            );
        }
    }

    fn check_spread_signatures(
        &mut self,
        signatures: &[Signature],
        prefix: &[(Value, Span)],
        minimum: usize,
        element: &Value,
        span: Span,
    ) -> Value {
        self.may_throw();
        let matching: Vec<_> = signatures
            .iter()
            .filter(|signature| signature.rest.is_some() || signature.args.len() >= minimum)
            .collect();
        if matching.is_empty() {
            return Value::new(Type::Unknown);
        }
        for (index, (value, span)) in prefix.iter().enumerate() {
            let expected: BTreeSet<_> = matching
                .iter()
                .filter_map(|sig| sig.args.get(index).or(sig.rest.as_ref()))
                .collect();
            for ty in expected {
                self.expect(value, ty, *span, &format!("Argument {}", index + 1));
            }
        }
        let expected_elements: BTreeSet<_> = matching
            .iter()
            .flat_map(|sig| sig.args.iter().skip(prefix.len()).chain(sig.rest.as_ref()))
            .collect();
        for ty in expected_elements {
            self.expect(element, ty, span, "apply collection elements");
        }
        Value::new(Type::union(matching.iter().map(|sig| sig.ret.clone())))
    }

    fn explain_spread_elements(&mut self, start: usize, span: Span) {
        for diagnostic in &mut self.diagnostics[start..] {
            if diagnostic.code == "PNX2322" && diagnostic.span == span {
                diagnostic.suggestions.clear();
                diagnostic.help = Some(
                    "Check the elements of the final collection against the function's argument types.".into(),
                );
            }
        }
    }
}

fn append_concat_arguments<'form>(
    args: &mut Vec<(Value, Span)>,
    projected: &mut Vec<Option<&'form Form>>,
    inputs: Vec<(Value, Span)>,
    operands: &'form [Form],
) -> Result<(), Span> {
    for ((input, span), form) in inputs.into_iter().zip(operands) {
        if input.known == Type::Nil {
            continue;
        }
        let (Type::Tuple(items), Some(container @ LiteralContainer::Vector(_))) =
            (input.known, input.literal_container)
        else {
            return Err(span);
        };
        let literal = form.vector().filter(|forms| forms.len() == items.len());
        for (index, ty) in items.into_vec().into_iter().enumerate() {
            let site = literal.map_or(form, |forms| &forms[index]);
            let mut value = Value::with_notes(ty, input.notes.clone());
            value.literal_container = container.position_child(index).cloned();
            args.push((value, site.span));
            projected.push(Some(site));
        }
    }
    Ok(())
}
