//! Definite project-namespace call errors in bodies that lack a type check.
use std::collections::BTreeSet;

use crate::Diagnostic;
use crate::checker::function_forms::function_clauses;
use crate::reader::{Form, Kind};

use super::contracts::function_parts;
use super::declarations::symbols_match;
use super::{Declarations, Model};

impl Declarations {
    /// An opaque top-level form or a known namespace mutation prevents
    /// diagnosing absence from the source declarations.
    pub(crate) fn has_known_var_surface(&self, forms: &[Form]) -> bool {
        self.diagnostics.is_empty()
            && forms.iter().all(|form| {
                symbols_match(form, &mut |symbol| {
                    let name = symbol.rsplit('/').next().unwrap_or(symbol);
                    !matches!(
                        name,
                        "intern"
                            | "eval"
                            | "load"
                            | "load-file"
                            | "load-string"
                            | "in-ns"
                            | "ns-unmap"
                    )
                })
            })
    }

    pub(crate) fn check_untyped_references(
        &self,
        forms: &[Form],
        model: &Model,
        mut unresolved: impl FnMut(&str, &str) -> bool,
    ) -> Vec<Diagnostic> {
        // A PNX9002 at the definition means no typed root checks this body.
        let unchecked: BTreeSet<_> = model
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "PNX9002")
            .map(|diagnostic| diagnostic.span.start.offset)
            .collect();
        if unchecked.is_empty() {
            return Vec::new();
        }
        let core_let = self.all_refers.is_empty()
            && model.names.resolve_parts("let").0 == "clojure.core"
            && !forms.iter().any(|form| {
                let Kind::List(items) = &form.without_metadata().kind else {
                    return false;
                };
                items.get(1).and_then(Form::symbol) == Some("let")
            });
        let mut diagnostics = Vec::new();
        let mut visit = References {
            names: self,
            core_let,
            unresolved: &mut unresolved,
            diagnostics: &mut diagnostics,
        };
        for &index in &self.functions {
            let form = &forms[index];
            if unchecked.contains(&form.span.start.offset)
                && let Kind::List(items) = &form.without_metadata().kind
                && let Some(tail) = items.get(2..)
                && let Ok(clauses) = function_clauses(function_parts(tail).0, form.span)
            {
                for clause in clauses.values() {
                    visit.forms(clause.body);
                }
            }
        }
        diagnostics
    }
}

struct References<'a, F> {
    names: &'a Declarations,
    core_let: bool,
    unresolved: &'a mut F,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl<F: FnMut(&str, &str) -> bool> References<'_, F> {
    fn forms(&mut self, forms: &[Form]) {
        for form in forms {
            match &form.without_metadata().kind {
                Kind::List(items) => self.call(items),
                Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => self.forms(items),
                Kind::AnonymousFunction(function) => self.call(&function.body),
                _ => {}
            }
        }
    }

    fn call(&mut self, items: &[Form]) {
        let Some((head, args)) = items.split_first() else {
            return;
        };
        let Some(symbol) = head.symbol() else { return };
        match symbol {
            "if" | "do" => self.forms(args),
            "let*" | "clojure.core/let" => self.bindings(args),
            "let" if self.core_let => self.bindings(args),
            _ => {
                // Argument evaluation of other macros and calls is left to
                // the typed checker. Inspect only this call head.
                if let Some((alias, _)) = symbol.split_once('/')
                    && self.names.names.aliases.contains_key(alias)
                    && let Some((namespace, local)) = self.names.qualified_reference(symbol)
                    && namespace != self.names.namespace()
                    && (self.unresolved)(namespace, local)
                {
                    self.diagnostics.push(Diagnostic::new(
                        "PNX2304",
                        head.without_metadata().span,
                        format!("Unresolved or private project var {symbol}"),
                    ));
                }
            }
        }
    }

    fn bindings(&mut self, args: &[Form]) {
        if let Some((bindings, body)) = args.split_first()
            && let Some(bindings) = bindings.vector()
            && let (pairs, []) = bindings.as_chunks::<2>()
        {
            for [_, value] in pairs {
                self.forms(std::slice::from_ref(value));
            }
            self.forms(body);
        }
    }
}
