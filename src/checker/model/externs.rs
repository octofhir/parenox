//! Immutable EDN declarations shared with each project snapshot.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::interfaces::Signatures;
use super::{Globals, Model, Names};
use crate::reader::{Form, Kind, Span};

pub(super) struct Externs {
    pub(super) namespaces: BTreeMap<String, Arc<Signatures>>,
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
        let mut namespaces = BTreeMap::<String, Signatures>::new();
        for [name, signature] in entries.as_chunks::<2>().0 {
            let Some(name_string) = name.symbol().filter(|symbol| {
                symbol
                    .split_once('/')
                    .is_some_and(|(namespace, local)| !namespace.is_empty() && !local.is_empty())
            }) else {
                model.error(
                    "PNX9001",
                    name.span,
                    "Extern function names must be qualified symbols",
                );
                continue;
            };
            let (namespace, _) = name_string.split_once('/').unwrap();
            if let Some(signatures) = model.parse_signatures(signature, globals)
                && namespaces
                    .entry(namespace.into())
                    .or_default()
                    .insert(name_string.to_owned(), signatures.into())
                    .is_some()
            {
                model.error(
                    "PNX2790",
                    name.span,
                    format!("Duplicate extern {name_string}"),
                );
            }
        }
        model.externs = Some(Arc::new(Externs {
            namespaces: namespaces
                .into_iter()
                .map(|(name, functions)| (name, Arc::new(functions)))
                .collect(),
        }));
        model
    }

    pub fn publish_externs(&self, globals: &mut Globals) {
        globals.externs.clone_from(&self.externs);
    }
}
