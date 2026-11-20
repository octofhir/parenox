//! Validate externally declared promises against public source contracts.
use super::{
    Contract, ExternCache, ExternFile, ExternalDeclaration, Globals, LiteralExtern,
    literals::SourceLiterals,
};
use crate::Diagnostic;
use crate::reader::Span;
use crate::types::{Compatibility, Type, extern_drift};

impl ExternCache {
    pub fn has_literals(&self) -> bool {
        self.table.as_ref().is_some_and(|table| {
            table.namespaces.values().any(|declarations| {
                declarations
                    .values()
                    .any(|decl| matches!(decl, ExternalDeclaration::Literal(_)))
            })
        })
    }

    pub fn validate_file(
        &self,
        file: &ExternFile,
        globals: &Globals,
        source_literals: &SourceLiterals<'_>,
    ) -> Vec<Diagnostic> {
        let Some(merged) = &self.table else {
            return Vec::new();
        };
        let mut diagnostics = Vec::new();
        for (namespace, declarations) in &file.table.namespaces {
            let Some(active) = merged.namespaces.get(namespace) else {
                continue;
            };
            let source = globals.exports.get(namespace);
            for (name, raw) in declarations.iter() {
                let Some(declaration) = active
                    .get(name)
                    .filter(|decl| decl.origin() == raw.origin())
                else {
                    continue; // Unresolved or conflicting promises already have diagnostics.
                };
                let span = file.spans[name];
                let diagnostic = match declaration {
                    ExternalDeclaration::Literal(promised) => {
                        validate_literal(promised, span, source_literals)
                    }
                    ExternalDeclaration::Function(promised) => validate_function(
                        promised,
                        source.and_then(|exports| exports.functions.get(name)),
                        span,
                    ),
                    ExternalDeclaration::Spec(promised) => {
                        let registration = globals
                            .spec_provider(name)
                            .and_then(|provider| provider.standalone_registration());
                        if registration.is_some_and(|source| {
                            source.namespace == promised.registration.namespace
                        }) && globals.spec_conversions.get(name) == Some(&promised.conversion)
                        {
                            None
                        } else {
                            Some(Diagnostic::new(
                                "PNX9011",
                                span,
                                format!(
                                    "Cannot verify extern spec {name} against the source registration in {}",
                                    promised.registration.namespace
                                ),
                            ))
                        }
                    }
                    ExternalDeclaration::Macro(_) => Some(Diagnostic::new(
                        "PNX9011",
                        span,
                        format!("Cannot validate macro scope {name} against source"),
                    )),
                    ExternalDeclaration::PendingSpec(_) => {
                        unreachable!("merged externs are resolved")
                    }
                };
                let Some(mut diagnostic) = diagnostic else {
                    continue;
                };
                diagnostic.contract = Some(declaration.origin().clone());
                diagnostics.push(diagnostic);
            }
        }
        diagnostics
    }
}

fn validate_literal(
    promised: &LiteralExtern,
    span: Span,
    sources: &SourceLiterals<'_>,
) -> Option<Diagnostic> {
    let name = &promised.origin.name;
    let Some((path, actual)) = sources.get(name) else {
        return Some(Diagnostic::new(
            "PNX9011",
            span,
            format!("No checked public source literal for extern {name}"),
        ));
    };
    let mut diagnostic = match actual {
        Ok(actual) if actual == &promised.edn => return None,
        Ok(actual) => {
            let mut diagnostic = Diagnostic::new(
                "PNX2322",
                span,
                format!(
                    "Extern {name}: expected literal {}, got {actual}",
                    promised.edn
                ),
            );
            diagnostic.expected = Some(promised.edn.clone());
            diagnostic.actual = Some(actual.clone());
            diagnostic
        }
        Err(reason) => Diagnostic::new(
            "PNX9011",
            span,
            format!("Cannot validate literal extern {name}: {reason}"),
        ),
    };
    diagnostic
        .notes
        .push(format!("Public source literal declared in {path}."));
    Some(diagnostic)
}

fn validate_function(
    promised: &Contract,
    source: Option<&Contract>,
    span: Span,
) -> Option<Diagnostic> {
    let name = &promised.origin.name;
    let Some(source) = source else {
        return Some(Diagnostic::new(
            "PNX9011",
            span,
            format!("No checked public source function for extern {name}"),
        ));
    };
    let (count, outcome, detail) = extern_drift(&source.signatures, &promised.signatures)?;
    let site = format!(
        "Extern {name} at {count} argument{}",
        if count == 1 { "" } else { "s" }
    );
    let expected = Type::Function(promised.signatures.clone()).to_string();
    let actual = Type::Function(source.signatures.clone()).to_string();
    let mut diagnostic = match outcome {
        Compatibility::Mismatch => Diagnostic::type_mismatch(span, &site, expected, actual),
        _ => {
            let mut diagnostic = Diagnostic::new(
                "PNX9011",
                span,
                format!("{site}: cannot prove that the source contract satisfies this extern"),
            );
            diagnostic.expected = Some(expected);
            diagnostic.actual = Some(actual);
            diagnostic
        }
    };
    diagnostic.notes.push(detail.to_string());
    if let Some(path) = &source.origin.path {
        diagnostic
            .notes
            .push(format!("Public source contract declared in {path}."));
    }
    Some(diagnostic)
}
