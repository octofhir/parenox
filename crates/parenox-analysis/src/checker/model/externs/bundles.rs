//! Standalone s/keys declarations and their direct registrations.
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::fmt::Write;
use std::sync::Arc;

use super::{EncodeError, SnapshotWriter, SpecExtern};
use crate::checker::model::specs::{MapFieldConversion, SpecConversion, StructuredConversion};
use crate::checker::model::type_forms::spec_field_key;
use crate::reader::{Form, Kind};
use crate::types::{ExtraFields, Field, MapType, SPEC_INVALID, Type};

const MAX_FIELDS: usize = 32;

impl SpecConversion {
    pub(crate) fn bundle_fields(&self) -> Option<&BTreeMap<Type, MapFieldConversion>> {
        let Self::Structured(map) = self else {
            return None;
        };
        let StructuredConversion::Map(fields) = map.as_ref() else {
            return None;
        };
        if fields.is_empty() || fields.len() > MAX_FIELDS {
            return None;
        }
        fields
            .values()
            .all(|field| field.conversion.as_ref().is_some_and(Self::is_extern_leaf))
            .then_some(fields)
    }

    pub(super) fn bundle_result(&self) -> Option<Type> {
        let fields = self.bundle_fields()?;
        let shape = Type::Map(MapType {
            fields: fields
                .iter()
                .map(|(name, field)| {
                    let returned = field.conversion.as_ref()?.extern_success_type()?;
                    Some((name.clone(), Field::new(returned, field.required)))
                })
                .collect::<Option<_>>()?,
            extra: ExtraFields::Open,
        });
        Some(Type::union([shape, Type::Key(SPEC_INVALID.into())]))
    }
}

pub(super) fn is_keys(form: &Form) -> bool {
    matches!(&form.kind, Kind::List(items)
        if items.first().and_then(Form::symbol) == Some("clojure.spec.alpha/keys"))
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
                let registration = key.keyword()?;
                let name = registration.strip_prefix(':')?;
                let (namespace, local) = name.split_once('/')?;
                if namespace.is_empty() || namespace.starts_with(':') || local.is_empty() {
                    return None;
                }
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
    ) -> Option<SpecConversion> {
        let mut fields = self.0.clone();
        for field in fields.values_mut() {
            let leaf = lookup(&field.registration)?;
            if leaf.registration.namespace != namespace || !leaf.conversion.is_extern_leaf() {
                return None;
            }
            field.conversion = Some(leaf.conversion.clone());
        }
        Some(SpecConversion::Structured(Arc::new(
            StructuredConversion::Map(fields),
        )))
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
