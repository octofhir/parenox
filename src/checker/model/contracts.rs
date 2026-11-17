//! Spec declarations and annotated function signatures.
use std::collections::BTreeMap;

use super::{Function, Globals, Model, Signature};
use crate::reader::{Form, Kind, Span};
use crate::types::{ExtraFields, Field, MapType, Type};

impl Model {
    pub(super) fn spec(
        &mut self,
        key: &str,
        visiting: &mut Vec<String>,
        span: Span,
        globals: &Globals,
    ) -> Type {
        if let Some(ty) = self.specs.get(key) {
            return ty.clone();
        }
        if !self.raw_specs.contains_key(key)
            && let Some(ty) = globals.specs.get(key)
        {
            return ty.clone();
        }
        if visiting.iter().any(|k| k == key) {
            self.error(
                "PNX9001",
                span,
                format!("Recursive spec {key} is not supported yet"),
            );
            return Type::Unknown;
        }
        if visiting.len() >= 128 {
            self.error("PNX9001", span, "Spec expansion limit (128) exceeded");
            return Type::Unknown;
        }
        let Some(form) = self.raw_specs.get(key).cloned() else {
            self.error("PNX9001", span, format!("No declaration for spec {key}"));
            return Type::Unknown;
        };
        visiting.push(key.to_string());
        let ty = self.parse_type(&form, visiting, globals);
        visiting.pop();
        self.specs.insert(key.to_string(), ty.clone());
        ty
    }

    fn parse_type(&mut self, form: &Form, visiting: &mut Vec<String>, globals: &Globals) -> Type {
        match &form.without_metadata().kind {
            Kind::Symbol(s) => match self.resolve(s).as_str() {
                "clojure.core/string?" => Type::String,
                "clojure.core/char?" => Type::Character,
                "clojure.core/number?" => Type::Number,
                "clojure.core/boolean?" => Type::Boolean,
                "clojure.core/keyword?" => Type::Keyword,
                "clojure.core/nil?" => Type::Nil,
                "clojure.core/map?" => Type::Map(MapType {
                    fields: BTreeMap::new(),
                    extra: ExtraFields::Open,
                }),
                _ => {
                    self.error("PNX9001", form.span, format!("Unsupported type {s}"));
                    Type::Unknown
                }
            },
            Kind::Keyword(key) => self.spec(&self.key(key), visiting, form.span, globals),
            Kind::Set(xs) => {
                let mut types = Vec::new();
                for x in xs {
                    types.push(match &x.kind {
                        Kind::String(s) => Type::Str(s.clone()),
                        Kind::Character(c) => Type::Char(*c),
                        Kind::Keyword(k) => Type::Key(self.key(k)),
                        Kind::Bool(b) => Type::Bool(*b),
                        Kind::Nil => Type::Nil,
                        _ => {
                            self.error("PNX9001", x.span, "Unsupported literal union member");
                            Type::Unknown
                        }
                    });
                }
                Type::union(types)
            }
            Kind::List(xs) => {
                let op = xs.first().and_then(Form::symbol).map(|s| self.resolve(s));
                match op.as_deref() {
                    Some("clojure.spec.alpha/nilable") if xs.len() == 2 => {
                        self.parse_type(&xs[1], visiting, globals).nilable()
                    }
                    Some("clojure.spec.alpha/or") if xs.len() >= 3 && xs.len() % 2 == 1 => {
                        let mut types = Vec::new();
                        for pair in xs[1..].as_chunks::<2>().0 {
                            if pair[0].keyword().is_none() {
                                self.error("PNX9001", pair[0].span, "s/or requires keyword tags");
                            }
                            types.push(self.parse_type(&pair[1], visiting, globals));
                        }
                        Type::union(types)
                    }
                    Some("clojure.spec.alpha/map-of") if xs.len() == 3 => {
                        let key = self.parse_type(&xs[1], visiting, globals);
                        let value = self.parse_type(&xs[2], visiting, globals);
                        Type::Map(MapType::dictionary(key, value))
                    }
                    Some("clojure.spec.alpha/keys") if xs.len() % 2 == 1 => {
                        let mut fields = BTreeMap::new();
                        for pair in xs[1..].as_chunks::<2>().0 {
                            let directive = pair[0].keyword().unwrap_or("");
                            if !matches!(directive, ":req" | ":opt" | ":req-un" | ":opt-un") {
                                self.error("PNX9001", pair[0].span, "Unsupported s/keys directive");
                                continue;
                            }
                            let Some(keys) = pair[1].vector() else {
                                self.error("PNX9001", pair[1].span, "s/keys requires key vectors");
                                continue;
                            };
                            for key in keys {
                                let Some(k) = key.keyword() else {
                                    self.error("PNX9001", key.span, "Expected a spec keyword");
                                    continue;
                                };
                                let qualified = self.key(k);
                                if !qualified.contains('/') {
                                    self.error(
                                        "PNX9001",
                                        key.span,
                                        "s/keys needs qualified spec keys",
                                    );
                                }
                                let ty = self.spec(&qualified, visiting, key.span, globals);
                                let field_key = if directive.ends_with("-un") {
                                    format!(":{}", qualified.rsplit('/').next().unwrap())
                                } else {
                                    qualified
                                };
                                let required = directive.starts_with(":req");
                                if fields
                                    .insert(field_key.clone(), Field { ty, required })
                                    .is_some()
                                {
                                    self.error(
                                        "PNX9001",
                                        key.span,
                                        format!("Repeated field {field_key}"),
                                    );
                                }
                            }
                        }
                        Type::Map(MapType {
                            fields,
                            extra: ExtraFields::Open,
                        })
                    }
                    Some("clojure.spec.alpha/coll-of")
                        if xs.len() == 4
                            && xs[2].keyword() == Some(":kind")
                            && matches!(
                                xs[3].symbol().map(|s| self.resolve(s)).as_deref(),
                                Some("clojure.core/vector?" | "clojure.core/set?")
                            ) =>
                    {
                        let element = Box::new(self.parse_type(&xs[1], visiting, globals));
                        if xs[3].symbol().map(|s| self.resolve(s)).as_deref()
                            == Some("clojure.core/set?")
                        {
                            Type::Set(element)
                        } else {
                            Type::Vector(element)
                        }
                    }
                    _ => {
                        self.error("PNX9001", form.span, "Unsupported type form");
                        Type::Unknown
                    }
                }
            }
            _ => {
                self.error("PNX9001", form.span, "Expected a type declaration");
                Type::Unknown
            }
        }
    }

    pub(super) fn read_function(&mut self, xs: &[Form], span: Span, globals: &Globals) {
        let Some(name) = xs.get(1).and_then(Form::symbol) else {
            self.error("PNX9001", span, "defn requires a function name");
            return;
        };
        let name = self.function_name(name);
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
            .map(|t| self.parse_type(t, &mut Vec::new(), globals))
            .collect();
        let ret = self.parse_type(types.last().unwrap(), &mut Vec::new(), globals);
        Some(Signature { args, ret })
    }
}
