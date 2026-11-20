//! Compare resolved extern declarations without consulting implementation bodies.
use std::collections::{BTreeMap, BTreeSet};

use super::{ExternCache, ExternalDeclaration, MacroScope};
use crate::reader::Span;
use crate::types::{Compatibility, Type, extern_drift};
use crate::{ExternChange, ExternChangeKind as Change, ExternDefinition, ExternMismatch};

impl ExternCache {
    pub fn changes_from(&self, before: &Self) -> Vec<ExternChange> {
        let before = before.surface();
        let after = self.surface();
        let names: BTreeSet<_> = before.keys().chain(after.keys()).copied().collect();
        names
            .into_iter()
            .filter_map(|name| {
                let old = before.get(name).copied();
                let new = after.get(name).copied();
                let (kind, mismatch) = match (old, new) {
                    (None, Some(_)) => (Change::Added, None),
                    (Some(_), None) => (Change::Removed, None),
                    (Some((old, _)), Some((new, _))) => changed(old, new)?,
                    (None, None) => unreachable!(),
                };
                let (argument_count, detail) =
                    mismatch.map_or((None, None), |(count, detail)| (Some(count), Some(detail)));
                Some(ExternChange {
                    name: name.into(),
                    kind,
                    argument_count,
                    detail,
                    before: old.map(definition),
                    after: new.map(definition),
                })
            })
            .collect()
    }

    fn surface(&self) -> BTreeMap<&str, (&ExternalDeclaration, Span)> {
        let spans: BTreeMap<_, _> = self
            .files
            .iter()
            .flat_map(|file| file.spans.iter())
            .collect();
        self.table
            .iter()
            .flat_map(|table| table.namespaces.values())
            .flat_map(|table| {
                table
                    .iter()
                    .map(|(name, declaration)| (name.as_str(), (declaration, *spans[name])))
            })
            .collect()
    }
}

fn changed(
    old: &ExternalDeclaration,
    new: &ExternalDeclaration,
) -> Option<(Change, Option<(usize, ExternMismatch)>)> {
    use ExternalDeclaration::{Function, Literal, Macro, Spec};
    match (old, new) {
        (Function(old), Function(new)) => {
            if old.signatures.len() == new.signatures.len()
                && old
                    .signatures
                    .iter()
                    .zip(new.signatures.iter())
                    .all(|(a, b)| a.same_type(b))
            {
                return None;
            }
            Some(match extern_drift(&new.signatures, &old.signatures) {
                None => (Change::Compatible, None),
                Some((count, Compatibility::Mismatch, detail)) => {
                    (Change::Breaking, Some((count, detail)))
                }
                Some((count, _, detail)) => (Change::Unproven, Some((count, detail))),
            })
        }
        (Macro(MacroScope::LetLike(_)), Macro(MacroScope::LetLike(_))) => None,
        (Literal(old), Literal(new)) => {
            (old.edn != new.edn).then_some((Change::ValueChanged, None))
        }
        (Spec(old), Spec(new)) => (old.conversion != new.conversion
            || old.registration.namespace != new.registration.namespace)
            .then_some((Change::Unproven, None)),
        _ => Some((Change::Breaking, None)),
    }
}

fn definition((extern_, span): (&ExternalDeclaration, Span)) -> ExternDefinition {
    let declaration = match extern_ {
        ExternalDeclaration::Function(contract) => {
            Type::Function(contract.signatures.clone()).to_string()
        }
        ExternalDeclaration::Macro(MacroScope::LetLike(_)) => "{:macro :let-like}".into(),
        ExternalDeclaration::Literal(literal) => format!("{{:value {}}}", literal.edn),
        ExternalDeclaration::Spec(spec) => {
            let result = spec
                .conversion
                .proposal_result()
                .expect("checked extern spec");
            format!(
                "{} -> {result} (registered by {})",
                spec.conversion.success_input(),
                spec.registration.namespace
            )
        }
        ExternalDeclaration::PendingSpec(_) => unreachable!("merged externs are resolved"),
    };
    ExternDefinition {
        origin: extern_.origin().clone(),
        span,
        declaration,
    }
}
