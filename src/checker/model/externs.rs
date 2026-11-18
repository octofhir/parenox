//! Immutable EDN declarations shared with each project snapshot.
use std::collections::{BTreeMap, btree_map::Entry};
use std::sync::Arc;

use super::{Contract, Globals, Model, Names};
use crate::checker::calls::is_modeled_call;
use crate::reader::{Form, Kind, Span};
use crate::{ContractKind, ContractOrigin, Diagnostic};

pub(super) struct Externs {
    pub(super) namespaces: BTreeMap<String, Arc<ExternalDeclarations>>,
}

pub(super) type ExternalDeclarations = BTreeMap<String, ExternalDeclaration>;

#[derive(PartialEq, Eq)]
pub(super) enum ExternalDeclaration {
    Function(Contract),
    Macro(MacroScope),
}

#[derive(PartialEq, Eq)]
pub(crate) enum MacroScope {
    LetLike(Box<ContractOrigin>),
}

impl MacroScope {
    pub fn origin(&self) -> &ContractOrigin {
        match self {
            Self::LetLike(origin) => origin,
        }
    }
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
                "Externs must be one EDN map of qualified names to signatures or macro scopes",
            );
            return model;
        };
        let mut namespaces = BTreeMap::<String, ExternalDeclarations>::new();
        for [name, declaration] in entries.as_chunks::<2>().0 {
            let Some(name_string) = name.symbol().filter(|symbol| {
                symbol
                    .split_once('/')
                    .is_some_and(|(namespace, local)| !namespace.is_empty() && !local.is_empty())
            }) else {
                model.error(
                    "PNX9001",
                    name.span,
                    "Extern names must be qualified symbols",
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
            let target = namespaces
                .entry(namespace.into())
                .or_default()
                .entry(name_string.to_owned());
            let Entry::Vacant(target) = target else {
                model.error(
                    "PNX2790",
                    name.span,
                    format!("Duplicate extern {name_string}"),
                );
                continue;
            };
            let parsed = if let Some(entries) = declaration.map() {
                let [kind, archetype] = entries else {
                    model.error(
                        "PNX9001",
                        declaration.span,
                        "Expected a macro scope declaration {:macro :let-like}",
                    );
                    continue;
                };
                if kind.keyword() != Some(":macro") || archetype.keyword() != Some(":let-like") {
                    model.error(
                        "PNX9001",
                        declaration.span,
                        "Expected a macro scope declaration {:macro :let-like}",
                    );
                    continue;
                }
                Some(ExternalDeclaration::Macro(MacroScope::LetLike(Box::new(
                    ContractOrigin {
                        name: name_string.into(),
                        kind: ContractKind::MacroScope,
                        path: Some(path.into()),
                    },
                ))))
            } else {
                model
                    .parse_signatures(declaration, name_string, globals)
                    .map(|signatures| {
                        ExternalDeclaration::Function(Contract::new(
                            signatures,
                            name_string,
                            ContractKind::Extern,
                            path,
                        ))
                    })
            };
            if let Some(parsed) = parsed {
                target.insert(parsed);
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
