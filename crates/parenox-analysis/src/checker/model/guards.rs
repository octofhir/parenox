//! Verify the two implications of a guard independently before publishing it.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{Declarations, Function, Globals, Model};
use crate::Diagnostic;
use crate::checker::calls::predicate_type;
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct VerifiedGuard {
    pub argument: usize,
    pub arity: usize,
    pub ty: Type,
    pub both: bool,
}

impl Model {
    pub(super) fn verify_guards(
        &mut self,
        claims: BTreeMap<String, &Form>,
        declarations: &Declarations,
        forms: &[Form],
        globals: &Globals,
    ) {
        if claims.is_empty() {
            return;
        }
        if self.check_project_core(globals) {
            self.diagnostics.extend(claims.values().map(|claim| {
                Diagnostic::new(
                    "PNX9008",
                    claim.span,
                    "Cannot verify a guard with a project-provided clojure.core namespace",
                )
            }));
            return;
        }
        // Even a rejected declaration can shadow an unqualified core name.
        let local_vars = declarations
            .functions
            .iter()
            .chain(&declarations.literal_vars)
            .chain(&declarations.unchecked_vars)
            .filter_map(|&index| match &forms[index].without_metadata().kind {
                Kind::List(parts) => parts.get(1).and_then(Form::symbol),
                _ => None,
            })
            .chain(
                declarations
                    .scoped_functions(forms)
                    .filter_map(|items| items[1].symbol()),
            )
            .chain(
                declarations
                    .protocol_vars(forms)
                    .filter_map(|var| var.name.symbol()),
            )
            .collect();
        for (name, claim) in claims {
            match prove(self, &self.functions[&name], claim, &local_vars) {
                Ok(guard) => {
                    self.functions
                        .get_mut(&name)
                        .unwrap()
                        .contract
                        .as_mut()
                        .unwrap()
                        .guard = Some(Arc::new(guard));
                }
                Err(diagnostic) => self.diagnostics.push(*diagnostic),
            }
        }
    }
}

fn prove(
    model: &Model,
    function: &Function,
    claim: &Form,
    local_vars: &BTreeSet<&str>,
) -> Result<VerifiedGuard, Box<Diagnostic>> {
    let invalid = |message| error(claim.span, message);
    let Some(data) = claim.quote_operand() else {
        return Err(invalid(":t/guard must be quoted data"));
    };
    let Some([mode, param, predicate]) = data.vector() else {
        return Err(invalid(
            "Expected :t/guard '[:then parameter predicate?] or '[:both parameter predicate?]",
        ));
    };
    let both = match mode.keyword() {
        Some(":then") => false,
        Some(":both") => true,
        _ => return Err(invalid(":t/guard direction must be :then or :both")),
    };
    let Some(param) = param.symbol() else {
        return Err(invalid(":t/guard must name a parameter"));
    };
    let Some(ty) = predicate
        .symbol()
        .and_then(|name| scalar_predicate(&model.resolve(name)))
    else {
        return Err(invalid(":t/guard requires a supported scalar predicate"));
    };
    let Some([signature]) = function.contract.as_ref().map(|c| c.signatures.as_ref()) else {
        return Err(invalid(":t/guard requires a single fixed-arity signature"));
    };
    if signature.rest.is_some() || !matches!(signature.ret, Type::Boolean | Type::Bool(_)) {
        return Err(invalid(
            ":t/guard requires a fixed arity and a Boolean return contract",
        ));
    }
    let arm = &function.arities[&signature.arity()];
    if !arm.preconditions.is_empty() || arm.postconditions.is_some() {
        return Err(invalid(
            ":t/guard verification requires an arity without preconditions or postconditions",
        ));
    }
    let mut names = BTreeSet::new();
    let params: Vec<_> = arm
        .params
        .iter()
        .enumerate()
        .map(|(index, form)| {
            let name = match &form.kind {
                Kind::Symbol(name) => Some(name.as_str()),
                _ => None,
            }
            .filter(|name| !name.contains('/') && *name != "&")
            .filter(|name| names.insert(*name))
            .ok_or_else(|| {
                invalid(":t/guard requires distinct symbol parameters without destructuring")
            })?;
            Ok((name, signature.args[index].subtype_of(&Type::Boolean)))
        })
        .collect::<Result<_, Box<Diagnostic>>>()?;
    let Some(argument) = params.iter().position(|(name, _)| *name == param) else {
        return Err(invalid(":t/guard must name a parameter of this function"));
    };
    let [body] = arm.body.as_slice() else {
        return Err(invalid(
            ":t/guard requires a single boolean expression as the body",
        ));
    };
    let proof = Proof {
        model,
        params: &params,
        subject: param,
        ty: &ty,
        local_vars,
    };

    // Do not use the signature's input types as evidence. In each run,
    // unrelated tests may be either true or false, even if correlated.
    if proof.evaluate(body, false)? != Truth::False {
        return Err(invalid(
            "Guard body does not prove that a true result implies the declared type",
        ));
    }
    if both && proof.evaluate(body, true)? != Truth::True {
        return Err(invalid(
            "Guard body does not prove that a false result excludes the declared type",
        ));
    }
    Ok(VerifiedGuard {
        argument,
        arity: params.len(),
        ty,
        both,
    })
}

fn scalar_predicate(op: &str) -> Option<Type> {
    if op == "clojure.core/nil?" {
        Some(Type::Nil)
    } else {
        predicate_type(op)
    }
}

fn error(span: Span, message: &str) -> Box<Diagnostic> {
    Box::new(Diagnostic::new("PNX9008", span, message))
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Truth {
    False,
    Either,
    True,
}

impl Truth {
    fn known(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    fn negate(self) -> Self {
        match self {
            Self::False => Self::True,
            Self::True => Self::False,
            Self::Either => Self::Either,
        }
    }
}

struct Proof<'a> {
    model: &'a Model,
    params: &'a [(&'a str, bool)],
    subject: &'a str,
    ty: &'a Type,
    local_vars: &'a BTreeSet<&'a str>,
}

impl Proof<'_> {
    fn evaluate(&self, form: &Form, has_type: bool) -> Result<Truth, Box<Diagnostic>> {
        let unsupported = || {
            error(
                form.span,
                "Guard proof supports only core scalar tests, Boolean parameters and literals, and/or/not and if",
            )
        };
        match &form.without_metadata().kind {
            Kind::Bool(value) => Ok(Truth::known(*value)),
            Kind::Symbol(name) if self.params.contains(&(name, true)) => Ok(Truth::Either),
            Kind::List(forms) => {
                let (head, args) = forms.split_first().ok_or_else(unsupported)?;
                let op = head.symbol().ok_or_else(unsupported)?;
                if op == "if" {
                    let [condition, yes, no] = args else {
                        return Err(unsupported());
                    };
                    let condition = self.evaluate(condition, has_type)?;
                    let yes = self.evaluate(yes, has_type)?;
                    let no = self.evaluate(no, has_type)?;
                    return Ok(match condition {
                        Truth::True => yes,
                        Truth::False => no,
                        Truth::Either if yes == no => yes,
                        Truth::Either => Truth::Either,
                    });
                }
                if self.params.iter().any(|(name, _)| *name == op) || self.local_vars.contains(op) {
                    return Err(unsupported());
                }
                let resolved = self.model.resolve(op);
                if resolved.split_once('/').is_some_and(|(namespace, name)| {
                    namespace == self.model.namespace() && self.local_vars.contains(name)
                }) {
                    return Err(unsupported());
                }
                match resolved.as_str() {
                    combinator @ ("clojure.core/and" | "clojure.core/or") if !args.is_empty() => {
                        let and = combinator == "clojure.core/and";
                        let mut result = Truth::known(and);
                        for operand in args {
                            let next = self.evaluate(operand, has_type)?;
                            result = if and {
                                result.min(next)
                            } else {
                                result.max(next)
                            };
                        }
                        Ok(result)
                    }
                    "clojure.core/not" => {
                        let [operand] = args else {
                            return Err(unsupported());
                        };
                        Ok(self.evaluate(operand, has_type)?.negate())
                    }
                    op => {
                        let [arg] = args else {
                            return Err(unsupported());
                        };
                        let name = arg
                            .symbol()
                            .filter(|name| self.params.iter().any(|(param, _)| param == name))
                            .ok_or_else(unsupported)?;
                        let positive = op != "clojure.core/some?";
                        let ty = if positive {
                            scalar_predicate(op).ok_or_else(unsupported)?
                        } else {
                            Type::Nil
                        };
                        Ok(if name == self.subject && &ty == self.ty {
                            Truth::known(has_type == positive)
                        } else {
                            Truth::Either
                        })
                    }
                }
            }
            _ => Err(unsupported()),
        }
    }
}
