//! Annotated signatures and function declarations.
use super::{Function, Globals, Model, Signature, type_forms::read_type};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

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
        if symbol.contains('/') && symbol != "/" {
            self.error(
                "PNX1001",
                xs[1].span,
                "A defn name must be an unqualified symbol",
            );
            return;
        }
        if let Some(referred) = self.names.refers.get(symbol)
            && !referred.starts_with("clojure.core/")
        {
            self.error(
                "PNX2790",
                xs[1].span,
                format!("{symbol} already refers to {referred}"),
            );
            return;
        }
        let name = format!("{}/{symbol}", self.names.namespace);
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
        let default_private = xs[0]
            .symbol()
            .is_some_and(|s| self.resolve(s) == "clojure.core/defn-");
        let private_form = attrs
            .as_chunks::<2>()
            .0
            .iter()
            .find(|p| p[0].keyword() == Some(":private"))
            .map(|p| &p[1])
            .or_else(|| {
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
        let signature = attrs
            .as_chunks::<2>()
            .0
            .iter()
            .find(|p| p[0].keyword() == Some(":t/sig"))
            .map(|p| &p[1])
            .or_else(|| xs[1].metadata(":t/sig"));
        let inferred = signature.is_none();
        if inferred && !private {
            self.error(
                "PNX9002",
                span,
                format!("Public function {name} needs a :t/sig annotation"),
            );
            return;
        }
        let signature = if let Some(signature) = signature {
            if !matches!(signature.without_metadata().kind, Kind::Quote(_)) {
                self.error("PNX9001", signature.span, ":t/sig must be quoted data");
            }
            let Some(signature) = self.parse_signature(signature, globals) else {
                return;
            };
            Some(signature)
        } else {
            None
        };
        let Some(params) = xs.get(index).and_then(Form::vector) else {
            self.error("PNX9001", span, "Only single-arity defn is supported");
            return;
        };
        if params.iter().any(|p| p.symbol() == Some("&")) {
            self.error(
                "PNX9001",
                xs[index].span,
                "Variadic definitions are not supported yet",
            );
            return;
        }
        let signature = signature.unwrap_or_else(|| Signature {
            args: vec![Type::Unknown; params.len()],
            ret: Type::Unknown,
        });
        if params.len() != signature.args.len() || xs.len() <= index + 1 {
            self.error(
                "PNX2554",
                span,
                "Signature arity must match parameters; body is required",
            );
            return;
        }
        let function = Function {
            name: name.clone(),
            private,
            inferred,
            params: params.to_vec(),
            args: signature.args,
            ret: signature.ret,
            body: xs[index + 1..].to_vec(),
            span,
        };
        self.names.refers.remove(symbol);
        if self.functions.insert(name.clone(), function).is_some() {
            self.error("PNX2790", span, format!("Duplicate function {name}"));
        }
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
        let args = types[..types.len() - 2]
            .iter()
            .map(|t| self.contract_type(t, globals))
            .collect();
        let ret = self.contract_type(types.last().unwrap(), globals);
        Some(Signature { args, ret })
    }
}
