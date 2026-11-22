//! Standalone, exact spec conversions that can be exchanged as externs.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::sync::Arc;

use super::{
    ExternCache, ExternalDeclaration, Externs, Globals, Model, Names, SnapshotWriter, bundles,
};
use crate::checker::model::type_forms::{
    literal_integer_enum, live_lookup_projection, source_conversion, string_keyed_map_child,
};
use crate::checker::model::{
    GuardedMapProjection, SpecConversion, SpecProvider, StructuredConversion,
    specs::{EnumKind, ProjectedFields, ProjectedValue, RegistrationSource},
};
use crate::reader::{Form, Span};
use crate::types::{ExtraFields, IntegerType, MapType, Parameter, SPEC_INVALID, Type};
use crate::{ContractKind, ContractOrigin, Diagnostic};

#[derive(Clone, PartialEq, Eq)]
pub(in crate::checker::model) struct SpecExtern {
    pub origin: ContractOrigin,
    pub conversion: SpecConversion,
    pub registration: Arc<RegistrationSource>,
    pub alias: Option<Arc<str>>,
    dependencies: BTreeSet<String>,
    depth: u8,
}

#[derive(PartialEq, Eq)]
pub(in crate::checker::model) struct PendingSpec {
    pub origin: ContractOrigin,
    declaration: Option<PendingDeclaration>,
}

#[derive(PartialEq, Eq)]
struct PendingDeclaration {
    namespace: String,
    body: PendingBody,
    span: Span,
}

#[derive(PartialEq, Eq)]
enum PendingBody {
    Keys(bundles::KeyFields),
    Alias(Arc<str>),
    Collection(Arc<str>),
    StringKeyedMap(Arc<str>),
}

impl PendingSpec {
    pub(super) fn invalid(origin: ContractOrigin) -> ExternalDeclaration {
        ExternalDeclaration::PendingSpec(Arc::new(Self {
            origin,
            declaration: None,
        }))
    }
}

fn spec_forms(declaration: &Form) -> Option<(&Form, &str)> {
    let [first, a, second, b] = declaration.map()? else {
        return None;
    };
    let (body, namespace) = match (first.keyword(), second.keyword()) {
        (Some(":spec"), Some(":ns")) => (a, b),
        (Some(":ns"), Some(":spec")) => (b, a),
        _ => return None,
    };
    Some((body, namespace.symbol().filter(|name| !name.contains('/'))?))
}

impl SpecConversion {
    pub(super) fn is_extern_leaf(&self) -> bool {
        match self {
            Self::CheckedInt32
            | Self::WrapField(_)
            | Self::VectorWrapField(_)
            | Self::MapOrWrapField(_)
            | Self::GuardedMap(_)
            | Self::LiveLookupMap(_)
            | Self::Enum {
                kind: EnumKind::FixedIntegerKeywords,
                ..
            }
            | Self::String => true,
            Self::IdentityDomain(domain) => literal_strings(domain).is_some(),
            _ => false,
        }
    }

    pub(super) fn extern_success_type(&self) -> Option<Type> {
        Some(match self {
            Self::CheckedInt32 => Type::Integral(IntegerType::FIXED_INT32),
            Self::WrapField(key) => Self::wrap_field(key, Type::Any),
            Self::VectorWrapField(key) => {
                Self::wrap_field(key, Type::Vector(Box::new(Type::Any))).nilable()
            }
            Self::MapOrWrapField(_) => Type::Map(MapType {
                fields: Default::default(),
                extra: ExtraFields::Open,
            }),
            Self::GuardedMap(projection) => projection.map_result(|_, _| Type::Any).nilable(),
            Self::LiveLookupMap(projection) => projection.map_result(),
            Self::String => Type::String,
            Self::Enum {
                entries,
                kind: EnumKind::FixedIntegerKeywords,
            } => Type::union(entries.iter().map(|(_, value)| value.clone())),
            Self::StringKeyedMap { .. } => {
                let (_, values) = self.portable_string_keyed_map()?;
                Type::Map(MapType::dictionary(
                    Type::String,
                    values.extern_success_type()?,
                ))
            }
            Self::Structured(_) => {
                if let Some((_, element)) = self.portable_collection() {
                    Type::Vector(Box::new(element.extern_success_type()?))
                } else {
                    self.bundle_success_type()?
                }
            }
            Self::IdentityDomain(domain) if literal_strings(domain).is_some() => (**domain).clone(),
            _ => return None,
        })
    }

    pub(crate) fn proposal_result(&self) -> Option<String> {
        let input = || {
            Type::Parameter(Arc::new(Parameter {
                name: "Input".into(),
                owner: String::new(),
                upper: None,
            }))
        };
        let collection = self.portable_collection().is_some();
        let result = match self {
            Self::WrapField(key) => Self::wrap_field(key, input()),
            Self::MapOrWrapField(key) => {
                let wrapped = Self::wrap_field(key, input());
                SnapshotWriter::new().ty(&wrapped).ok()?;
                return Some(format!(
                    "Input when vector? is false and map? is true, {wrapped} otherwise"
                ));
            }
            Self::Structured(_) if !collection => self.bundle_result()?,
            Self::Structured(_) | Self::StringKeyedMap { .. } => {
                Type::union([self.extern_success_type()?, Type::Key(SPEC_INVALID.into())])
            }
            Self::CheckedInt32 | Self::String | Self::IdentityDomain(_) | Self::Enum { .. } => {
                Type::union([self.extern_success_type()?, Type::Key(SPEC_INVALID.into())])
            }
            _ => self.extern_success_type()?,
        };
        SnapshotWriter::new().ty(&result).ok()?;
        Some(if collection {
            format!("{result} for a checked literal vector")
        } else if self.portable_string_keyed_map().is_some() || self.has_nested_collection() {
            format!("{result} for checked literal maps and nested vectors")
        } else {
            result.to_string()
        })
    }
}

fn literal_strings(domain: &Type) -> Option<&[Type]> {
    match domain {
        Type::Str(_) => Some(std::slice::from_ref(domain)),
        Type::Union(members) if members.iter().all(|ty| matches!(ty, Type::Str(_))) => {
            Some(members)
        }
        _ => None,
    }
}

impl SnapshotWriter {
    pub(crate) fn spec(
        &mut self,
        namespace: &str,
        conversion: &SpecConversion,
        alias: Option<&str>,
    ) -> Result<(), super::EncodeError> {
        self.nested(1, "{:spec ", "}", |out| {
            if let Some(alias) = alias {
                return Ok(write!(out, "{alias} :ns {namespace}")?);
            }
            match conversion {
                SpecConversion::CheckedInt32 => out.nested(
                    2,
                    "(clojure.spec.alpha/and clojure.core/integer? (clojure.spec.alpha/conformer clojure.core/int))",
                    "",
                    |_| Ok(()),
                ),
                SpecConversion::WrapField(key) => out.nested(
                    3,
                    "(clojure.spec.alpha/conformer (clojure.core/fn [input] {",
                    " input}))",
                    |out| out.key(key),
                ),
                SpecConversion::VectorWrapField(key) => out.nested(
                    4,
                    "(clojure.spec.alpha/conformer (clojure.core/fn [input] (clojure.core/when (clojure.core/vector? input) {",
                    " input})))",
                    |out| out.key(key),
                ),
                SpecConversion::MapOrWrapField(key) => out.nested(
                    4,
                    "(clojure.spec.alpha/conformer (clojure.core/fn [input] (clojure.core/cond (clojure.core/vector? input) {",
                    " input})))",
                    |out| {
                        out.key(key)?;
                        out.write_str(" input} (clojure.core/map? input) input :else {")?;
                        out.key(key)
                    },
                ),
                SpecConversion::GuardedMap(projection) => out.guarded_map(projection),
                SpecConversion::Enum { entries, kind: EnumKind::FixedIntegerKeywords } => {
                    out.nested(2, "(clojure.spec.alpha/and clojure.core/int? ", "))", |out| {
                        out.integer_enum_table(entries)?;
                        out.write_str(" (clojure.spec.alpha/conformer ")?;
                        out.integer_enum_table(entries)
                    })
                }
                SpecConversion::LiveLookupMap(projection) => out.nested(
                    2,
                    "(clojure.spec.alpha/conformer (clojure.core/fn [input] ",
                    "))",
                    |out| out.projection_fields(&projection.fields, Some(&projection.var)),
                ),
                SpecConversion::Structured(_) if let Some(fields) = conversion.bundle_fields() => {
                    out.keys(fields)
                }
                SpecConversion::Structured(_) if let Some((child, _)) = conversion.portable_collection() => {
                    out.nested(1, "(clojure.spec.alpha/coll-of ", ")", |out| Ok(out.write_str(child)?))
                }
                SpecConversion::StringKeyedMap { .. } if let Some((child, _)) = conversion.portable_string_keyed_map() => {
                    out.nested(
                        2,
                        "(clojure.spec.alpha/coll-of (clojure.spec.alpha/tuple clojure.core/string? ",
                        ") :kind clojure.core/map?)",
                        |out| Ok(out.write_str(child)?),
                    )
                }
                SpecConversion::String => Ok(out.write_str("clojure.core/string?")?),
                SpecConversion::IdentityDomain(domain) => {
                    let items = literal_strings(domain).ok_or(super::EncodeError::Unsupported(
                        "expected a finite set of strings",
                    ))?;
                    out.nested(1, "#{", "}", |out| {
                        for (index, key) in items.iter().enumerate() {
                            out.separator(index)?;
                            out.key(key)?;
                        }
                        Ok(())
                    })
                }
                _ => Err(super::EncodeError::Unsupported(
                    "this conversion has no standalone extern representation",
                )),
            }?;
            Ok(write!(out, " :ns {namespace}")?)
        })
    }

    fn integer_enum_table(&mut self, entries: &[(Type, Type)]) -> Result<(), super::EncodeError> {
        self.nested(1, "{", "}", |out| {
            for (index, (key, value)) in entries.iter().enumerate() {
                let Some(code) = key.exact_integer_value() else {
                    return Err(super::EncodeError::Unsupported(
                        "Expected a fixed integer key",
                    ));
                };
                out.separator(index)?;
                write!(out, "{code} ")?;
                out.key(value)?;
            }
            Ok(())
        })
    }

    fn guarded_map(&mut self, projection: &GuardedMapProjection) -> Result<(), super::EncodeError> {
        self.nested(
            3,
            "(clojure.spec.alpha/conformer (clojure.core/fn [input] (clojure.core/when ",
            ")))",
            |out| {
                out.nested(1, "(clojure.core/and", ")", |out| {
                    for key in &projection.guards {
                        out.write_char(' ')?;
                        out.projection_lookup(key)?;
                    }
                    Ok(())
                })?;
                out.write_char(' ')?;
                out.projection_fields(&projection.fields, None)
            },
        )
    }

    fn projection_lookup(&mut self, key: &Type) -> Result<(), super::EncodeError> {
        self.nested(1, "(", " input)", |out| out.key(key))
    }

    fn projection_fields(
        &mut self,
        fields: &ProjectedFields,
        live_var: Option<&str>,
    ) -> Result<(), super::EncodeError> {
        self.nested(1, "{", "}", |out| {
            for (index, (key, value)) in fields.iter().enumerate() {
                out.separator(index)?;
                out.key(key)?;
                out.write_char(' ')?;
                match value {
                    ProjectedValue::Lookup(key) => out.projection_lookup(key)?,
                    ProjectedValue::BooleanLookup(key) => out.nested(
                        2,
                        "(clojure.core/boolean (clojure.core/get input ",
                        " false))",
                        |out| out.key(key),
                    )?,
                    ProjectedValue::Map(fields) => out.projection_fields(fields, live_var)?,
                    ProjectedValue::Boolean(value) => write!(out, "{value}")?,
                    ProjectedValue::LiveFallback(key) => {
                        let var = live_var
                            .ok_or(super::EncodeError::Unsupported("Missing lookup Var"))?;
                        out.nested(2, "(clojure.core/or ", "))", |out| {
                            write!(
                                out,
                                "(clojure.core/get {var} input) (clojure.core/get {var} "
                            )?;
                            out.key(key)
                        })?;
                    }
                }
            }
            Ok(())
        })
    }
}

impl Model {
    pub(super) fn read_extern_spec(
        &mut self,
        name: &str,
        declaration: &Form,
        path: &str,
        source: &str,
    ) -> ExternalDeclaration {
        let names = Names::default();
        let origin = ContractOrigin {
            name: name.into(),
            kind: ContractKind::Extern,
            path: Some(path.into()),
        };
        let parsed = spec_forms(declaration);
        let is_keys = parsed.is_some_and(|(body, _)| bundles::is_keys(body));
        if let Some((body, namespace)) = parsed {
            let pending = if is_keys {
                bundles::KeyFields::parse(body).map(PendingBody::Keys)
            } else if let Some(child) = bundles::collection_child(body) {
                Some(PendingBody::Collection(child.into()))
            } else if let Some(child) =
                string_keyed_map_child(&names, body).and_then(bundles::spec_keyword)
            {
                Some(PendingBody::StringKeyedMap(child.into()))
            } else {
                bundles::spec_keyword(body).map(|target| PendingBody::Alias(target.into()))
            };
            if let Some(body) = pending {
                return ExternalDeclaration::PendingSpec(Arc::new(PendingSpec {
                    origin,
                    declaration: Some(PendingDeclaration {
                        namespace: namespace.into(),
                        body,
                        span: declaration.span,
                    }),
                }));
            }
            if let Some(conversion) = source_conversion(&names, body)
                .or_else(|| literal_integer_enum(&names, body, source))
                .or_else(|| live_lookup_projection(&names, namespace, body))
                .filter(|conversion| conversion.proposal_result().is_some())
            {
                return ExternalDeclaration::Spec(Arc::new(SpecExtern::new(
                    origin, namespace, conversion,
                )));
            }
        }
        self.error(
            "PNX9001",
            declaration.span,
            if is_keys {
                "Expected an s/keys extern with up to 32 qualified child references"
            } else {
                "Expected {:spec <supported standalone spec conversion> :ns <library namespace>}"
            },
        );
        PendingSpec::invalid(origin)
    }
}

impl SpecExtern {
    fn new(origin: ContractOrigin, namespace: &str, conversion: SpecConversion) -> Self {
        let registration = RegistrationSource {
            path: origin.path.clone().expect("extern has a path"),
            namespace: namespace.into(),
        };
        Self {
            origin,
            conversion,
            registration: Arc::new(registration),
            alias: None,
            dependencies: BTreeSet::new(),
            depth: 0,
        }
    }
}

impl PendingSpec {
    fn resolve(&self, table: &Externs) -> Option<ExternalDeclaration> {
        let declaration = self.declaration.as_ref()?;
        let namespace = &declaration.namespace;
        let (conversion, children, alias) = match &declaration.body {
            PendingBody::Keys(keys) => {
                let (conversion, children) = keys.resolve(namespace, |key| table.spec(key))?;
                (conversion, children, None)
            }
            PendingBody::Alias(target) => {
                let spec = table.spec(target)?;
                if spec.registration.namespace != *namespace {
                    return None;
                }
                (spec.conversion.clone(), vec![spec], Some(target.clone()))
            }
            PendingBody::Collection(target) => {
                let spec = table.spec(target)?;
                if spec.registration.namespace != *namespace {
                    return None;
                }
                let conversion =
                    SpecConversion::Structured(Arc::new(StructuredConversion::Collection {
                        element: spec.conversion.clone(),
                        registration: Some(target.to_string()),
                    }));
                (conversion, vec![spec], None)
            }
            PendingBody::StringKeyedMap(target) => {
                let spec = table.spec(target)?;
                if spec.registration.namespace != *namespace {
                    return None;
                }
                let conversion = SpecConversion::StringKeyedMap {
                    values: Arc::new(spec.conversion.clone()),
                    registration: target.to_string(),
                };
                (conversion, vec![spec], None)
            }
        };
        let depth = children.iter().map(|child| child.depth).max()? + 1;
        if depth > SpecConversion::MAX_EXTERN_DEPTH || conversion.proposal_result().is_none() {
            return None;
        }
        let mut dependencies = BTreeSet::new();
        for child in children {
            dependencies.insert(child.origin.name.clone());
            dependencies.extend(child.dependencies.iter().cloned());
            if dependencies.len() > SpecConversion::MAX_EXTERN_DEPENDENCIES {
                return None;
            }
        }
        let mut spec = SpecExtern::new(self.origin.clone(), namespace, conversion);
        spec.alias = alias;
        spec.dependencies = dependencies;
        spec.depth = depth;
        Some(ExternalDeclaration::Spec(Arc::new(spec)))
    }
}

pub(super) fn resolve_specs(
    mut table: Arc<Externs>,
    before: Option<&Externs>,
    diagnostics: &mut BTreeMap<String, Vec<Diagnostic>>,
) -> Arc<Externs> {
    if !table.has_specs {
        return table;
    }
    let mut pending = Vec::new();
    for (namespace, declarations) in &table.namespaces {
        for (name, declaration) in declarations.iter() {
            let ExternalDeclaration::PendingSpec(spec) = declaration else {
                continue;
            };
            pending.push((namespace.clone(), name.clone(), spec.clone()));
        }
    }
    if pending.is_empty() {
        return table;
    }
    let resolved = Arc::make_mut(&mut table);
    for _ in 0..SpecConversion::MAX_EXTERN_DEPTH {
        let remaining = pending.len();
        pending.retain(|(namespace, name, spec)| {
            let Some(declaration) = spec.resolve(resolved) else {
                return true;
            };
            Arc::make_mut(resolved.namespaces.get_mut(namespace).unwrap())
                .insert(name.clone(), declaration);
            false
        });
        if pending.is_empty() || pending.len() == remaining {
            break;
        }
    }
    for (namespace, name, spec) in pending {
        Arc::make_mut(resolved.namespaces.get_mut(&namespace).unwrap()).remove(&name);
        if let Some(declaration) = &spec.declaration {
            let message = match declaration.body {
                PendingBody::Keys(_) => format!(
                    "An s/keys extern needs supported direct child specs with the same :ns ({}) in a bounded acyclic graph",
                    declaration.namespace
                ),
                PendingBody::Alias(ref target) => format!(
                    "An extern spec alias needs a supported target {target} with the same :ns ({}) in a bounded acyclic graph",
                    declaration.namespace
                ),
                PendingBody::Collection(ref target) => format!(
                    "An s/coll-of extern needs a supported map element spec {target} with the same :ns ({}) in a bounded acyclic graph",
                    declaration.namespace
                ),
                PendingBody::StringKeyedMap(ref target) => format!(
                    "An s/coll-of map extern needs a supported named collection {target} with the same :ns ({}) in a bounded acyclic graph",
                    declaration.namespace
                ),
            };
            let mut diagnostic = Diagnostic::analysis_gap("PNX9001", declaration.span, message);
            diagnostic.contract = Some(spec.origin.clone());
            diagnostics
                .entry(spec.origin.path.clone().expect("extern has a path"))
                .or_default()
                .push(diagnostic);
        }
    }
    resolved.namespaces.retain(|name, table| {
        if let Some(previous) = before.and_then(|before| before.namespaces.get(name))
            && table == previous
        {
            table.clone_from(previous);
        }
        !table.is_empty()
    });
    table
}

impl ExternCache {
    pub(crate) fn extern_spec(&self, name: &str) -> Option<(&str, &SpecConversion, Option<&str>)> {
        let spec = self.table.as_ref()?.spec(name)?;
        Some((
            &spec.registration.namespace,
            &spec.conversion,
            spec.alias.as_deref(),
        ))
    }

    pub(crate) fn declares_spec(&self, name: &str) -> bool {
        name.starts_with(':') && self.files.iter().any(|file| file.spans.contains_key(name))
    }
}

impl Externs {
    fn spec(&self, name: &str) -> Option<&SpecExtern> {
        let (namespace, _) = name.strip_prefix(':')?.split_once('/')?;
        let ExternalDeclaration::Spec(spec) = self.namespaces.get(namespace)?.get(name)? else {
            return None;
        };
        Some(spec)
    }
}

impl ExternCache {
    /// Publish source-free specs before type signatures in source modules are read.
    pub(crate) fn publish_specs(&self, globals: &mut Globals, trusted: Option<&BTreeSet<&str>>) {
        let Some(table) = self.table.as_ref().filter(|table| table.has_specs) else {
            return;
        };
        let conversions = table
            .namespaces
            .values()
            .flat_map(|table| table.values())
            .filter_map(|declaration| match declaration {
                ExternalDeclaration::Spec(spec) => Some(spec),
                _ => None,
            });
        let eligible = |spec: &SpecExtern| {
            !(globals.specs.contains_key(&spec.origin.name)
                || globals
                    .project_namespaces
                    .contains(&spec.registration.namespace)
                || ((spec.conversion.requires_standard_core() || spec.conversion.is_structured())
                    && globals.project_namespaces.contains("clojure.core"))
                || trusted.is_some_and(|paths| !paths.contains(spec.registration.path.as_str())))
        };
        // Resolve visibility against the source-only table. A bundle must not
        // keep a child hidden by source precedence or a cross-file conflict.
        let visible: Vec<_> = conversions
            .filter(|spec| {
                eligible(spec)
                    && spec
                        .dependencies
                        .iter()
                        .all(|name| table.spec(name).is_some_and(&eligible))
            })
            .collect();
        for spec in visible {
            let name = &spec.origin.name;
            Arc::make_mut(&mut globals.specs).insert(name.clone(), spec.conversion.success_input());
            if spec.conversion.has_unchecked_input()
                || spec.dependencies.iter().any(|name| {
                    table
                        .spec(name)
                        .is_some_and(|child| child.conversion.has_unchecked_input())
                })
            {
                Arc::make_mut(&mut globals.spec_unchecked_inputs).insert(name.clone());
            }
            Arc::make_mut(&mut globals.spec_conversions)
                .insert(name.clone(), spec.conversion.clone());
            let provider = SpecProvider::Imported {
                source: spec.registration.clone(),
                alias: spec.alias.clone(),
                structured: spec.conversion.is_structured(),
            };
            Arc::make_mut(&mut globals.spec_providers).insert(name.clone(), Arc::new(provider));
        }
    }
}
