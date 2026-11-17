//! Annotated signatures and function declarations.
use std::collections::BTreeMap;

use super::{Function, FunctionArity, Globals, Model, Signature, type_forms::read_type};
use crate::checker::function_forms::{function_clauses, split_rest};
use crate::reader::{Form, Kind, Span};
use crate::types::{Arity, Type};

impl Model {
    fn contract_type(&mut self, form: &Form, globals: &Globals) -> Type {
        read_type(&self.names, form, &mut self.diagnostics, |key, _| {
            let ty = globals.specs.get(key);
            self.external_specs
                .entry(key.to_owned())
                .or_insert_with(|| ty.cloned());
            ty.cloned()
        })
    }

    pub(super) fn read_function(&mut self, xs: &[Form], span: Span, globals: &Globals) {
        let Some(symbol) = xs.get(1).and_then(Form::symbol) else {
            self.error("PNX9001", span, "defn requires a function name");
            return;
        };
        let Some(name) = self.definition_name(symbol, xs[1].span, "defn") else {
            return;
        };
        if self.literals.contains_key(&name) {
            self.error("PNX2790", xs[1].span, format!("Duplicate var {name}"));
            return;
        }
        let mut index = 2;
        if xs
            .get(index)
            .is_some_and(|x| matches!(x.kind, Kind::String(_)))
        {
            index += 1;
        }
        let attrs = if let Some(attrs) = xs.get(index).and_then(Form::map) {
            index += 1;
            attrs
        } else {
            &[]
        };
        let mut arms = &xs[index..];
        let trailing_attrs = if arms.first().and_then(Form::vector).is_none()
            && let Some(attrs) = arms.last().and_then(Form::map)
        {
            arms = &arms[..arms.len() - 1];
            attrs
        } else {
            &[]
        };
        let attribute = |key| {
            trailing_attrs
                .as_chunks::<2>()
                .0
                .iter()
                .chain(attrs.as_chunks::<2>().0)
                .find(|[name, _]| name.keyword() == Some(key))
                .map(|[_, value]| value)
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
        let signature = attribute(":t/sig").or_else(|| xs[1].metadata(":t/sig"));
        let inferred = signature.is_none();
        if inferred && !private {
            self.error(
                "PNX9002",
                span,
                format!("Public function {name} needs a :t/sig annotation"),
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
        let signatures = if let Some(signature) = signature {
            if !matches!(signature.without_metadata().kind, Kind::Quote(_)) {
                self.error("PNX9001", signature.span, ":t/sig must be quoted data");
            }
            let Some(signatures) = self.parse_signatures(signature, globals) else {
                return;
            };
            if !arities
                .keys()
                .copied()
                .eq(signatures.iter().map(Signature::arity))
            {
                self.error(
                    "PNX2554",
                    signature.span,
                    "Signature arities must match the function's parameters",
                );
                return;
            }
            signatures
        } else {
            arities
                .keys()
                .map(|arity| Signature {
                    args: vec![Type::Unknown; arity.required()],
                    rest: matches!(arity, Arity::Variadic(_)).then_some(Type::Unknown),
                    ret: Type::Unknown,
                })
                .collect()
        };
        let function = Function {
            name: name.clone(),
            private,
            inferred,
            arities,
            signatures: signatures.into(),
            span,
        };
        self.names.refers.remove(symbol);
        if self.functions.insert(name.clone(), function).is_some() {
            self.error("PNX2790", span, format!("Duplicate function {name}"));
        }
    }

    pub(super) fn parse_signatures(
        &mut self,
        form: &Form,
        globals: &Globals,
    ) -> Option<Vec<Signature>> {
        let types = form.unquoted().vector();
        let Some(arms) =
            types.filter(|arms| !arms.is_empty() && arms.iter().all(|arm| arm.vector().is_some()))
        else {
            return self
                .parse_signature(form, globals)
                .map(|signature| vec![signature]);
        };
        let mut signatures = BTreeMap::new();
        for arm in arms {
            let signature = self.parse_signature(arm, globals)?;
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

    pub(super) fn parse_signature(&mut self, form: &Form, globals: &Globals) -> Option<Signature> {
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
            .map(|t| self.contract_type(t, globals))
            .collect();
        let ret = self.contract_type(types.last().unwrap(), globals);
        let rest = rest.map(|form| self.contract_type(form, globals));
        Some(Signature { args, rest, ret })
    }
}
