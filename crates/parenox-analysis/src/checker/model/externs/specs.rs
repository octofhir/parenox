//! Standalone, exact spec conversions that can be exchanged as externs.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::sync::Arc;

use super::{
    ExternCache, ExternalDeclaration, Externs, Globals, Model, Names, SnapshotWriter, bundles,
};
use crate::checker::model::type_forms::{live_lookup_projection, source_conversion};
use crate::checker::model::{
    GuardedMapProjection, SpecConversion, SpecProvider,
    specs::{ProjectedFields, ProjectedValue, RegistrationSource},
};
use crate::reader::{Form, Span};
use crate::types::{ExtraFields, MapType, Parameter, SPEC_INVALID, Type};
use crate::{ContractKind, ContractOrigin, Diagnostic};

#[derive(Clone, PartialEq, Eq)]
pub(in crate::checker::model) struct SpecExtern {
    pub origin: ContractOrigin,
    pub conversion: SpecConversion,
    pub registration: Arc<RegistrationSource>,
}

#[derive(PartialEq, Eq)]
pub(in crate::checker::model) struct PendingSpec {
    pub origin: ContractOrigin,
    keys: Option<PendingKeys>,
}

#[derive(PartialEq, Eq)]
struct PendingKeys {
    namespace: String,
    fields: bundles::KeyFields,
    span: Span,
}

impl PendingSpec {
    pub(super) fn invalid(origin: ContractOrigin) -> ExternalDeclaration {
        ExternalDeclaration::PendingSpec(Arc::new(Self { origin, keys: None }))
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
            Self::WrapField(_)
            | Self::VectorWrapField(_)
            | Self::MapOrWrapField(_)
            | Self::GuardedMap(_)
            | Self::LiveLookupMap(_)
            | Self::String => true,
            Self::IdentityDomain(domain) => literal_strings(domain).is_some(),
            _ => false,
        }
    }

    pub(super) fn extern_success_type(&self) -> Option<Type> {
        Some(match self {
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
        let result = match self {
            Self::WrapField(key) => Self::wrap_field(key, input()),
            Self::MapOrWrapField(key) => {
                let wrapped = Self::wrap_field(key, input());
                SnapshotWriter::new().ty(&wrapped).ok()?;
                return Some(format!(
                    "Input when vector? is false and map? is true, {wrapped} otherwise"
                ));
            }
            Self::Structured(_) => self.bundle_result()?,
            Self::String | Self::IdentityDomain(_) => {
                Type::union([self.extern_success_type()?, Type::Key(SPEC_INVALID.into())])
            }
            _ => self.extern_success_type()?,
        };
        SnapshotWriter::new().ty(&result).ok()?;
        Some(result.to_string())
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
    ) -> Result<(), super::EncodeError> {
        self.nested(1, "{:spec ", "}", |out| {
            match conversion {
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
                SpecConversion::LiveLookupMap(projection) => out.nested(
                    2,
                    "(clojure.spec.alpha/conformer (clojure.core/fn [input] ",
                    "))",
                    |out| out.projection_fields(&projection.fields, Some(&projection.var)),
                ),
                SpecConversion::Structured(_) if let Some(fields) = conversion.bundle_fields() => {
                    out.keys(fields)
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
            if is_keys {
                if let Some(fields) = bundles::KeyFields::parse(body) {
                    let keys = Some(PendingKeys {
                        namespace: namespace.into(),
                        fields,
                        span: declaration.span,
                    });
                    return ExternalDeclaration::PendingSpec(Arc::new(PendingSpec {
                        origin,
                        keys,
                    }));
                }
            } else if let Some(conversion) = source_conversion(&names, body)
                .or_else(|| live_lookup_projection(&names, namespace, body))
                .filter(|conversion| conversion.proposal_result().is_some())
            {
                return spec_extern(origin, namespace, conversion);
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

fn spec_extern(
    origin: ContractOrigin,
    namespace: &str,
    conversion: SpecConversion,
) -> ExternalDeclaration {
    let registration = RegistrationSource {
        path: origin.path.clone().expect("extern has a path"),
        namespace: namespace.into(),
    };
    ExternalDeclaration::Spec(Arc::new(SpecExtern {
        origin,
        conversion,
        registration: Arc::new(registration),
    }))
}

pub(super) fn resolve_specs(
    mut table: Arc<Externs>,
    before: Option<&Externs>,
    diagnostics: &mut BTreeMap<String, Vec<Diagnostic>>,
) -> Arc<Externs> {
    if !table.has_specs {
        return table;
    }
    let mut edits = Vec::new();
    for (namespace, declarations) in &table.namespaces {
        for (name, declaration) in declarations.iter() {
            let ExternalDeclaration::PendingSpec(spec) = declaration else {
                continue;
            };
            let resolved = spec.keys.as_ref().and_then(|keys| {
                let conversion = keys
                    .fields
                    .resolve(&keys.namespace, |key| table.spec(key))?;
                conversion.proposal_result()?;
                Some(spec_extern(
                    spec.origin.clone(),
                    &keys.namespace,
                    conversion,
                ))
            });
            if resolved.is_none()
                && let Some(keys) = &spec.keys
            {
                let message = format!(
                    "An s/keys extern needs supported direct child specs with the same :ns ({})",
                    keys.namespace
                );
                let mut diagnostic = Diagnostic::analysis_gap("PNX9001", keys.span, message);
                diagnostic.contract = Some(spec.origin.clone());
                diagnostics
                    .entry(spec.origin.path.clone().expect("extern has a path"))
                    .or_default()
                    .push(diagnostic);
            }
            edits.push((namespace.clone(), name.clone(), resolved));
        }
    }
    if edits.is_empty() {
        return table;
    }
    let resolved = Arc::make_mut(&mut table);
    for (namespace, name, declaration) in edits {
        let declarations = Arc::make_mut(resolved.namespaces.get_mut(&namespace).unwrap());
        if let Some(declaration) = declaration {
            declarations.insert(name, declaration);
        } else {
            declarations.remove(&name);
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
    pub(crate) fn extern_spec(&self, name: &str) -> Option<(&str, &SpecConversion)> {
        let spec = self.table.as_ref()?.spec(name)?;
        Some((&spec.registration.namespace, &spec.conversion))
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
                || ((spec.conversion.requires_standard_core()
                    || matches!(spec.conversion, SpecConversion::Structured(_)))
                    && globals.project_namespaces.contains("clojure.core"))
                || trusted.is_some_and(|paths| !paths.contains(spec.registration.path.as_str())))
        };
        // Resolve visibility against the source-only table. A bundle must not
        // keep a child hidden by source precedence or a cross-file conflict.
        let visible: Vec<_> = conversions
            .filter(|spec| {
                eligible(spec)
                    && spec.conversion.bundle_fields().is_none_or(|fields| {
                        fields
                            .values()
                            .all(|field| table.spec(&field.registration).is_some_and(&eligible))
                    })
            })
            .collect();
        for spec in visible {
            let name = &spec.origin.name;
            Arc::make_mut(&mut globals.specs).insert(name.clone(), spec.conversion.success_input());
            if spec.conversion.has_unchecked_input()
                || spec.conversion.bundle_fields().is_some_and(|fields| {
                    fields.values().any(|field| {
                        field
                            .conversion
                            .as_ref()
                            .is_some_and(SpecConversion::has_unchecked_input)
                    })
                })
            {
                Arc::make_mut(&mut globals.spec_unchecked_inputs).insert(name.clone());
            }
            Arc::make_mut(&mut globals.spec_conversions)
                .insert(name.clone(), spec.conversion.clone());
            let provider = if matches!(spec.conversion, SpecConversion::Structured(_)) {
                SpecProvider::ImportedKeys(spec.registration.clone())
            } else {
                SpecProvider::LoadChecked {
                    direct: true,
                    sources: Box::from([spec.registration.clone()]),
                }
            };
            Arc::make_mut(&mut globals.spec_providers).insert(name.clone(), Arc::new(provider));
        }
    }
}
