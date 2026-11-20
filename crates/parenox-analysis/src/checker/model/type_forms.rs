//! The supported type syntax shared by source contracts and spec declarations.
mod conjunction;
mod conversion;
mod function_specs;
pub(super) use conversion::{
    enum_conversion, inverted_literal, named_tagged_branches, projected_collection_child, resolves,
    source_conversion, string_keyed_map_child, structured_conversion, tagged_or, tagged_projection,
    tagged_projection_branches,
};
pub(super) use function_specs::read_function_spec;

use std::collections::{BTreeMap, BTreeSet};

use super::Names;
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::{ExtraFields, Field, IntegerType, MAX_TUPLE_ITEMS, MapType, Parameters, Type};

pub(super) fn read_type(
    names: &Names,
    form: &Form,
    source: &str,
    diagnostics: &mut Vec<Diagnostic>,
    lookup: impl FnMut(&str, Span) -> Option<Type>,
) -> Type {
    TypeReader {
        names,
        source,
        parameters: None,
        literal_shapes: false,
        unchecked_inputs: None,
        diagnostics,
        lookup,
    }
    .parse(form)
}

pub(super) fn read_type_with_parameters(
    names: &Names,
    form: &Form,
    source: &str,
    parameters: Option<&Parameters>,
    unchecked_inputs: Option<&BTreeSet<String>>,
    diagnostics: &mut Vec<Diagnostic>,
    lookup: impl FnMut(&str, Span) -> Option<Type>,
) -> Type {
    TypeReader {
        names,
        source,
        parameters,
        literal_shapes: true,
        unchecked_inputs,
        diagnostics,
        lookup,
    }
    .parse(form)
}

struct TypeReader<'a, F> {
    names: &'a Names,
    source: &'a str,
    parameters: Option<&'a Parameters>,
    literal_shapes: bool,
    unchecked_inputs: Option<&'a BTreeSet<String>>,
    diagnostics: &'a mut Vec<Diagnostic>,
    lookup: F,
}

impl<F: FnMut(&str, Span) -> Option<Type>> TypeReader<'_, F> {
    fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    fn lookup_spec(&mut self, key: &str, span: Span) -> Option<Type> {
        let ty = (self.lookup)(key, span)?;
        if self.unchecked_inputs.is_some_and(|keys| keys.contains(key)) {
            self.diagnostics.push(Diagnostic::analysis_gap(
                "PNX9001",
                span,
                format!("Spec {key} has an unchecked input type in this function contract"),
            ));
        }
        Some(ty)
    }

    fn spec(&mut self, key: &str, span: Span) -> Type {
        self.lookup_spec(key, span).unwrap_or_else(|| {
            self.error("PNX9001", span, format!("No declaration for spec {key}"));
            Type::Unknown
        })
    }

    fn map_fields(
        &mut self,
        forms: &[Form],
        fields: &mut BTreeMap<Type, Field>,
        required: bool,
    ) -> Option<()> {
        for [key, ty] in forms.as_chunks::<2>().0 {
            let name = match &key.without_metadata().kind {
                Kind::Keyword(key) => Type::Key(self.names.key(key)),
                Kind::String(key) => Type::Str(key.clone()),
                _ => {
                    self.error(
                        "PNX9001",
                        key.span,
                        "A literal map type requires literal keyword or string keys",
                    );
                    return None;
                }
            };
            if fields
                .insert(name, Field::new(self.parse(ty), required))
                .is_some()
            {
                self.error("PNX9001", key.span, "Duplicate key in a literal map type");
                return None;
            }
        }
        Some(())
    }

    fn map_shape(&mut self, form: &Form, args: &[Form]) -> Option<MapType> {
        if let [options] = args
            && let Some(options) = options.map()
        {
            let mut map = MapType {
                fields: BTreeMap::new(),
                extra: ExtraFields::Closed,
            };
            let mut seen = std::collections::BTreeSet::new();
            for [key, value] in options.as_chunks::<2>().0 {
                let option = key.keyword();
                if !seen.insert(option) {
                    self.error("PNX9001", key.span, "Duplicate t/map option");
                    return None;
                }
                match (option, value.map(), value.keyword()) {
                    (Some(":req" | ":opt"), Some(fields), _) => {
                        self.map_fields(fields, &mut map.fields, option == Some(":req"))?;
                    }
                    (Some(":extra"), _, Some(":open")) => map.extra = ExtraFields::Open,
                    (Some(":extra"), _, Some(":closed")) => {}
                    _ => {
                        self.error(
                            "PNX9001",
                            key.span,
                            "t/map expects :req and :opt field maps and :extra :open or :closed",
                        );
                        return None;
                    }
                }
            }
            Some(map)
        } else {
            self.error(
                "PNX9001",
                form.span,
                "Expected (t/map {:req {...} :opt {...} :extra :open})",
            );
            None
        }
    }

    fn parse(&mut self, form: &Form) -> Type {
        if let Some(symbol) = form.symbol()
            && let Some(parameter) = self
                .parameters
                .and_then(|binders| binders.iter().find(|parameter| parameter.name == symbol))
        {
            return Type::Parameter(parameter.clone());
        }
        match &form.without_metadata().kind {
            Kind::Symbol(s) => predicate_type(self.names, s).unwrap_or_else(|| {
                if self.names.spec_core_shadows.contains(s) {
                    self.diagnostics.push(Diagnostic::analysis_gap(
                        "PNX9001",
                        form.span,
                        format!("Source predicate {s} is unchecked"),
                    ));
                } else {
                    self.error("PNX9001", form.span, format!("Unsupported type {s}"));
                }
                Type::Unknown
            }),
            Kind::Keyword(key) => self.spec(&self.names.key(key), form.span),
            Kind::Map(forms) if self.literal_shapes => {
                let mut fields = BTreeMap::new();
                self.map_fields(forms, &mut fields, true)
                    .map_or(Type::Unknown, |()| {
                        Type::Map(MapType {
                            fields,
                            extra: ExtraFields::Closed,
                        })
                    })
            }
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
                if self.literal_shapes && xs.first().and_then(Form::symbol) == Some("t/map") {
                    return self
                        .map_shape(form, &xs[1..])
                        .map_or(Type::Unknown, Type::Map);
                }
                match op.as_deref() {
                    Some("clojure.spec.alpha/fspec") => self
                        .function_spec(&xs[1..], form.span, "s/fspec")
                        .map_or(Type::Unknown, |signature| {
                            Type::Function([signature].into())
                        }),
                    Some("clojure.spec.alpha/nilable") if xs.len() == 2 => {
                        self.parse(&xs[1]).nilable()
                    }
                    Some("clojure.spec.alpha/with-gen") if xs.len() == 3 => self.parse(&xs[1]),
                    Some("clojure.spec.alpha/and") => self.conjunction(form, &xs[1..]),
                    Some("clojure.spec.alpha/conformer") if matches!(xs.len(), 2 | 3) => {
                        self.unchecked_conformer(form)
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
                    Some("clojure.spec.alpha/map-of")
                        if xs.len() >= 3 && generation_options(&xs[3..]) =>
                    {
                        let key = self.parse(&xs[1]);
                        let value = self.parse(&xs[2]);
                        Type::Map(MapType::dictionary(key, value))
                    }
                    Some("clojure.spec.alpha/keys") if xs.len() % 2 == 1 => {
                        use std::collections::btree_map::Entry;

                        let mut fields: BTreeMap<String, (String, Field)> = BTreeMap::new();
                        let mut directives = BTreeSet::new();
                        let mut supported = true;
                        for pair in xs[1..].as_chunks::<2>().0 {
                            let directive = pair[0].keyword().unwrap_or("");
                            if !matches!(directive, ":req" | ":opt" | ":req-un" | ":opt-un") {
                                self.error("PNX9001", pair[0].span, "Unsupported s/keys directive");
                                supported = false;
                                continue;
                            }
                            if !directives.insert(directive) {
                                self.error("PNX9001", pair[0].span, "Repeated s/keys directive");
                                supported = false;
                            }
                            let Some(keys) = pair[1].vector() else {
                                self.error("PNX9001", pair[1].span, "s/keys requires key vectors");
                                supported = false;
                                continue;
                            };
                            for key in keys {
                                let Some(k) = key.keyword() else {
                                    self.error("PNX9001", key.span, "Expected a spec keyword");
                                    supported = false;
                                    continue;
                                };
                                let qualified = self.names.key(k);
                                let Some(field_key) = spec_field_key(&qualified, directive) else {
                                    self.error(
                                        "PNX9001",
                                        key.span,
                                        "s/keys needs qualified spec keys",
                                    );
                                    supported = false;
                                    continue;
                                };
                                let required = directive.starts_with(":req");
                                // Look up each source occurrence so navigation can
                                // resolve a repeated reference at its own span.
                                let value = self.lookup_spec(&qualified, key.span);
                                match fields.entry(field_key) {
                                    Entry::Occupied(mut entry) => {
                                        let (spec, field) = entry.get_mut();
                                        if *spec == qualified {
                                            field.required |= required;
                                        } else {
                                            self.error(
                                                "PNX9001",
                                                key.span,
                                                format!("Repeated field {}", entry.key()),
                                            );
                                            supported = false;
                                        }
                                    }
                                    Entry::Vacant(entry) => {
                                        let field = value.map_or_else(
                                            || {
                                                self.diagnostics.push(Diagnostic::analysis_gap(
                                                    "PNX9001",
                                                    key.span,
                                                    format!("No value spec for {qualified}; s/keys checks presence only"),
                                                ));
                                                Field::presence_only(required)
                                            },
                                            |ty| Field::new(ty, required),
                                        );
                                        entry.insert((qualified, field));
                                    }
                                }
                            }
                        }
                        if supported {
                            Type::Map(MapType {
                                fields: fields
                                    .into_iter()
                                    .map(|(name, (_, field))| (Type::Key(name), field))
                                    .collect(),
                                extra: ExtraFields::Open,
                            })
                        } else {
                            // Repeated options or colliding unqualified keys
                            // can alter both presence and the winning predicate.
                            Type::Unknown
                        }
                    }
                    Some("clojure.spec.alpha/coll-of") if xs.len() >= 2 => {
                        self.collection(form, &xs[1], &xs[2..])
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

    fn collection(&mut self, form: &Form, element: &Form, options: &[Form]) -> Type {
        let (pairs, extra) = options.as_chunks::<2>();
        if !extra.is_empty()
            || pairs
                .iter()
                .any(|[key, _]| !matches!(key.keyword(), Some(":kind" | ":gen-max")))
        {
            self.error("PNX9001", form.span, "Unsupported type form");
            return Type::Unknown;
        }
        let Some([_, kind]) = pairs
            .iter()
            .rfind(|[key, _]| key.keyword() == Some(":kind"))
        else {
            let element = self.parse(element);
            if collection_element(&element) {
                return Type::SpecCollection(Box::new(element));
            }
            self.diagnostics.push(Diagnostic::analysis_gap(
                "PNX9001",
                form.span,
                "A collection spec without :kind requires any?, strings, or modeled map shapes",
            ));
            return Type::Unknown;
        };
        match kind
            .symbol()
            .filter(|symbol| !self.names.spec_core_shadows.contains(*symbol))
            .map(|symbol| self.names.resolve_parts(symbol))
        {
            Some(("clojure.core", "set?")) => Type::Set(Box::new(self.parse(element))),
            Some(("clojure.core", "vector?")) => Type::Vector(Box::new(self.parse(element))),
            Some(("clojure.core", "seq?")) => {
                Type::union([Type::empty_seq(), Type::Seq(Box::new(self.parse(element)))])
            }
            Some(("clojure.core", "list?")) => Type::union([
                Type::empty_list(),
                Type::List(Box::new(self.parse(element))),
            ]),
            Some(("clojure.core", "map?")) => {
                match &self.parse(element) {
                    // Each map member is an entry, even if its key or value
                    // happens to be the spec-invalid keyword.
                    Type::SpecValue | Type::SpecTruthy => core_predicate_type("map?").unwrap(),
                    Type::Tuple(items) if let [key, value] = items.as_ref() => {
                        Type::Map(MapType::dictionary(key.clone(), value.clone()))
                    }
                    _ => {
                        self.diagnostics.push(Diagnostic::analysis_gap(
                            "PNX9001",
                            element.span,
                            "A map collection spec requires a modeled entry predicate",
                        ));
                        Type::Unknown
                    }
                }
            }
            _ => {
                self.error("PNX9001", kind.span, "Unsupported collection kind");
                Type::Unknown
            }
        }
    }
}

fn collection_element(element: &Type) -> bool {
    match element {
        Type::SpecValue | Type::String | Type::Str(_) => true,
        Type::Map(map) => !map.has_unknown_value_constraint(),
        Type::Union(parts) => parts.iter().all(collection_element),
        _ => false,
    }
}

fn spec_field_key(qualified: &str, directive: &str) -> Option<String> {
    let (_, local) = qualified.split_once('/')?;
    Some(if directive.ends_with("-un") {
        format!(":{local}")
    } else {
        qualified.to_owned()
    })
}

fn predicate_type(names: &Names, symbol: &str) -> Option<Type> {
    if symbol == "java.util.regex.Pattern" {
        return Some(Type::Pattern);
    }
    let (namespace, local) = names.resolve_parts(symbol);
    if namespace == "clojure.core" && !names.spec_core_shadows.contains(symbol) {
        core_predicate_type(local)
    } else {
        None
    }
}

pub(super) fn core_predicate_type(name: &str) -> Option<Type> {
    Some(match name {
        "string?" => Type::String,
        "char?" => Type::Character,
        "number?" => Type::Number,
        "integer?" => Type::Integral(IntegerType::ANY),
        "int?" => Type::Integral(IntegerType::FIXED),
        "boolean?" => Type::Boolean,
        "any?" => Type::SpecValue,
        "identity" => Type::SpecTruthy,
        "keyword?" => Type::Keyword,
        "symbol?" => Type::Symbol,
        "simple-symbol?" => Type::SimpleSymbol,
        "qualified-symbol?" => Type::QualifiedSymbol,
        "nil?" => Type::Nil,
        "map?" => Type::Map(MapType {
            fields: BTreeMap::new(),
            extra: ExtraFields::Open,
        }),
        _ => return None,
    })
}

fn generation_options(options: &[Form]) -> bool {
    let (pairs, extra) = options.as_chunks::<2>();
    extra.is_empty()
        && pairs
            .iter()
            .all(|[key, _]| key.keyword() == Some(":gen-max"))
}
