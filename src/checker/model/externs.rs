//! Immutable EDN declarations shared with each project snapshot.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{Globals, Model, Names, Signature};
use crate::reader::{Form, Kind, Span};

pub(super) struct Externs {
    pub(super) functions: BTreeMap<String, Signature>,
    pub(super) interfaces: BTreeMap<String, String>,
}

impl Model {
    pub fn externs(forms: &[Form], globals: &Globals) -> Self {
        let mut model = Self {
            names: Names {
                namespace: "externs".into(),
                ..Names::default()
            },
            ..Self::default()
        };
        let [
            Form {
                kind: Kind::Map(entries),
                ..
            },
        ] = forms
        else {
            model.error(
                "PNX9001",
                forms.first().map_or(Span::default(), |f| f.span),
                "Externs must be one EDN map of qualified function names to signatures",
            );
            return model;
        };
        let mut functions = BTreeMap::new();
        for [name, signature] in entries.as_chunks::<2>().0 {
            let Some(name_string) = name.symbol().filter(|s| s.contains('/')) else {
                model.error(
                    "PNX9001",
                    name.span,
                    "Extern function names must be qualified symbols",
                );
                continue;
            };
            if let Some(signature) = model.parse_signature(signature, globals)
                && functions
                    .insert(name_string.to_owned(), signature)
                    .is_some()
            {
                model.error(
                    "PNX2790",
                    name.span,
                    format!("Duplicate extern {name_string}"),
                );
            }
        }
        let mut namespaces: BTreeMap<&str, Vec<_>> = BTreeMap::new();
        for (name, signature) in &functions {
            namespaces
                .entry(name.rsplit_once('/').unwrap().0)
                .or_default()
                .push((name, signature));
        }
        let interfaces = namespaces
            .into_iter()
            .map(|(namespace, signatures)| (namespace.to_owned(), format!("{signatures:?}")))
            .collect();
        model.externs = Some(Arc::new(Externs {
            functions,
            interfaces,
        }));
        model
    }

    pub fn publish_externs(&self, globals: &mut Globals) {
        globals.externs.clone_from(&self.externs);
    }
}
