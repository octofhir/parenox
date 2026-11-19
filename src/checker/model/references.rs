//! Definite project-namespace call errors in bodies that lack a type check.
use std::collections::BTreeSet;

use crate::Diagnostic;
use crate::checker::bindings::binding_names;
use crate::checker::calls::arity_counts;
use crate::checker::function_forms::{Clause, function_clauses};
use crate::reader::{Form, Kind, Span};

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
            return if self.forward_vars.contains_key(name) {
                ProjectCall::Unknown
            } else {
                ProjectCall::Missing
            };
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
        let top_level_names = forms
            .iter()
            .filter_map(|form| {
                let Kind::List(items) = &form.without_metadata().kind else {
                    return None;
                };
                items.get(1).and_then(Form::symbol)
            })
            .collect();
        let mut diagnostics = Vec::new();
        let mut visit = References {
            names: self,
            top_level_names: &top_level_names,
            locals: Vec::new(),
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
                    visit.clause(clause);
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
    top_level_names: &'a BTreeSet<&'a str>,
    locals: Vec<(&'a str, Span)>,
    inside_test: bool,
    resolve: &'a mut F,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl<'a, F: FnMut(&str, &str, usize) -> ProjectCall> References<'a, F> {
    fn clause(&mut self, clause: &Clause<'a>) {
        let scope = self.locals.len();
        for parameter in clause.parameters() {
            binding_names(parameter, &mut self.locals);
        }
        self.forms(clause.body);
        self.locals.truncate(scope);
    }

    fn forms(&mut self, forms: &'a [Form]) {
        for form in forms {
            match &form.without_metadata().kind {
                Kind::List(items) => self.call(items),
                Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => self.forms(items),
                Kind::AnonymousFunction(function) => self.call(&function.body),
                _ => {}
            }
        }
    }

    fn call(&mut self, items: &'a [Form]) {
        let Some((head, args)) = items.split_first() else {
            return;
        };
        let Some(symbol) = head.symbol() else { return };
        match symbol {
            "if" | "do" | "recur" => return self.forms(args),
            "let*" | "loop*" => return self.bindings(args),
            "fn*" => return self.function(args, head.span),
            _ => {}
        }
        if self.is_local(symbol) {
            self.forms(args);
            return;
        }
        if self.inside_test && self.test_macro(symbol, args) {
            return;
        }
        let qualified = self.names.qualified_reference(symbol);
        let (namespace, local) = self.names.names.resolve_parts(symbol);
        if self.trusted_core(symbol, namespace) {
            match local {
                "let" | "loop" => return self.bindings(args),
                "fn" => return self.function(args, head.span),
                "when" | "when-not" | "if-not" | "and" | "or" | "cond" | "=" | "not=" | "<"
                | "<=" | ">" | ">=" | "+" | "-" | "*" | "inc" | "dec" | "not" | "nil?"
                | "some?" | "str" | "vector" | "list" | "hash-map" | "set" | "get"
                | "contains?" | "assoc" | "conj" | "map" | "mapv" | "filter" | "reduce"
                | "apply" | "subs" | "first" | "next" | "rest" | "count" => {
                    return self.forms(args);
                }
                _ => {}
            }
        }
        if namespace == self.names.namespace()
            || (qualified.is_none()
                && (self.top_level_names.contains(symbol)
                    || !self.names.names.refers.contains_key(symbol)))
        {
            return;
        }
        let span = head.without_metadata().span;
        match (self.resolve)(namespace, local, args.len()) {
            // Missing explicit imports already have a finding on the declaration.
            ProjectCall::Missing if qualified.is_some() => self.diagnostics.push(Diagnostic::new(
                "PNX2304",
                span,
                format!("Unresolved or private project var {symbol}"),
            )),
            ProjectCall::Function { expected_arity } => {
                if let Some(expected) = expected_arity {
                    self.diagnostics.push(Diagnostic::new(
                        "PNX2554",
                        span,
                        format!("{symbol} expects {expected} arguments, got {}", args.len()),
                    ));
                }
                self.forms(args);
            }
            _ => {}
        }
    }

    fn is_local(&self, symbol: &str) -> bool {
        self.locals.iter().rev().any(|(name, _)| *name == symbol)
    }

    fn trusted_core(&self, symbol: &str, namespace: &str) -> bool {
        namespace == "clojure.core"
            && (symbol.contains('/')
                || (self.names.all_refers.is_empty() && !self.top_level_names.contains(symbol)))
    }

    fn test_macro(&mut self, symbol: &str, args: &'a [Form]) -> bool {
        if !symbol.contains('/') && self.top_level_names.contains(symbol) {
            return false;
        }
        match self.names.names.resolve_parts(symbol) {
            ("clojure.test", "testing") => self.forms(args),
            ("clojure.test", "is") => {
                if let Some(assertion) = args.first() {
                    if let Kind::List(items) = &assertion.without_metadata().kind {
                        if let Some((head, values)) = items.split_first()
                            && let Some(name) = head.symbol()
                            && (name.contains('/')
                                || (!self.top_level_names.contains(name) && !self.is_local(name)))
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

    fn function(&mut self, args: &'a [Form], span: Span) {
        let scope = self.locals.len();
        let clauses = if let Some((name, tail)) = args.split_first()
            && let Some(symbol) = name.symbol()
        {
            self.locals.push((symbol, name.without_metadata().span));
            tail
        } else {
            args
        };
        if let Ok(clauses) = function_clauses(clauses, span) {
            for clause in clauses.values() {
                self.clause(clause);
            }
        }
        self.locals.truncate(scope);
    }

    fn bindings(&mut self, args: &'a [Form]) {
        if let Some((bindings, body)) = args.split_first()
            && let Some(bindings) = bindings.vector()
            && let (pairs, []) = bindings.as_chunks::<2>()
        {
            let scope = self.locals.len();
            for [pattern, value] in pairs {
                self.forms(std::slice::from_ref(value));
                binding_names(pattern, &mut self.locals);
            }
            self.forms(body);
            self.locals.truncate(scope);
        }
    }
}
