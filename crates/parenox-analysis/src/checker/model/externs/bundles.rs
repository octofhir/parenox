//! Portable map and collection spec declarations and their registrations.
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::fmt::Write;
use std::sync::Arc;

use super::{EncodeError, SnapshotWriter, SpecExtern};
use crate::checker::model::specs::{MapFieldConversion, SpecConversion, StructuredConversion};
use crate::checker::model::type_forms::spec_field_key;
use crate::reader::{Form, Kind};
use crate::types::{ExtraFields, Field, MapType, SPEC_INVALID, Type};

const MAX_FIELDS: usize = 32;
const MAX_EXPANDED_FIELDS: usize = 256;

impl SpecConversion {
    pub(crate) const MAX_EXTERN_DEPTH: u8 = 8;
    pub(crate) const MAX_EXTERN_DEPENDENCIES: usize = 256;

    pub(crate) fn portable_collection(&self) -> Option<(&str, &SpecConversion)> {
        let collection = self.named_collection()?;
        self.is_portable().then_some(collection)
    }

    pub(crate) fn portable_string_keyed_map(&self) -> Option<(&str, &SpecConversion)> {
        let map = self.named_string_keyed_map()?;
        self.is_portable().then_some(map)
    }

    fn named_string_keyed_map(&self) -> Option<(&str, &SpecConversion)> {
        let Self::StringKeyedMap {
            values,
            registration,
        } = self
        else {
            return None;
        };
        values.named_collection()?;
        Some((registration, values))
    }

    fn named_collection(&self) -> Option<(&str, &SpecConversion)> {
        let Self::Structured(structure) = self else {
            return None;
        };
        let StructuredConversion::Collection {
            element,
            registration: Some(name),
        } = structure.as_ref()
        else {
            return None;
        };
        element.map_fields()?;
        Some((name, element))
    }
    pub(crate) fn map_fields(&self) -> Option<&BTreeMap<Type, MapFieldConversion>> {
        let Self::Structured(map) = self else {
            return None;
        };
        let StructuredConversion::Map(fields) = map.as_ref() else {
            return None;
        };
        Some(fields)
    }

    pub(crate) fn bundle_fields(&self) -> Option<&BTreeMap<Type, MapFieldConversion>> {
        let fields = self.map_fields()?;
        self.is_portable().then_some(fields)
    }

    pub(in crate::checker::model) fn is_portable(&self) -> bool {
        fn check(conversion: &SpecConversion, depth: u8, budget: &mut usize) -> bool {
            if let Some((_, element)) = conversion
                .named_collection()
                .or_else(|| conversion.named_string_keyed_map())
            {
                if depth == 0 || *budget == 0 {
                    return false;
                }
                *budget -= 1;
                return check(element, depth - 1, budget);
            }
            let Some(fields) = conversion.map_fields() else {
                return conversion.is_extern_leaf();
            };
            if depth == 0
                || fields.is_empty()
                || fields.len() > MAX_FIELDS
                || fields.len() > *budget
            {
                return false;
            }
            *budget -= fields.len();
            fields.values().all(|field| {
                field
                    .conversion
                    .as_ref()
                    .is_some_and(|child| check(child, depth - 1, budget))
            })
        }
        let mut budget = MAX_EXPANDED_FIELDS;
        check(self, Self::MAX_EXTERN_DEPTH, &mut budget)
    }

    pub(crate) fn has_nested_collection(&self) -> bool {
        self.bundle_fields().is_some_and(|fields| {
            fields.values().any(|field| {
                field.conversion.as_ref().is_some_and(|child| {
                    child.named_collection().is_some()
                        || child.named_string_keyed_map().is_some()
                        || child.has_nested_collection()
                })
            })
        })
    }

    pub(super) fn bundle_success_type(&self) -> Option<Type> {
        let fields = self.bundle_fields()?;
        Some(Type::Map(MapType {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    let returned = field.conversion.as_ref()?.extern_success_type()?;
                    Some((name.clone(), Field::new(returned, field.required)))
                })
                .collect::<Option<_>>()?,
            extra: ExtraFields::Open,
        }))
    }

    pub(super) fn bundle_result(&self) -> Option<Type> {
        Some(Type::union([
            self.bundle_success_type()?,
            Type::Key(SPEC_INVALID.into()),
        ]))
    }
}

pub(super) fn is_keys(form: &Form) -> bool {
    matches!(&form.kind, Kind::List(items)
        if items.first().and_then(Form::symbol) == Some("clojure.spec.alpha/keys"))
}

pub(super) fn collection_child(form: &Form) -> Option<&str> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [head, child] = items.as_slice() else {
        return None;
    };
    (head.symbol() == Some("clojure.spec.alpha/coll-of"))
        .then(|| spec_keyword(child))
        .flatten()
}

pub(super) fn spec_keyword(form: &Form) -> Option<&str> {
    let keyword = form.keyword()?;
    let (namespace, local) = keyword.strip_prefix(':')?.split_once('/')?;
    (!namespace.is_empty() && !namespace.starts_with(':') && !local.is_empty()).then_some(keyword)
}

#[derive(PartialEq, Eq)]
pub(super) struct KeyFields(BTreeMap<Type, MapFieldConversion>);

impl KeyFields {
    pub(super) fn parse(body: &Form) -> Option<Self> {
        let Kind::List(items) = &body.kind else {
            return None;
        };
        let (_, options) = items.split_first()?;
        let (options, trailing) = options.as_chunks::<2>();
        if !trailing.is_empty() || options.len() > 4 {
            return None;
        }
        let mut fields = BTreeMap::<_, MapFieldConversion>::new();
        let mut directives = BTreeSet::new();
        let mut members = 0;
        for [directive, keys] in options {
            let directive = directive.keyword()?;
            let required = match directive {
                ":req" | ":req-un" => true,
                ":opt" | ":opt-un" => false,
                _ => return None,
            };
            let keys = keys.vector()?;
            members += keys.len();
            if members > MAX_FIELDS || !directives.insert(directive) {
                return None;
            }
            for key in keys {
                let registration = spec_keyword(key)?;
                let field = Type::Key(spec_field_key(registration, directive)?);
                match fields.entry(field) {
                    Entry::Vacant(entry) => {
                        entry.insert(MapFieldConversion {
                            registration: registration.into(),
                            required,
                            conversion: None,
                        });
                    }
                    Entry::Occupied(mut entry) if entry.get().registration == registration => {
                        entry.get_mut().required |= required;
                    }
                    Entry::Occupied(_) => return None,
                }
            }
        }
        (!fields.is_empty()).then_some(Self(fields))
    }

    pub(super) fn resolve<'a>(
        &self,
        namespace: &str,
        lookup: impl Fn(&str) -> Option<&'a SpecExtern>,
    ) -> Option<(SpecConversion, Vec<&'a SpecExtern>)> {
        let mut fields = self.0.clone();
        let mut children = Vec::with_capacity(fields.len());
        for field in fields.values_mut() {
            let child = lookup(&field.registration)?;
            if child.registration.namespace != namespace {
                return None;
            }
            field.conversion = Some(child.conversion.clone());
            children.push(child);
        }
        Some((
            SpecConversion::Structured(Arc::new(StructuredConversion::Map(fields))),
            children,
        ))
    }
}

impl SnapshotWriter {
    pub(super) fn keys(
        &mut self,
        fields: &BTreeMap<Type, MapFieldConversion>,
    ) -> Result<(), EncodeError> {
        self.nested(1, "(clojure.spec.alpha/keys", ")", |out| {
            for (directive, required, unqualified) in [
                ("req", true, false),
                ("req-un", true, true),
                ("opt", false, false),
                ("opt-un", false, true),
            ] {
                let mut members = fields
                    .iter()
                    .filter_map(|(key, field)| {
                        let Type::Key(key) = key else { return None };
                        (field.required == required && (key != &field.registration) == unqualified)
                            .then_some(field.registration.as_str())
                    })
                    .peekable();
                if members.peek().is_some() {
                    write!(out, " :{directive} ")?;
                    out.nested(1, "[", "]", |out| {
                        for (index, name) in members.enumerate() {
                            out.separator(index)?;
                            out.write_str(name)?;
                        }
                        Ok(())
                    })?;
                }
            }
            Ok(())
        })
    }
}
