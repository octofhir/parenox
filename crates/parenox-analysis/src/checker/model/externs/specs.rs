//! Standalone, exact spec conversions that can be exchanged as externs.
use std::collections::BTreeSet;
use std::fmt::Write;
use std::sync::Arc;

use super::{ExternCache, ExternalDeclaration, Globals, Model, Names, SnapshotWriter};
use crate::checker::model::type_forms::source_conversion;
use crate::checker::model::{SpecConversion, SpecProvider, specs::RegistrationSource};
use crate::reader::Form;
use crate::types::{Parameter, SPEC_INVALID, Type};
use crate::{ContractKind, ContractOrigin};

#[derive(Clone, PartialEq, Eq)]
pub(in crate::checker::model) struct SpecExtern {
    pub origin: ContractOrigin,
    pub conversion: SpecConversion,
    pub registration: Arc<RegistrationSource>,
}

impl SpecConversion {
    pub(crate) fn proposal_result(&self) -> Option<String> {
        let invalid = || Type::Key(SPEC_INVALID.into());
        let input = || {
            Type::Parameter(Arc::new(Parameter {
                name: "Input".into(),
                owner: String::new(),
                upper: None,
            }))
        };
        let result = match self {
            Self::WrapField(key) => Self::wrap_field(key, input()),
            Self::VectorWrapField(key) => {
                Self::wrap_field(key, Type::Vector(Box::new(Type::Any))).nilable()
            }
            Self::MapOrWrapField(key) => {
                let wrapped = Self::wrap_field(key, input());
                SnapshotWriter::new().ty(&wrapped).ok()?;
                return Some(format!(
                    "Input when vector? is false and map? is true, {wrapped} otherwise"
                ));
            }
            Self::String => Type::union([Type::String, invalid()]),
            Self::IdentityDomain(domain) if literal_strings(domain).is_some() => {
                Type::union([(**domain).clone(), invalid()])
            }
            _ => return None,
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
}

impl Model {
    pub(super) fn read_extern_spec(
        &mut self,
        name: &str,
        declaration: &Form,
        path: &str,
    ) -> Option<ExternalDeclaration> {
        let names = Names::default();
        let parse = || {
            let [first, a, second, b] = declaration.map()? else {
                return None;
            };
            let (body, namespace) = match (first.keyword(), second.keyword()) {
                (Some(":spec"), Some(":ns")) => (a, b),
                (Some(":ns"), Some(":spec")) => (b, a),
                _ => return None,
            };
            let namespace = namespace.symbol().filter(|name| !name.contains('/'))?;
            let conversion = source_conversion(&names, body)?;
            conversion.proposal_result()?;
            Some((namespace, conversion))
        };
        let Some((namespace, conversion)) = parse() else {
            self.error(
                "PNX9001",
                declaration.span,
                "Expected {:spec <supported standalone spec conversion> :ns <library namespace>}",
            );
            return None;
        };
        Some(ExternalDeclaration::Spec(Arc::new(SpecExtern {
            origin: ContractOrigin {
                name: name.into(),
                kind: ContractKind::Extern,
                path: Some(path.into()),
            },
            conversion,
            registration: Arc::new(RegistrationSource {
                path: path.into(),
                namespace: namespace.into(),
            }),
        })))
    }

    pub(crate) fn extern_spec(&self, name: &str) -> Option<(&str, &SpecConversion)> {
        let (namespace, _) = name.strip_prefix(':')?.split_once('/')?;
        let ExternalDeclaration::Spec(spec) = self
            .externs
            .as_ref()?
            .table
            .namespaces
            .get(namespace)?
            .get(name)?
        else {
            return None;
        };
        Some((&spec.registration.namespace, &spec.conversion))
    }
}

impl ExternCache {
    pub(crate) fn contains_spec(&self, name: &str) -> bool {
        let Some((namespace, _)) = name.strip_prefix(':').and_then(|name| name.split_once('/'))
        else {
            return false;
        };
        self.table
            .as_ref()
            .and_then(|table| table.namespaces.get(namespace))
            .is_some_and(|table| matches!(table.get(name), Some(ExternalDeclaration::Spec(_))))
    }

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
        for spec in conversions {
            let name = &spec.origin.name;
            if globals.specs.contains_key(name)
                || globals
                    .project_namespaces
                    .contains(&spec.registration.namespace)
                || trusted.is_some_and(|paths| {
                    !paths.contains(spec.origin.path.as_deref().unwrap_or_default())
                })
            {
                continue;
            }
            Arc::make_mut(&mut globals.specs).insert(name.clone(), spec.conversion.success_input());
            Arc::make_mut(&mut globals.spec_conversions)
                .insert(name.clone(), spec.conversion.clone());
            Arc::make_mut(&mut globals.spec_providers).insert(
                name.clone(),
                Arc::new(SpecProvider::LoadChecked {
                    direct: true,
                    sources: Box::from([spec.registration.clone()]),
                }),
            );
        }
    }
}
