//! Definite project-namespace call errors in bodies that lack a type check.
use std::collections::BTreeSet;

use crate::Diagnostic;
use crate::checker::calls::arity_counts;
use crate::checker::function_forms::function_clauses;
use crate::reader::{Form, Kind};

use super::contracts::function_parts;
use super::declarations::{SourceDeclaration, symbols_match};
use super::{Declarations, Model};

pub(crate) enum ProjectCall {
    Unknown,
    Missing,
    Function { expected_arity: Option<String> },
}

impl Declarations {
    /// An opaque top-level form or a known namespace mutation prevents
    /// diagnosing absence from the source declarations.
    pub(crate) fn has_known_var_surface(&self, forms: &[Form]) -> bool {
        !self.opaque_var_surface
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
                            | "alter-var-root"
                    )
                })
            })
    }

    pub(crate) fn project_call(&self, forms: &[Form], name: &str, actual: usize) -> ProjectCall {
        let mut declarations = self.source_declarations(forms, name);
        let Some(declaration) = declarations.next() else {
            return ProjectCall::Missing;
        };
        if declarations.next().is_some() {
            return ProjectCall::Unknown;
        }
        if self.source_private(&declaration) {
            return ProjectCall::Missing;
        }
        let expected_arity = match declaration {
            SourceDeclaration::Value(_) => return ProjectCall::Unknown,
            SourceDeclaration::RecordFactory { arity, .. } => {
                (!arity.accepts(actual)).then(|| arity.to_string())
            }
            SourceDeclaration::Function(items) => {
                let (arms, attrs) = function_parts(&items[2..]);
                if [":macro", ":dynamic", ":inline"].iter().any(|key| {
                    attrs
                        .clone()
                        .find_map(|[attribute, value]| {
                            (attribute.keyword() == Some(*key)).then_some(value)
                        })
                        .or_else(|| items[1].metadata(key))
                        .is_some_and(|value| {
                            !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
                        })
                }) {
                    return ProjectCall::Unknown;
                }
                let Ok(clauses) = function_clauses(arms, items[1].span) else {
                    return ProjectCall::Unknown;
                };
                (!clauses.keys().any(|arity| arity.accepts(actual)))
                    .then(|| arity_counts(clauses.keys().copied()))
            }
        };
        ProjectCall::Function { expected_arity }
    }

    pub(crate) fn check_untyped_references(
        &self,
        forms: &[Form],
        model: &Model,
        mut resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Vec<Diagnostic> {
        // A PNX9002 at the definition means no typed root checks this body.
        let unchecked: BTreeSet<_> = model
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "PNX9002")
            .map(|diagnostic| diagnostic.span.start.offset)
            .collect();
        if unchecked.is_empty() && self.test_forms.is_empty() {
            return Vec::new();
        }
        let top_level_names = forms.iter().filter_map(|form| {
            let Kind::List(items) = &form.without_metadata().kind else {
                return None;
            };
            items.get(1).and_then(Form::symbol)
        });
        let shadowed_let = top_level_names.clone().any(|name| name == "let");
        let test_names = if self.test_forms.is_empty() {
            BTreeSet::new()
        } else {
            top_level_names.collect()
        };
        let core_let = self.all_refers.is_empty()
            && model.names.resolve_parts("let").0 == "clojure.core"
            && !shadowed_let;
        let mut diagnostics = Vec::new();
        let mut visit = References {
            names: self,
            core_let,
            test_names: &test_names,
            inside_test: false,
            resolve: &mut resolve,
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
        visit.inside_test = true;
        for &index in &self.test_forms {
            if let Kind::List(items) = &forms[index].without_metadata().kind {
                visit.forms(&items[2..]);
            }
        }
        diagnostics
    }
}

struct References<'a, F> {
    names: &'a Declarations,
    core_let: bool,
    test_names: &'a BTreeSet<&'a str>,
    inside_test: bool,
    resolve: &'a mut F,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl<F: FnMut(&str, &str, usize) -> ProjectCall> References<'_, F> {
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
        if self.inside_test && self.test_macro(symbol, args) {
            return;
        }
        match symbol {
            "if" | "do" => self.forms(args),
            "let*" | "clojure.core/let" => self.bindings(args),
            "let" if self.core_let => self.bindings(args),
            _ => {
                if let Some((alias, _)) = symbol.split_once('/')
                    && self.names.names.aliases.contains_key(alias)
                    && let Some((namespace, local)) = self.names.qualified_reference(symbol)
                    && namespace != self.names.namespace()
                {
                    let span = head.without_metadata().span;
                    match (self.resolve)(namespace, local, args.len()) {
                        ProjectCall::Unknown => {}
                        ProjectCall::Missing => self.diagnostics.push(Diagnostic::new(
                            "PNX2304",
                            span,
                            format!("Unresolved or private project var {symbol}"),
                        )),
                        ProjectCall::Function { expected_arity } => {
                            if let Some(expected) = expected_arity {
                                self.diagnostics.push(Diagnostic::new(
                                    "PNX2554",
                                    span,
                                    format!(
                                        "{symbol} expects {expected} arguments, got {}",
                                        args.len()
                                    ),
                                ));
                            }
                            self.forms(args);
                        }
                    }
                }
            }
        }
    }

    fn test_macro(&mut self, symbol: &str, args: &[Form]) -> bool {
        if !symbol.contains('/') && self.test_names.contains(symbol) {
            return false;
        }
        match self.names.names.resolve_parts(symbol) {
            ("clojure.test", "testing") => self.forms(args),
            ("clojure.test", "is") => {
                if let Some(assertion) = args.first() {
                    if let Kind::List(items) = &assertion.without_metadata().kind {
                        if let Some((head, values)) = items.split_first()
                            && let Some(name) = head.symbol()
                            && (name.contains('/') || !self.test_names.contains(name))
                            && matches!(
                                self.names.names.resolve_parts(name),
                                ("clojure.core", "=" | "nil?" | "some?" | "true?" | "false?")
                            )
                        {
                            self.forms(values);
                        }
                        // Other assertion forms can have user-defined expansion.
                    } else {
                        self.forms(std::slice::from_ref(assertion));
                    }
                }
            }
            _ => return false,
        }
        true
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
