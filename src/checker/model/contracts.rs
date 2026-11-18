//! Annotated signatures and function declarations.
use std::collections::BTreeMap;

pub(super) fn function_parts(tail: &[Form]) -> (&[Form], impl Iterator<Item = &[Form; 2]> + Clone) {
    let tail = if matches!(tail.first().map(|form| &form.kind), Some(Kind::String(_))) {
        &tail[1..]
    } else {
        tail
    };
    let (attrs, arms) = match tail.split_first() {
        Some((first, rest)) if first.map().is_some() => (first.map().unwrap(), rest),
        _ => (&[][..], tail),
    };
    let (trailing, arms) = match arms.split_last() {
        Some((last, rest))
            if arms.first().and_then(Form::vector).is_none() && last.map().is_some() =>
        {
            (last.map().unwrap(), rest)
        }
        _ => (&[][..], arms),
    };
    (
        arms,
        trailing
            .as_chunks::<2>()
            .0
            .iter()
            .chain(attrs.as_chunks::<2>().0),
    )
}
use std::sync::Arc;

use super::{
    Contract, Function, FunctionArity, Globals, Model, Signature,
    type_forms::read_type_with_parameters,
};
use crate::ContractKind;
use crate::checker::function_forms::{function_clauses, split_rest};
use crate::reader::{Form, Kind, Span};
use crate::types::{Arity, MAX_TYPE_PARAMETERS, Parameter, Parameters, Type};

impl Model {
    fn contract_type(
        &mut self,
        form: &Form,
        parameters: Option<&Parameters>,
        globals: &Globals,
    ) -> Type {
        read_type_with_parameters(
            &self.names,
            form,
            parameters,
            &mut self.diagnostics,
            |key, _| track_spec(&mut self.external_specs, globals, key),
        )
    }

    pub(super) fn read_function<'a>(
        &mut self,
        xs: &'a [Form],
        span: Span,
        globals: &Globals,
        fdefs: &mut BTreeMap<String, &[Form]>,
        guards: &mut BTreeMap<String, &'a Form>,
    ) {
        let Some(symbol) = xs.get(1).and_then(Form::symbol) else {
            self.error("PNX9001", span, "defn requires a function name");
            return;
        };
        let Some(name) = self.definition_name(symbol, xs[1].span, "defn") else {
            return;
        };
        let fdef = fdefs.remove(&name);
        if self.literals.contains_key(&name) {
            self.error("PNX2790", xs[1].span, format!("Duplicate var {name}"));
            return;
        }
        let (arms, attrs) = function_parts(&xs[2..]);
        let attribute = |key| {
            attrs
                .clone()
                .find_map(|[name, value]| (name.keyword() == Some(key)).then_some(value))
        };
        let default_private = xs[0]
            .symbol()
            .is_some_and(|s| self.resolve(s) == "clojure.core/defn-");
        let private_form = attribute(":private").or_else(|| {
            if default_private {
                None
            } else {
                xs[1].metadata(":private")
            }
        });
        let private = if let Some(form) = private_form {
            match form.without_metadata().kind {
                Kind::Bool(value) => value,
                _ => {
                    self.error(
                        "PNX9001",
                        form.span,
                        ":private currently requires a literal boolean",
                    );
                    default_private
                }
            }
        } else {
            default_private
        };
        let metadata = |key| attribute(key).or_else(|| xs[1].metadata(key));
        let signature = metadata(":t/sig");
        let guard = metadata(":t/guard").filter(|guard| {
            // An inline expansion or a dynamic var can bypass the verified body.
            if [":dynamic", ":inline", ":macro"].iter().any(|key| {
                metadata(key).is_some_and(|form| {
                    !matches!(form.without_metadata().kind, Kind::Nil | Kind::Bool(false))
                })
            }) {
                self.error(
                    "PNX9008",
                    guard.span,
                    ":t/guard cannot describe a dynamic, inline, or macro var",
                );
                false
            } else {
                true
            }
        });
        let inferred = signature.is_none() && fdef.is_none();
        if inferred && !private {
            self.error(
                "PNX9002",
                span,
                format!("Public function {name} needs a :t/sig or s/fdef signature"),
            );
            return;
        }
        let clauses = match function_clauses(arms, span) {
            Ok(clauses) => clauses,
            Err(diagnostic) => {
                self.diagnostics.push(*diagnostic);
                return;
            }
        };
        let arities: BTreeMap<_, _> = clauses
            .into_iter()
            .map(|(count, clause)| {
                (
                    count,
                    FunctionArity {
                        params: clause.bindings().into_owned(),
                        body: clause.body.to_vec(),
                        span: clause.span,
                    },
                )
            })
            .collect();
        if inferred && arities.len() > 1 {
            self.error(
                "PNX9002",
                span,
                format!("Multi-arity private function {name} needs a :t/sig annotation"),
            );
            return;
        }
        let declared = if let Some(signature) = signature {
            if !matches!(signature.without_metadata().kind, Kind::Quote(_)) {
                self.error("PNX9001", signature.span, ":t/sig must be quoted data");
            }
            let Some(signatures) = self.parse_signatures(signature, &name, globals) else {
                return;
            };
            Some((signatures, signature.span, ContractKind::TSig))
        } else {
            None
        };
        let spec_signature = if let Some(fdef) = fdef {
            let Some(signature) = self.parse_fdef(fdef, globals) else {
                return;
            };
            if let Some((signatures, _, _)) = &declared
                && signatures.as_slice() != std::slice::from_ref(&signature)
            {
                self.error(
                    "PNX2790",
                    fdef[1].span,
                    format!("s/fdef and :t/sig disagree for {name}"),
                );
            }
            Some((vec![signature], fdef[1].span, ContractKind::SFdef))
        } else {
            None
        };
        let contract = if let Some((signatures, signature_span, kind)) = declared.or(spec_signature)
        {
            if !arities
                .keys()
                .copied()
                .eq(signatures.iter().map(Signature::arity))
            {
                self.error(
                    "PNX2554",
                    signature_span,
                    "Signature arities must match the function's parameters",
                );
                return;
            }
            Some(Contract::new(signatures, &name, kind, &self.path))
        } else {
            None
        };
        let function = Function {
            name: name.clone(),
            private,
            arities,
            contract,
            span,
        };
        if let Some(guard) = guard {
            guards.insert(name.clone(), guard);
        } else {
            guards.remove(&name);
        }
        self.names.refers.remove(symbol);
        if self.functions.insert(name.clone(), function).is_some() {
            self.error("PNX2790", span, format!("Duplicate function {name}"));
        }
    }

    pub(super) fn parse_signatures(
        &mut self,
        form: &Form,
        owner: &str,
        globals: &Globals,
    ) -> Option<Vec<Signature>> {
        if let Kind::List(items) = &form.unquoted().kind
            && items.first().and_then(Form::symbol) == Some("t/all")
        {
            return self
                .parse_generic_signature(items, form.span, owner, globals)
                .map(|signature| vec![signature]);
        }
        let types = match &form.unquoted().without_metadata().kind {
            Kind::Vector(items) | Kind::List(items) => Some(items.as_slice()),
            _ => None,
        };
        let Some(arms) =
            types.filter(|arms| !arms.is_empty() && arms.iter().all(|arm| arm.vector().is_some()))
        else {
            return self
                .parse_signature(form, None, globals)
                .map(|signature| vec![signature]);
        };
        let mut signatures = BTreeMap::new();
        for arm in arms {
            let signature = self.parse_signature(arm, None, globals)?;
            let count = signature.arity();
            if signatures.insert(count, signature).is_some() {
                self.error(
                    "PNX2554",
                    arm.span,
                    format!("Duplicate signature arity {count}"),
                );
                return None;
            }
        }
        if !Arity::valid_set(signatures.keys().copied()) {
            self.error("PNX2554", form.span,
                "A variadic signature must be unique and require at least as many arguments as every fixed signature");
            return None;
        }
        Some(signatures.into_values().collect())
    }

    fn parse_generic_signature(
        &mut self,
        forms: &[Form],
        span: Span,
        owner: &str,
        globals: &Globals,
    ) -> Option<Signature> {
        let [_, binders, signature] = forms else {
            self.error(
                "PNX9001",
                span,
                "Expected (t/all [Parameter ...] [Arg ... :-> Result])",
            );
            return None;
        };
        let Some(binders) = binders
            .vector()
            .filter(|xs| (1..=MAX_TYPE_PARAMETERS).contains(&xs.len()))
        else {
            self.error(
                "PNX9001",
                binders.span,
                "t/all supports one or two input parameters and one optional callback result parameter",
            );
            return None;
        };
        let mut parameters: Vec<Arc<Parameter>> = Vec::with_capacity(binders.len());
        for form in binders {
            let parameter = self.parse_type_parameter(form, owner, globals)?;
            if parameters
                .iter()
                .any(|previous| previous.name == parameter.name)
            {
                self.error(
                    "PNX9001",
                    form.span,
                    "Type parameters must have distinct names",
                );
                return None;
            }
            parameters.push(parameter);
        }
        let parameters = Arc::new(Parameters {
            binders: parameters.into_boxed_slice(),
            output: None,
        });
        let mut signature = self.parse_signature(signature, Some(parameters), globals)?;
        if signature.rest.is_some() {
            self.error("PNX9001", span, "t/all requires fixed arity");
            return None;
        }
        let parameters = signature.parameters.as_ref().unwrap();
        let leaf = |ty: &Type| matches!(ty, Type::Parameter(_)) || !ty.contains_parameter();
        let value_slot =
            |ty: &Type| leaf(ty) || parameters.iter().any(|p| ty.has_parameter_slot(p));
        let result_slot =
            |ty: &Type| value_slot(ty) || parameters.iter().any(|p| ty.has_nullable_parameter(p));
        if !result_slot(&signature.ret)
            || !signature.args.iter().all(|ty| {
                value_slot(ty)
                    || matches!(ty, Type::Function(signatures)
                if matches!(signatures.as_ref(), [callback] if callback.rest.is_none()
                    && callback.args.iter().all(value_slot) && result_slot(&callback.ret)))
            })
        {
            self.error("PNX9001", span,
                "A type parameter can appear directly, as a vector element or map value, or in a nullable result");
            return None;
        }
        let mut inputs = 0;
        let mut output = None;
        for (index, parameter) in parameters.iter().enumerate() {
            if signature.input_witness(parameter).is_some() {
                inputs += 1;
                continue;
            }
            if output.is_some()
                || !signature.output_callback(parameter).is_some_and(|witness| {
                    (signature.ret.has_parameter_slot(parameter)
                        || signature.ret.has_nullable_parameter(parameter))
                        && witness
                            .signature
                            .args
                            .iter()
                            .all(|ty| !ty.mentions_parameter(parameter))
                        && signature
                            .args
                            .iter()
                            .enumerate()
                            .all(|(i, ty)| i == witness.index || !ty.mentions_parameter(parameter))
                })
            {
                self.error("PNX9001", binders[index].span, format!("Type parameter {} needs an ordinary argument or must be the only parameter inferred from a callback result used by the call's result type", parameter.name));
                return None;
            }
            output = Some(index);
        }
        if !(1..=2).contains(&inputs) {
            self.error(
                "PNX9001",
                span,
                "t/all requires one or two parameters witnessed by ordinary arguments",
            );
            return None;
        }
        Arc::make_mut(signature.parameters.as_mut().unwrap()).output = output;
        Some(signature)
    }

    fn parse_type_parameter(
        &mut self,
        binder: &Form,
        owner: &str,
        globals: &Globals,
    ) -> Option<Arc<Parameter>> {
        let (name, upper) = match &binder.without_metadata().kind {
            Kind::List(items) if matches!(items.as_slice(), [_, op, _] if op.keyword() == Some(":<")) => {
                (&items[0], Some(&items[2]))
            }
            Kind::Symbol(_) => (binder, None),
            _ => {
                self.error(
                    "PNX9001",
                    binder.span,
                    "Expected a type parameter name or (name :< primitive-type)",
                );
                return None;
            }
        };
        let Some(name) = name
            .symbol()
            .filter(|name| !name.contains('/') && *name != "&")
        else {
            self.error(
                "PNX9001",
                name.span,
                "A type parameter must be an unqualified symbol",
            );
            return None;
        };
        let upper = if let Some(form) = upper {
            if form.symbol().is_none() {
                self.error(
                    "PNX9001",
                    form.span,
                    "A type parameter bound must be a primitive predicate",
                );
                return None;
            }
            let ty = self.contract_type(form, None, globals);
            match ty {
                Type::Number
                | Type::String
                | Type::Boolean
                | Type::Character
                | Type::Keyword
                | Type::Symbol => Some(ty),
                Type::Unknown => return None, // contract_type already reported this bound.
                _ => {
                    self.error(
                        "PNX9001",
                        form.span,
                        "A type parameter bound must be a primitive predicate",
                    );
                    return None;
                }
            }
        } else {
            None
        };
        Some(Arc::new(Parameter {
            owner: owner.into(),
            name: name.into(),
            upper,
        }))
    }

    fn parse_signature(
        &mut self,
        form: &Form,
        parameters: Option<Arc<Parameters>>,
        globals: &Globals,
    ) -> Option<Signature> {
        let Some(types) = form.unquoted().vector() else {
            self.error("PNX9001", form.span, "Expected [Arg ... :-> Result]");
            return None;
        };
        if types.len() < 2
            || types[types.len() - 2].keyword() != Some(":->")
            || types[..types.len() - 2]
                .iter()
                .any(|t| t.keyword() == Some(":->"))
        {
            self.error("PNX9001", form.span, "Expected [Arg ... :-> Result]");
            return None;
        }
        let (args, rest) = match split_rest(&types[..types.len() - 2]) {
            Ok(parts) => parts,
            Err(span) => {
                self.error("PNX1001", span, "& requires exactly one rest element type");
                return None;
            }
        };
        if args.len() > 20 {
            self.error(
                "PNX2554",
                form.span,
                "Clojure/JVM supports at most 20 fixed parameters",
            );
            return None;
        }
        let args = args
            .iter()
            .map(|t| self.contract_type(t, parameters.as_deref(), globals))
            .collect();
        let ret = self.contract_type(types.last().unwrap(), parameters.as_deref(), globals);
        let rest = rest.map(|form| self.contract_type(form, parameters.as_deref(), globals));
        Some(Signature {
            args,
            rest,
            ret,
            parameters,
        })
    }
}

pub(super) fn track_spec(
    dependencies: &mut BTreeMap<String, Option<Type>>,
    globals: &Globals,
    key: &str,
) -> Option<Type> {
    let ty = globals.specs.get(key);
    dependencies
        .entry(key.to_owned())
        .or_insert_with(|| ty.cloned());
    ty.cloned()
}
