//! Definite project-namespace call errors in bodies that lack a type check.
use std::collections::BTreeSet;

use crate::Diagnostic;
use crate::checker::bindings::binding_names;
use crate::checker::calls::arity_counts;
use crate::checker::function_forms::{Clause, function_clauses, lexical_function_clauses};
use crate::reader::{Form, Kind, Span};

use super::contracts::function_parts;
use super::declarations::{SourceDeclaration, symbols_match};
use super::{Declarations, Model};

pub(crate) enum ProjectCall {
    Unknown,
    Missing,
    Function { expected_arity: Option<String> },
}

pub(crate) enum ScopedReference<'a> {
    Local(Span),
    Referred(&'a str, &'a str),
    InNamespace(&'a str),
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
            SourceDeclaration::Value(_) | SourceDeclaration::Protocol(_) => {
                return ProjectCall::Unknown;
            }
            SourceDeclaration::TypeFactory { arity, .. } => {
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
        standard_test: bool,
        mut resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Vec<Diagnostic> {
        let mut unchecked = self.unchecked_definitions(forms, model).peekable();
        if unchecked.peek().is_none() && self.test_forms.is_empty() && self.method_forms.is_empty()
        {
            return Vec::new();
        }
        let top_level_names = self.top_level_names(forms);
        let mut diagnostics = Vec::new();
        let mut visit = References {
            names: self,
            top_level_names: &top_level_names,
            locals: Vec::new(),
            inside_test: false,
            standard_test,
            template_scope: None,
            resolve: &mut resolve,
            purpose: Purpose::Check(&mut diagnostics),
        };
        for form in unchecked {
            visit.definition(form);
        }
        for &index in &self.method_forms {
            visit.method(&forms[index]);
        }
        visit.inside_test = true;
        for &index in &self.test_forms {
            if let Kind::List(items) = &forms[index].without_metadata().kind {
                visit.forms(&items[2..]);
            }
        }
        diagnostics
    }

    pub(super) fn unchecked_definitions<'a>(
        &'a self,
        forms: &'a [Form],
        model: &Model,
    ) -> impl Iterator<Item = &'a Form> {
        // A PNX9002 at the definition means no typed root checks this body.
        let unchecked: BTreeSet<_> = model
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "PNX9002")
            .map(|diagnostic| diagnostic.span.start.offset)
            .collect();
        self.functions
            .iter()
            .map(|&index| &forms[index])
            .filter(move |form| unchecked.contains(&form.span.start.offset))
    }

    pub(crate) fn scoped_reference_at<'a>(
        &'a self,
        forms: &'a [Form],
        offset: usize,
        standard_test: bool,
        mut resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Option<ScopedReference<'a>> {
        let index = forms.partition_point(|form| form.span.end.offset <= offset);
        let form = forms.get(index)?;
        let inside_test = self.test_forms.binary_search(&index).is_ok();
        let inside_method = self.method_forms.binary_search(&index).is_ok();
        if !inside_test && !inside_method && self.functions.binary_search(&index).is_err() {
            return None;
        }
        let top_level_names = self.top_level_names(forms);
        let mut visit = References {
            names: self,
            top_level_names: &top_level_names,
            locals: Vec::new(),
            inside_test,
            standard_test,
            template_scope: None,
            resolve: &mut resolve,
            purpose: Purpose::Navigate {
                offset,
                target: None,
            },
        };
        if inside_test {
            if let Kind::List(items) = &form.without_metadata().kind {
                visit.forms(&items[2..]);
            }
        } else if inside_method {
            visit.method(form);
        } else {
            visit.definition(form);
        }
        match visit.purpose {
            Purpose::Navigate { target, .. } => target,
            Purpose::Check(_) => unreachable!(),
        }
    }

    pub(super) fn top_level_names<'a>(&'a self, forms: &'a [Form]) -> BTreeSet<&'a str> {
        forms
            .iter()
            .filter_map(|form| {
                let Kind::List(items) = &form.without_metadata().kind else {
                    return None;
                };
                items.get(1).and_then(Form::symbol)
            })
            .chain(
                self.scoped_functions(forms)
                    .filter_map(|items| items[1].symbol()),
            )
            .chain(
                self.protocol_vars(forms)
                    .filter_map(|var| var.name.symbol()),
            )
            .collect()
    }
}

enum Purpose<'source, 'query> {
    Check(&'query mut Vec<Diagnostic>),
    Navigate {
        offset: usize,
        target: Option<ScopedReference<'source>>,
    },
}

struct References<'source, 'query, F> {
    names: &'source Declarations,
    top_level_names: &'query BTreeSet<&'source str>,
    locals: Vec<(&'source str, Span)>,
    inside_test: bool,
    standard_test: bool,
    template_scope: Option<usize>,
    resolve: &'query mut F,
    purpose: Purpose<'source, 'query>,
}

impl<'a, F: FnMut(&str, &str, usize) -> ProjectCall> References<'a, '_, F> {
    fn definition(&mut self, form: &'a Form) {
        if let Kind::List(items) = &form.without_metadata().kind
            && let Some(tail) = items.get(2..)
            && let Ok(clauses) = lexical_function_clauses(function_parts(tail).0, form.span)
        {
            for clause in clauses.values() {
                self.clause(clause);
            }
        }
    }

    fn clause(&mut self, clause: &Clause<'a>) {
        let scope = self.locals.len();
        for parameter in clause.parameters() {
            binding_names(parameter, &mut self.locals);
        }
        self.forms(clause.body);
        self.locals.truncate(scope);
    }

    fn method(&mut self, form: &'a Form) {
        if let Kind::List(items) = &form.without_metadata().kind
            && let [_, declaration @ ..] = items.as_slice()
            && let [target, dispatch, tail @ ..] = declaration
        {
            // The dispatch value is evaluated before the method's bindings exist.
            self.forms(std::slice::from_ref(target));
            self.forms(std::slice::from_ref(dispatch));
            self.function(tail, form.span);
        }
    }

    fn forms(&mut self, forms: &'a [Form]) {
        for form in forms {
            if let Purpose::Navigate { offset, .. } = self.purpose
                && !(form.span.start.offset..form.span.end.offset).contains(&offset)
            {
                continue;
            }
            match &form.without_metadata().kind {
                Kind::List(items) => self.call(items),
                Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => self.forms(items),
                Kind::AnonymousFunction(function) => self.call(&function.body),
                Kind::Symbol(symbol) => self.select_reference(symbol, form.without_metadata().span),
                _ => {}
            }
        }
    }

    fn call(&mut self, items: &'a [Form]) {
        let Some((head, args)) = items.split_first() else {
            return;
        };
        let Some(symbol) = head.symbol() else {
            return self.forms(items);
        };
        let span = head.without_metadata().span;
        if self.template_parameter(symbol) {
            // Substitution may turn an operator into a macro or special form.
            self.select_reference(symbol, span);
            return;
        }
        match symbol {
            "if" | "do" | "recur" => return self.forms(args),
            "let*" | "loop*" => return self.bindings(args),
            "fn*" => return self.function(args, head.span),
            _ => {}
        }
        self.select_reference(symbol, span);
        if matches!(
            self.purpose,
            Purpose::Navigate {
                target: Some(_),
                ..
            }
        ) {
            return;
        }
        if self.is_local(symbol) {
            self.forms(args);
            return;
        }
        if self.inside_test && self.standard_test && self.test_macro(symbol, args) {
            return;
        }
        let qualified = self.names.qualified_reference(symbol);
        let (namespace, local) = self.names.names.resolve_parts(symbol);
        if self.trusted_core(symbol, namespace) {
            match local {
                "let" | "loop" => return self.bindings(args),
                "if-let" | "if-some" => return self.conditional_binding(args, true),
                "when-let" | "when-some" | "when-first" => {
                    return self.conditional_binding(args, false);
                }
                "fn" => return self.function(args, head.span),
                "letfn" => return self.local_functions(args),
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
        match (self.resolve)(namespace, local, args.len()) {
            // Missing explicit imports already have a finding on the declaration.
            ProjectCall::Missing if qualified.is_some() => {
                if let Purpose::Check(diagnostics) = &mut self.purpose {
                    diagnostics.push(Diagnostic::new(
                        "PNX2304",
                        span,
                        format!("Unresolved or private project var {symbol}"),
                    ));
                }
            }
            ProjectCall::Function { expected_arity } => {
                if let Some(expected) = expected_arity
                    && let Purpose::Check(diagnostics) = &mut self.purpose
                {
                    diagnostics.push(Diagnostic::new(
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

    fn select_reference(&mut self, symbol: &'a str, span: Span) {
        if let Purpose::Navigate { offset, target } = &mut self.purpose
            && (span.start.offset..span.end.offset).contains(offset)
        {
            *target = self
                .locals
                .iter()
                .rev()
                .find_map(|(name, binding)| {
                    (*name == symbol).then_some(ScopedReference::Local(*binding))
                })
                .or_else(|| {
                    if !self.top_level_names.contains(symbol)
                        && let Some((namespace, local)) = self
                            .names
                            .names
                            .refers
                            .get(symbol)
                            .and_then(|target| target.split_once('/'))
                    {
                        return Some(ScopedReference::Referred(namespace, local));
                    }
                    Some(ScopedReference::InNamespace(symbol))
                });
        }
    }

    fn is_local(&self, symbol: &str) -> bool {
        self.locals.iter().rev().any(|(name, _)| *name == symbol)
    }

    fn template_parameter(&self, symbol: &str) -> bool {
        self.template_scope
            .is_some_and(|scope| self.locals[scope..].iter().any(|(name, _)| *name == symbol))
    }

    fn trusted_core(&self, symbol: &str, namespace: &str) -> bool {
        let refers = &self.names.all_refers;
        let known = refers.is_empty()
            || (self.standard_test && refers.len() == 1 && refers.contains("clojure.test"));
        namespace == "clojure.core"
            && (symbol.contains('/') || (known && !self.top_level_names.contains(symbol)))
    }

    fn test_macro(&mut self, symbol: &str, args: &'a [Form]) -> bool {
        if !symbol.contains('/') && self.top_level_names.contains(symbol) {
            return false;
        }
        match self.names.names.resolve_parts(symbol) {
            ("clojure.test", "testing") => self.forms(args),
            ("clojure.test", "is") => {
                if let Some(assertion) = args.first() {
                    self.assertion(assertion);
                }
            }
            ("clojure.test", "are") if self.template_scope.is_none() => {
                self.assertion_template(args)
            }
            _ => return false,
        }
        true
    }

    fn assertion(&mut self, assertion: &'a Form) {
        let Kind::List(items) = &assertion.without_metadata().kind else {
            return self.forms(std::slice::from_ref(assertion));
        };
        let Some((head, values)) = items.split_first() else {
            return;
        };
        let Some(name) = head.symbol().filter(|name| !self.template_parameter(name)) else {
            return;
        };
        match name {
            "thrown?" | "thrown-with-msg?"
                if let [class, expressions @ ..] = values
                    && class.symbol().is_some() =>
            {
                self.forms(expressions)
            }
            _ if (name.contains('/')
                || (!self.top_level_names.contains(name) && !self.is_local(name)))
                && matches!(
                    self.names.names.resolve_parts(name),
                    ("clojure.core", "=" | "nil?" | "some?" | "true?" | "false?")
                ) =>
            {
                self.forms(values)
            }
            // Other assertion forms can have user-defined expansion.
            _ => {}
        }
    }

    fn assertion_template(&mut self, args: &'a [Form]) {
        let [parameters, assertion, rows @ ..] = args else {
            return;
        };
        let Some(parameters) = parameters.vector() else {
            return;
        };
        if parameters.is_empty() || rows.is_empty() || !rows.len().is_multiple_of(parameters.len())
        {
            return;
        }
        let scope = self.locals.len();
        for parameter in parameters {
            let Some(name) = parameter
                .symbol()
                .filter(|name| !name.contains('/') || *name == "/")
            else {
                self.locals.truncate(scope);
                return;
            };
            self.locals.push((name, parameter.without_metadata().span));
        }
        self.template_scope = Some(scope);
        self.assertion(assertion);
        self.template_scope = None;
        self.locals.truncate(scope);
    }

    fn function(&mut self, args: &'a [Form], span: Span) {
        // Template substitution can change the names of lexical binders.
        if self.template_scope.is_some() {
            return;
        }
        let scope = self.locals.len();
        let clauses = if let Some((name, tail)) = args.split_first()
            && let Some(symbol) = name.symbol()
        {
            self.locals.push((symbol, name.without_metadata().span));
            tail
        } else {
            args
        };
        if let Ok(clauses) = lexical_function_clauses(clauses, span) {
            for clause in clauses.values() {
                self.clause(clause);
            }
        }
        self.locals.truncate(scope);
    }

    fn local_functions(&mut self, args: &'a [Form]) {
        if self.template_scope.is_some() {
            return;
        }
        let [specs, body @ ..] = args else {
            return;
        };
        let Some(specs) = specs.vector() else {
            return;
        };
        let mut functions = Vec::with_capacity(specs.len());
        for spec in specs {
            let Kind::List(items) = &spec.without_metadata().kind else {
                return;
            };
            let [name, tail @ ..] = items.as_slice() else {
                return;
            };
            let Some(symbol) = name
                .symbol()
                .filter(|name| !name.contains('/') || *name == "/")
            else {
                return;
            };
            let Ok(clauses) = lexical_function_clauses(tail, spec.span) else {
                return;
            };
            functions.push(((symbol, name.without_metadata().span), clauses));
        }
        let scope = self.locals.len();
        self.locals
            .extend(functions.iter().map(|(binding, _)| *binding));
        for (_, clauses) in &functions {
            for clause in clauses.values() {
                self.clause(clause);
            }
        }
        self.forms(body);
        self.locals.truncate(scope);
    }

    fn bindings(&mut self, args: &'a [Form]) {
        if self.template_scope.is_some() {
            return;
        }
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

    fn conditional_binding(&mut self, args: &'a [Form], has_else: bool) {
        let [bindings, body @ ..] = args else {
            return;
        };
        if self.template_scope.is_some() || (has_else && !(1..=2).contains(&body.len())) {
            return;
        }
        let Some([pattern, value]) = bindings.vector() else {
            return;
        };
        let (matched, fallback) = if has_else {
            body.split_at(1)
        } else {
            (body, &[][..])
        };
        self.forms(std::slice::from_ref(value));
        let scope = self.locals.len();
        binding_names(pattern, &mut self.locals);
        self.forms(matched);
        self.locals.truncate(scope);
        self.forms(fallback);
    }
}
