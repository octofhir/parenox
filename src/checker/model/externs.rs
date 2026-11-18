//! Immutable EDN declarations shared with each project snapshot.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::interfaces::Contracts;
use super::{Contract, Globals, Model, Names};
use crate::checker::calls::is_modeled_call;
use crate::reader::{Form, Kind, Span};
use crate::{ContractKind, ContractOrigin, Diagnostic};

pub(super) struct Externs {
    pub(super) namespaces: BTreeMap<String, Arc<Contracts>>,
}

impl Model {
    pub fn externs(forms: &[Form], path: &str, globals: &Globals) -> Self {
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
        let mut namespaces = BTreeMap::<String, Contracts>::new();
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
            if is_modeled_call(name_string) {
                let mut diagnostic = Diagnostic::new(
                    "PNX2790",
                    name.span,
                    format!("Extern {name_string} cannot replace a built-in checking rule"),
                );
                diagnostic.contract = Some(ContractOrigin::builtin(name_string));
                model.diagnostics.push(diagnostic);
                continue;
            }
            let (namespace, _) = name_string.split_once('/').unwrap();
            if let Some(signatures) = model.parse_signatures(signature, globals)
                && namespaces
                    .entry(namespace.into())
                    .or_default()
                    .insert(
                        name_string.to_owned(),
                        Contract::new(signatures, name_string, ContractKind::Extern, path),
                    )
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
