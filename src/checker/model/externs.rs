//! Immutable EDN declarations shared with each project snapshot.
mod diff;
mod encode;
mod literals;
mod validation;

use literals::LiteralExtern;

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::sync::Arc;

use super::{Contract, Globals, Model, Names};
use crate::checker::calls::is_modeled_call;
use crate::reader::{Form, Kind, Span};
use crate::{ContractKind, ContractOrigin, Diagnostic};

pub(super) struct Externs {
    pub(super) namespaces: BTreeMap<String, Arc<ExternalDeclarations>>,
}

pub(super) type ExternalDeclarations = BTreeMap<String, ExternalDeclaration>;

pub(crate) struct ExternFile {
    path: String,
    table: Arc<Externs>,
    spans: BTreeMap<String, Span>,
}

#[derive(Default)]
pub(crate) struct ExternCache {
    files: Vec<Arc<ExternFile>>,
    table: Option<Arc<Externs>>,
    conflicts: BTreeMap<String, Vec<Diagnostic>>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum ExternalDeclaration {
    Function(Contract),
    Macro(MacroScope),
    Literal(Arc<LiteralExtern>),
}

impl ExternalDeclaration {
    fn origin(&self) -> &ContractOrigin {
        match self {
            Self::Function(contract) => &contract.origin,
            Self::Macro(scope) => scope.origin(),
            Self::Literal(literal) => &literal.origin,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
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
    pub fn externs(forms: &[Form], path: &str, source: &str, globals: &Globals) -> Self {
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
                "Externs must be one EDN map of qualified names to signatures, literal values or macro scopes",
            );
            return model;
        };
        let mut namespaces = BTreeMap::<String, ExternalDeclarations>::new();
        let mut spans = BTreeMap::new();
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
                match entries {
                    [kind, value] if kind.keyword() == Some(":value") => {
                        model.read_extern_literal(value, name_string, path, source, globals)
                    }
                    [kind, shape]
                        if kind.keyword() == Some(":macro")
                            && shape.keyword() == Some(":let-like") =>
                    {
                        Some(ExternalDeclaration::Macro(MacroScope::LetLike(Box::new(
                            ContractOrigin {
                                name: name_string.into(),
                                kind: ContractKind::MacroScope,
                                path: Some(path.into()),
                            },
                        ))))
                    }
                    _ => {
                        model.error(
                            "PNX9001",
                            declaration.span,
                            "Expected {:value <EDN data>} or {:macro :let-like}",
                        );
                        None
                    }
                }
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
                spans.insert(name_string.into(), name.span);
            }
        }
        model.externs = Some(Arc::new(ExternFile {
            path: path.into(),
            table: Arc::new(Externs {
                namespaces: namespaces
                    .into_iter()
                    .map(|(name, functions)| (name, Arc::new(functions)))
                    .collect(),
            }),
            spans,
        }));
        model
    }

    pub fn extern_file(&self) -> Option<&Arc<ExternFile>> {
        self.externs.as_ref()
    }
}

impl ExternCache {
    pub fn update<'a>(&mut self, files: impl Iterator<Item = &'a Arc<ExternFile>> + Clone) {
        if !files
            .clone()
            .map(Arc::as_ptr)
            .eq(self.files.iter().map(Arc::as_ptr))
        {
            self.files = files.cloned().collect();
            self.conflicts.clear();
            self.table = match self.files.as_slice() {
                [] => None,
                [file] => Some(file.table.clone()),
                files => Some(Arc::new(merge(
                    files,
                    self.table.as_deref(),
                    &mut self.conflicts,
                ))),
            };
        }
    }

    pub fn publish(&self, globals: &mut Globals) {
        globals.externs.clone_from(&self.table);
    }

    pub fn publish_dependencies(&self, paths: &BTreeSet<&str>, globals: &mut Globals) {
        let Some(table) = &self.table else {
            return;
        };
        let trusted = |declaration: &ExternalDeclaration| {
            declaration
                .origin()
                .path
                .as_deref()
                .is_some_and(|path| paths.contains(path))
        };
        let namespaces = table
            .namespaces
            .iter()
            .filter_map(|(name, declarations)| {
                if declarations.values().all(trusted) {
                    return Some((name.clone(), declarations.clone()));
                }
                let subset: ExternalDeclarations = declarations
                    .iter()
                    .filter(|(_, decl)| trusted(decl))
                    .map(|(name, decl)| (name.clone(), decl.clone()))
                    .collect();
                (!subset.is_empty()).then(|| (name.clone(), Arc::new(subset)))
            })
            .collect();
        globals.externs = Some(Arc::new(Externs { namespaces }));
    }

    pub fn diagnostics(&self, path: &str) -> &[Diagnostic] {
        self.conflicts.get(path).map_or(&[], Vec::as_slice)
    }
}

fn merge(
    files: &[Arc<ExternFile>],
    before: Option<&Externs>,
    conflicts: &mut BTreeMap<String, Vec<Diagnostic>>,
) -> Externs {
    let mut files: Vec<_> = files.iter().collect();
    // Report both sides of a collision regardless of the argument order.
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut by_namespace = BTreeMap::<_, Vec<_>>::new();
    for file in files {
        for (name, table) in &file.table.namespaces {
            by_namespace
                .entry(name)
                .or_default()
                .push((file.as_ref(), table));
        }
    }
    let namespaces = by_namespace
        .into_iter()
        .filter_map(|(name, tables)| {
            let table = if let [(_, table)] = tables.as_slice() {
                Arc::clone(table)
            } else {
                let merged = merge_namespace(tables, conflicts);
                if merged.is_empty() {
                    return None;
                }
                before
                    .and_then(|before| before.namespaces.get(name))
                    .filter(|previous| previous.as_ref() == &merged)
                    .map_or_else(|| Arc::new(merged), Arc::clone)
            };
            Some((name.clone(), table))
        })
        .collect();
    Externs { namespaces }
}

fn merge_namespace(
    tables: Vec<(&ExternFile, &Arc<ExternalDeclarations>)>,
    conflicts: &mut BTreeMap<String, Vec<Diagnostic>>,
) -> ExternalDeclarations {
    let mut seen = BTreeMap::<&str, (&ExternFile, &ExternalDeclaration, bool)>::new();
    for (file, table) in tables {
        for (name, declaration) in table.iter() {
            match seen.entry(name) {
                Entry::Vacant(entry) => {
                    entry.insert((file, declaration, false));
                }
                Entry::Occupied(mut entry) => {
                    let (first, original, duplicate) = entry.get_mut();
                    if !*duplicate {
                        record_conflict(conflicts, first, original, file);
                        *duplicate = true;
                    }
                    record_conflict(conflicts, file, declaration, first);
                }
            }
        }
    }
    seen.into_iter()
        .filter(|(_, (_, _, duplicate))| !duplicate)
        .map(|(name, (_, declaration, _))| (name.to_owned(), declaration.clone()))
        .collect()
}

fn record_conflict(
    conflicts: &mut BTreeMap<String, Vec<Diagnostic>>,
    file: &ExternFile,
    declaration: &ExternalDeclaration,
    other: &ExternFile,
) {
    let origin = declaration.origin();
    let mut diagnostic = Diagnostic::new(
        "PNX2790",
        file.spans[&origin.name],
        format!("Duplicate extern {}", origin.name),
    );
    diagnostic.contract = Some(origin.clone());
    diagnostic.notes.push(format!(
        "Also declared in {}. Conflicting declarations are not used.",
        other.path
    ));
    conflicts
        .entry(file.path.clone())
        .or_default()
        .push(diagnostic);
}
