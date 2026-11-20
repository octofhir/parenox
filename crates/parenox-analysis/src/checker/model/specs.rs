//! Project-wide spec declarations, resolution and cache validity.
mod registration;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::contracts::function_parts;
use super::declarations::{Declarations, Names, SpecDeclaration, symbols_match};
use super::type_forms::{
    core_predicate_type, enum_conversion, projected_collection_child, read_type, source_conversion,
    string_keyed_map_child, structured_conversion, tagged_or, tagged_projection,
};
use crate::Diagnostic;
use crate::reader::{Form, Kind, Span};
use crate::types::{ExtraFields, Field, IntegerType, MapType, Type};

#[derive(PartialEq, Eq)]
pub(crate) struct SpecFingerprint {
    names: Names,
    bodies: Vec<(String, Span, Span, String)>,
    duplicates: BTreeSet<String>,
    enums: BTreeMap<String, SpecConversion>,
    registration_barriers: Vec<usize>,
}

impl Declarations {
    pub(super) fn has_preserving_spec(&self, forms: &[Form]) -> bool {
        let uses_preserving_predicate = |form: &Form| {
            !symbols_match(form, &mut |name| {
                !matches!(
                    self.names.resolve_parts(name),
                    ("clojure.core", "identity" | "any?")
                )
            })
        };
        self.specs.values().any(|spec| {
            uses_preserving_predicate(&spec.body)
                || spec.body.symbol().is_some_and(|symbol| {
                    matches!(
                        self.names.resolve_parts(symbol),
                        ("clojure.core", "keyword?" | "string?")
                    )
                })
        }) || self
            .fdefs
            .iter()
            .any(|&index| uses_preserving_predicate(&forms[index]))
    }

    pub(super) fn has_inline_spec_composition(&self, forms: &[Form]) -> bool {
        self.functions.iter().any(|&index| {
            let Kind::List(items) = &forms[index].without_metadata().kind else {
                return false;
            };
            let [_, name, tail @ ..] = items.as_slice() else {
                return false;
            };
            // Dependency scanning skips quoted data; :t/sig uses quoted types.
            let signature = function_parts(tail)
                .1
                .find_map(|[key, value]| (key.keyword() == Some(":t/sig")).then_some(value))
                .or_else(|| name.metadata(":t/sig"));
            signature.is_some_and(|form| {
                !symbols_match(form.unquoted(), &mut |symbol| {
                    !matches!(
                        self.names.resolve_parts(symbol),
                        ("clojure.spec.alpha", "and" | "conformer" | "coll-of")
                            | ("clojure.core", "integer?" | "int?" | "identity" | "any?")
                    )
                })
            })
        })
    }

    pub(super) fn read_spec_core_scope(&mut self, forms: &[Form]) {
        self.names.spec_core_shadows = self
            .top_level_names(forms)
            .into_iter()
            .chain(self.forward_vars.keys().map(String::as_str))
            .filter(|name| {
                let (namespace, local) = self.names.resolve_parts(name);
                ((namespace == "clojure.core"
                    && (matches!(local, "map" | "second" | "int" | "fn" | "name" | "=")
                        || core_predicate_type(local).is_some()))
                    || (namespace == "clojure.spec.alpha"
                        && matches!(
                            local,
                            "def"
                                | "and"
                                | "or"
                                | "conformer"
                                | "keys"
                                | "coll-of"
                                | "map-of"
                                | "tuple"
                                | "nilable"
                        ))
                    || (namespace == "clojure.set" && local == "map-invert"))
                    && (self.forward_vars.contains_key(*name)
                        || self.source_declarations(forms, name).next().is_some())
            })
            .map(str::to_owned)
            .collect();
        self.names.spec_core_scope_known =
            self.all_refers.is_empty() && self.has_known_var_surface(forms);
    }

    pub fn spec_fingerprint(&self, source: &str, forms: &[Form]) -> Option<SpecFingerprint> {
        if self.specs.is_empty() {
            return None;
        }
        let bodies = self
            .specs
            .iter()
            .map(|(key, declaration)| {
                let span = declaration.body.span;
                let text = &source[span.start.offset..span.end.offset];
                (key.clone(), declaration.name, span, text.to_owned())
            })
            .collect();
        Some(SpecFingerprint {
            names: self.names.clone(),
            bodies,
            duplicates: self.duplicate_specs.clone(),
            enums: self
                .specs
                .iter()
                .filter_map(|(key, declaration)| {
                    enum_conversion(self, forms, source, &declaration.body)
                        .map(|conversion| (key.clone(), conversion))
                })
                .collect(),
            registration_barriers: registration::barriers(self, forms),
        })
    }
}

pub(crate) struct SpecSource<'a> {
    pub path: &'a str,
    pub source: &'a str,
    pub declarations: &'a Declarations,
    pub fingerprint: &'a Arc<SpecFingerprint>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RegistrationSource {
    pub path: String,
    pub namespace: String,
}

#[derive(PartialEq, Eq)]
pub(crate) enum SpecProvider {
    Direct(Arc<RegistrationSource>),
    LoadChecked {
        direct: bool,
        sources: Box<[Arc<RegistrationSource>]>,
    },
}

impl SpecProvider {
    pub fn is_self_contained(&self) -> bool {
        matches!(
            self,
            Self::Direct(_) | Self::LoadChecked { direct: true, .. }
        )
    }
}

/// An exact built-in conversion, separate from a spec's static input shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SpecConversion {
    CheckedInt32,
    Name,
    String,
    Keyword,
    Identity,
    IdentityDomain(Arc<Type>),
    Any,
    AnyCollection,
    Enum {
        entries: Arc<[(Type, Type)]>,
        inverted: bool,
    },
    TaggedChoice(Arc<[SpecConversion]>),
    TaggedOr(Arc<[(String, SpecConversion)]>),
    ProjectedCollection(Arc<SpecConversion>),
    StringKeyedMap(Arc<SpecConversion>),
    Structured(Arc<StructuredConversion>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StructuredConversion {
    Map(BTreeMap<Type, MapFieldConversion>),
    Collection(SpecConversion),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MapFieldConversion {
    pub required: bool,
    /// A missing registration checks presence but leaves a present value alone.
    pub conversion: Option<SpecConversion>,
}

impl SpecConversion {
    /// The original input domain of a checked contract (successful validation).
    pub fn success_input(&self) -> Type {
        match self {
            Self::CheckedInt32 => Type::Integral(IntegerType::INT32),
            // JVM Named can also be implemented by user classes. This
            // conversion supplies output and effects but no input contract.
            Self::Name => Type::Unknown,
            Self::String => Type::String,
            Self::Keyword => Type::SpecKeyword,
            Self::Identity => Type::SpecTruthy,
            Self::IdentityDomain(domain) => (**domain).clone(),
            Self::Any => Type::SpecValue,
            Self::AnyCollection => Type::SpecCollection(Box::new(Type::SpecValue)),
            Self::Enum { entries, .. } => Type::union(entries.iter().map(|(key, _)| key.clone())),
            Self::TaggedChoice(branches) => Type::union(branches.iter().map(Self::success_input)),
            Self::TaggedOr(branches) => {
                Type::union(branches.iter().map(|(_, branch)| branch.success_input()))
            }
            Self::ProjectedCollection(child) => {
                Type::SpecCollection(Box::new(child.success_input()))
            }
            Self::StringKeyedMap(child) => {
                Type::Map(MapType::dictionary(Type::String, child.success_input()))
            }
            Self::Structured(spec) => match spec.as_ref() {
                StructuredConversion::Map(fields) => Type::Map(MapType {
                    fields: fields
                        .iter()
                        .map(|(key, field)| {
                            let input = field.conversion.as_ref().map_or_else(
                                || Field::presence_only(field.required),
                                |child| Field::new(child.success_input(), field.required),
                            );
                            (key.clone(), input)
                        })
                        .collect(),
                    extra: ExtraFields::Open,
                }),
                StructuredConversion::Collection(child) => {
                    Type::SpecCollection(Box::new(child.success_input()))
                }
            },
        }
    }
}

#[derive(Default)]
pub(crate) struct SpecCache {
    previous: Vec<(String, Arc<SpecFingerprint>)>,
    project_set_namespace: bool,
    project_core_namespace: bool,
    pub types: Arc<BTreeMap<String, Type>>,
    pub conversions: Arc<BTreeMap<String, SpecConversion>>,
    pub(super) unchecked_inputs: Arc<BTreeSet<String>>,
    pub(super) providers: Arc<BTreeMap<String, Arc<SpecProvider>>>,
    pub diagnostics: BTreeMap<String, Vec<Diagnostic>>,
}

impl SpecCache {
    pub fn update(
        &mut self,
        mut sources: Vec<SpecSource<'_>>,
        project_set_namespace: bool,
        project_core_namespace: bool,
    ) {
        sources.sort_by_key(|source| source.path);
        if project_set_namespace == self.project_set_namespace
            && project_core_namespace == self.project_core_namespace
            && sources.len() == self.previous.len()
            && sources
                .iter()
                .zip(&self.previous)
                .all(|(source, (path, fingerprint))| {
                    source.path == path && source.fingerprint == fingerprint
                })
        {
            return;
        }

        let mut resolver = Resolver {
            project_core_namespace,
            ..Resolver::default()
        };
        let mut duplicate = BTreeSet::new();
        let mut registrations = BTreeMap::new();
        for source in &sources {
            let registration = Arc::new(RegistrationSource {
                path: source.path.to_owned(),
                namespace: source.declarations.namespace().to_owned(),
            });
            for (key, declaration) in &source.declarations.specs {
                let owner = Owner {
                    path: source.path,
                    source: source.source,
                    names: &source.declarations.names,
                    declaration,
                    registration_barriers: &source.fingerprint.registration_barriers,
                    enum_conversion: source.fingerprint.enums.get(key).filter(|conversion| {
                        !project_set_namespace
                            || !matches!(conversion, SpecConversion::Enum { inverted: true, .. })
                    }),
                };
                if let Some(previous) = resolver.declarations.get(key.as_str()).copied() {
                    if duplicate.insert(key.as_str()) {
                        resolver.duplicate(key, previous);
                    }
                    resolver.duplicate(key, owner);
                } else {
                    resolver.declarations.insert(key, owner);
                    registrations.insert(key.as_str(), registration.clone());
                }
                if source.declarations.duplicate_specs.contains(key) {
                    resolver.types.insert(key.to_owned(), Type::Unknown);
                }
            }
        }
        for key in duplicate {
            resolver.types.insert(key.to_owned(), Type::Unknown);
        }

        let keys: Vec<_> = resolver.declarations.keys().copied().collect();
        for key in keys {
            let owner = resolver.declarations[key];
            resolver.resolve(key, owner.path, owner.declaration.name);
        }
        let providers = registrations
            .iter()
            .filter_map(|(&key, source)| {
                let direct = resolver.direct_conversions.contains(key);
                let provider = if let Some(keys) = resolver.required_registrations.get(key) {
                    let sources: BTreeSet<_> =
                        keys.iter().map(|key| registrations[key].clone()).collect();
                    SpecProvider::LoadChecked {
                        direct,
                        sources: sources.into_iter().collect(),
                    }
                } else if direct {
                    SpecProvider::Direct(source.clone())
                } else {
                    return None;
                };
                Some((key.to_owned(), Arc::new(provider)))
            })
            .collect();
        self.types = Arc::new(resolver.types);
        self.conversions = Arc::new(resolver.conversions);
        self.unchecked_inputs = Arc::new(resolver.unchecked_inputs);
        self.providers = Arc::new(providers);
        self.diagnostics = resolver.diagnostics;
        self.project_set_namespace = project_set_namespace;
        self.project_core_namespace = project_core_namespace;
        self.previous = sources
            .into_iter()
            .map(|source| (source.path.to_owned(), source.fingerprint.clone()))
            .collect();
    }
}

#[derive(Clone, Copy)]
struct Owner<'a> {
    path: &'a str,
    source: &'a str,
    names: &'a Names,
    declaration: &'a SpecDeclaration,
    registration_barriers: &'a [usize],
    enum_conversion: Option<&'a SpecConversion>,
}

#[derive(Default)]
struct Resolver<'a> {
    project_core_namespace: bool,
    unchecked_inputs: BTreeSet<String>,
    declarations: BTreeMap<&'a str, Owner<'a>>,
    active: Vec<&'a str>,
    types: BTreeMap<String, Type>,
    conversions: BTreeMap<String, SpecConversion>,
    direct_conversions: BTreeSet<&'a str>,
    local_conversions: BTreeMap<&'a str, usize>,
    required_registrations: BTreeMap<&'a str, BTreeSet<&'a str>>,
    diagnostics: BTreeMap<String, Vec<Diagnostic>>,
}

impl<'a> Resolver<'a> {
    fn resolve(&mut self, key: &str, path: &str, span: Span) -> Option<Type> {
        if let Some(ty) = self.types.get(key) {
            return Some(ty.clone());
        }
        let (&key, &owner) = self.declarations.get_key_value(key)?;
        if self.active.contains(&key) {
            self.diagnostics
                .entry(path.into())
                .or_default()
                .push(Diagnostic::analysis_gap(
                    "PNX9001",
                    span,
                    format!("Recursive spec {key} is not supported yet"),
                ));
            return Some(Type::Unknown);
        }
        if self.active.len() >= 128 {
            self.error(path, span, "Spec expansion limit (128) exceeded");
            return Some(Type::Unknown);
        }

        let body = &owner.declaration.body;
        let registered_at = body.span.end.offset;
        if let Some(target) = body.keyword().map(|target| owner.names.key(target))
            && let Some(declaration) = self.declarations.get(target.as_str())
            && declaration.path == owner.path
            && declaration.declaration.body.span.end.offset > registered_at
        {
            // A direct registry alias probes its target during s/def. In
            // contrast, a registered map can resolve its fields on use.
            self.diagnostics
                .entry(owner.path.into())
                .or_default()
                .push(Diagnostic::analysis_gap(
                    "PNX9001",
                    body.span,
                    format!("Spec alias {key} requires {target} to be registered first"),
                ));
            self.types.insert(key.to_owned(), Type::Unknown);
            return Some(Type::Unknown);
        }

        self.active.push(key);
        let mut diagnostics = Vec::new();
        let mut required = BTreeSet::new();
        let direct = owner
            .enum_conversion
            .cloned()
            .or_else(|| source_conversion(owner.names, body))
            .filter(|conversion| {
                !self.project_core_namespace || !matches!(conversion, SpecConversion::Name)
            })
            .or_else(|| self.collection_conversion(owner, registered_at, &mut required));
        if direct.is_some() {
            self.direct_conversions.insert(key);
        }
        let mut local_ready_at = Some(registered_at);
        let ty = if let Some(conversion) = &direct {
            conversion.success_input()
        } else {
            read_type(
                owner.names,
                &owner.declaration.body,
                owner.source,
                &mut diagnostics,
                |child, span| {
                    let ty = self.resolve(child, owner.path, span);
                    if let Some(sources) = self.required_registrations.get(child) {
                        required.extend(sources);
                    }
                    if ty.is_some() {
                        local_ready_at = local_ready_at.and_then(|ready| {
                            let source = self.declarations.get(child)?;
                            if source.path != owner.path {
                                return None;
                            }
                            Some(ready.max(*self.local_conversions.get(child)?))
                        });
                    }
                    // s/keys handles an absent field registration as presence
                    // only. An unresolved named branch cannot produce a conversion.
                    ty
                },
            )
        };
        let conversion = direct
            .or_else(|| {
                structured_conversion(owner.names, body, &ty, |key| {
                    self.conversions.get(key).cloned()
                })
            })
            .or_else(|| {
                // The choice memoizes resolution of all branch specs, including
                // branches it will skip. Require local registrations with no
                // intervening initializer that could force resolution early.
                let ready = local_ready_at?;
                let lookup = |key: &str| self.conversions.get(key).cloned();
                let conversion = tagged_projection(owner.names, body, &ty, lookup)
                    .or_else(|| tagged_or(owner.names, body, lookup))?;
                let barriers = owner.registration_barriers;
                let next = barriers.partition_point(|offset| *offset < registered_at);
                barriers
                    .get(next)
                    .is_none_or(|offset| *offset >= ready)
                    .then_some(conversion)
            })
            .or_else(|| {
                let alias = owner.names.key(body.keyword()?);
                self.conversions.get(&alias).cloned()
            });
        if let Some(conversion) = conversion {
            if let Some(ready) = local_ready_at {
                self.local_conversions.insert(key, ready);
            }
            if matches!(
                conversion,
                SpecConversion::Name
                    | SpecConversion::IdentityDomain(_)
                    | SpecConversion::TaggedChoice(_)
                    | SpecConversion::TaggedOr(_)
                    | SpecConversion::ProjectedCollection(_)
                    | SpecConversion::StringKeyedMap(_)
                    | SpecConversion::Enum { inverted: true, .. }
            ) {
                required.insert(key);
            }
            self.conversions.insert(key.to_owned(), conversion);
        }
        // A supported conversion may still lack an input type. Propagate
        // that distinction to aliases and contracts containing the spec.
        if matches!(self.conversions.get(key), Some(SpecConversion::Name))
            || required
                .iter()
                .any(|child| self.unchecked_inputs.contains(*child))
        {
            self.unchecked_inputs.insert(key.to_owned());
        }
        if !required.is_empty() {
            required.insert(key);
            self.required_registrations.insert(key, required);
        }
        self.active.pop();
        if !diagnostics.is_empty() {
            self.diagnostics
                .entry(owner.path.into())
                .or_default()
                .extend(diagnostics);
        }
        self.types.insert(key.to_owned(), ty.clone());
        Some(ty)
    }

    fn collection_conversion(
        &mut self,
        owner: Owner<'a>,
        registered_at: usize,
        required: &mut BTreeSet<&'a str>,
    ) -> Option<SpecConversion> {
        let body = &owner.declaration.body;
        let (key, conversion) = if let Some(child) = projected_collection_child(owner.names, body) {
            let (key, element) = self.earlier_local_conversion(owner, registered_at, child)?;
            if !matches!(element, SpecConversion::TaggedOr(_)) {
                return None;
            }
            (key, SpecConversion::ProjectedCollection(Arc::new(element)))
        } else {
            let child = string_keyed_map_child(owner.names, body)?;
            let (key, value) = self.earlier_local_conversion(owner, registered_at, child)?;
            (key, SpecConversion::StringKeyedMap(Arc::new(value)))
        };
        if let Some(sources) = self.required_registrations.get(key) {
            required.extend(sources);
        }
        Some(conversion)
    }

    fn earlier_local_conversion(
        &mut self,
        owner: Owner<'a>,
        registered_at: usize,
        child: &Form,
    ) -> Option<(&'a str, SpecConversion)> {
        let name = owner.names.validation_key(child.keyword()?)?;
        let (&key, source) = self.declarations.get_key_value(name.as_str())?;
        if source.path != owner.path || source.declaration.body.span.end.offset >= registered_at {
            return None;
        }
        self.resolve(key, owner.path, child.span)?;
        if *self.local_conversions.get(key)? > registered_at {
            return None;
        }
        Some((key, self.conversions.get(key)?.clone()))
    }

    fn duplicate(&mut self, key: &str, owner: Owner<'_>) {
        self.error(
            owner.path,
            owner.declaration.name,
            format!("Spec {key} is declared in more than one source file"),
        );
    }

    fn error(&mut self, path: &str, span: Span, message: impl Into<String>) {
        self.diagnostics
            .entry(path.into())
            .or_default()
            .push(Diagnostic::new("PNX9001", span, message));
    }
}
