//! The supported type syntax shared by source contracts and spec declarations.
use std::collections::BTreeMap;

use super::Names;
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::{ExtraFields, Field, MAX_TUPLE_ITEMS, MapType, Type};

pub(super) fn read_type(
    names: &Names,
    form: &Form,
    diagnostics: &mut Vec<Diagnostic>,
    lookup: impl FnMut(&str, Span) -> Option<Type>,
) -> Type {
    TypeReader {
        names,
        diagnostics,
        lookup,
    }
    .parse(form)
}

struct TypeReader<'a, F> {
    names: &'a Names,
    diagnostics: &'a mut Vec<Diagnostic>,
    lookup: F,
}

impl<F: FnMut(&str, Span) -> Option<Type>> TypeReader<'_, F> {
    fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    fn spec(&mut self, key: &str, span: Span) -> Type {
        (self.lookup)(key, span).unwrap_or_else(|| {
            self.error("PNX9001", span, format!("No declaration for spec {key}"));
            Type::Unknown
        })
    }

    fn parse(&mut self, form: &Form) -> Type {
        match &form.without_metadata().kind {
            Kind::Symbol(s) => match self.names.resolve(s).as_str() {
                "clojure.core/string?" => Type::String,
                "clojure.core/char?" => Type::Character,
                "clojure.core/number?" => Type::Number,
                "clojure.core/boolean?" => Type::Boolean,
                "clojure.core/keyword?" => Type::Keyword,
                "clojure.core/symbol?" => Type::Symbol,
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
            Kind::Keyword(key) => self.spec(&self.names.key(key), form.span),
            Kind::Set(xs) => {
                let mut types = Vec::new();
                for x in xs {
                    types.push(match &x.kind {
                        Kind::String(s) => Type::Str(s.clone()),
                        Kind::Character(c) => Type::Char(*c),
                        Kind::Keyword(k) => Type::Key(self.names.key(k)),
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
                let op = xs
                    .first()
                    .and_then(Form::symbol)
                    .map(|s| self.names.resolve(s));
                match op.as_deref() {
                    Some("clojure.spec.alpha/nilable") if xs.len() == 2 => {
                        self.parse(&xs[1]).nilable()
                    }
                    Some("clojure.spec.alpha/or") if xs.len() >= 3 && xs.len() % 2 == 1 => {
                        let mut types = Vec::new();
                        for pair in xs[1..].as_chunks::<2>().0 {
                            if pair[0].keyword().is_none() {
                                self.error("PNX9001", pair[0].span, "s/or requires keyword tags");
                            }
                            types.push(self.parse(&pair[1]));
                        }
                        Type::union(types)
                    }
                    Some("clojure.spec.alpha/tuple")
                        if (2..=MAX_TUPLE_ITEMS + 1).contains(&xs.len()) =>
                    {
                        Type::Tuple(xs[1..].iter().map(|item| self.parse(item)).collect())
                    }
                    Some("clojure.spec.alpha/map-of") if xs.len() == 3 => {
                        let key = self.parse(&xs[1]);
                        let value = self.parse(&xs[2]);
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
                                let qualified = self.names.key(k);
                                if !qualified.contains('/') {
                                    self.error(
                                        "PNX9001",
                                        key.span,
                                        "s/keys needs qualified spec keys",
                                    );
                                }
                                let ty = self.spec(&qualified, key.span);
                                let field_key = if directive.ends_with("-un") {
                                    format!(":{}", qualified.rsplit('/').next().unwrap())
                                } else {
                                    qualified
                                };
                                let required = directive.starts_with(":req");
                                if fields
                                    .insert(Type::Key(field_key.clone()), Field { ty, required })
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
                        if xs.len() == 4 && xs[2].keyword() == Some(":kind") =>
                    {
                        let kind = xs[3].symbol().map(|s| self.names.resolve(s));
                        match kind.as_deref() {
                            Some("clojure.core/set?") => Type::Set(Box::new(self.parse(&xs[1]))),
                            Some("clojure.core/vector?") => {
                                Type::Vector(Box::new(self.parse(&xs[1])))
                            }
                            Some("clojure.core/seq?") => Type::union([
                                Type::empty_seq(),
                                Type::Seq(Box::new(self.parse(&xs[1]))),
                            ]),
                            _ => {
                                self.error("PNX9001", xs[3].span, "Unsupported collection kind");
                                Type::Unknown
                            }
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
}
