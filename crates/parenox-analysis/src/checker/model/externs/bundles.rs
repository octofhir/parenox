//! Standalone s/keys declarations and their direct registrations.
use std::collections::BTreeMap;
use std::fmt::Write;

use super::{EncodeError, Names, SnapshotWriter};
use crate::checker::model::specs::{MapFieldConversion, SpecConversion, StructuredConversion};
use crate::checker::model::type_forms::{read_type, structured_conversion};
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

pub(super) fn parse(
    body: &Form,
    source: &str,
    lookup: impl Fn(&str) -> Option<SpecConversion>,
) -> Option<SpecConversion> {
    let names = Names::default();
    let Kind::List(items) = &body.kind else {
        return None;
    };
    let [_, options @ ..] = items.as_slice() else {
        return None;
    };
    if options.len() > 8
        || options
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[_, fields]| fields.vector())
            .map(<[Form]>::len)
            .sum::<usize>()
            > MAX_FIELDS
    {
        return None;
    }
    let mut diagnostics = Vec::new();
    let input = read_type(&names, body, source, &mut diagnostics, |key, _| {
        lookup(key).map(|conversion| conversion.success_input())
    });
    if !diagnostics.is_empty() {
        return None;
    }
    let conversion = structured_conversion(&names, body, &input, lookup)?;
    conversion.bundle_fields()?;
    Some(conversion)
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
