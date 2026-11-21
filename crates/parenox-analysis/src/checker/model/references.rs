//! Definite project-namespace call errors in bodies that lack a type check.
use std::collections::BTreeSet;
use std::ops::Range;

use crate::Diagnostic;
use crate::checker::bindings::binding_names;
use crate::checker::calls::arity_counts;
use crate::checker::function_forms::{
    Clause, function_clauses, letfn_bindings, letfn_definition, lexical_fn_star_clauses,
    lexical_function_clauses,
};
use crate::expand::Threading;
use crate::reader::{Form, Kind, Span};

use super::contracts::function_parts;
use super::declarations::{SourceDeclaration, collect_protocol_group_methods, symbols_match};
use super::{Declarations, Model, java_type_tag_at};

pub(crate) enum ProjectCall {
    Unknown,
    Missing,
    Function { expected_arity: Option<String> },
}

#[derive(Clone, Copy)]
pub(crate) enum ScopedReference<'a> {
    Local(Span),
    Referred(&'a str, &'a str),
    InNamespace(&'a str),
}

#[derive(Default)]
pub(crate) struct DocumentReferences<'a> {
    pub uses: Vec<(Span, ScopedReference<'a>)>,
    pub bindings: Vec<Span>,
    pub calls: Option<Vec<(Span, ScopedReference<'a>)>>,
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
        self.source_function_call(forms, name, actual, true)
    }

    pub(crate) fn source_function_call(
        &self,
        forms: &[Form],
        name: &str,
        actual: usize,
        public_only: bool,
    ) -> ProjectCall {
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
        if public_only && self.source_private(&declaration) {
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
            post_result_scope: None,
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
        let unchecked = model.unchecked_definition_offsets();
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
        resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Option<ScopedReference<'a>> {
        self.navigation_at(forms, offset, standard_test, false, None, resolve)?
            .symbol
    }

    pub(crate) fn validation_spec_at<'a>(
        &'a self,
        forms: &'a [Form],
        offset: usize,
        standard_test: bool,
        resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Option<&'a str> {
        self.navigation_at(forms, offset, standard_test, false, None, resolve)?
            .spec
    }

    pub(crate) fn evaluated_reference_at<'a>(
        &'a self,
        forms: &'a [Form],
        offset: usize,
        standard_test: bool,
        resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Option<(&'a str, ScopedReference<'a>)> {
        let target = self.navigation_at(forms, offset, standard_test, true, None, resolve)?;
        target.evaluated_symbol.zip(target.symbol)
    }

    pub(crate) fn lexical_binding_at(
        &self,
        forms: &[Form],
        offset: usize,
        name: &str,
        standard_test: bool,
        resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Option<bool> {
        self.navigation_at(forms, offset, standard_test, true, Some(name), resolve)?
            .lexical_binding
    }

    pub(crate) fn evaluated_java_class_at<'a>(
        &'a self,
        forms: &'a [Form],
        offset: usize,
        standard_test: bool,
        resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Option<&'a str> {
        self.navigation_at(forms, offset, standard_test, true, None, resolve)?
            .java_class
    }

    fn navigation_at<'a>(
        &'a self,
        forms: &'a [Form],
        offset: usize,
        standard_test: bool,
        include_top_level_expressions: bool,
        binding_name: Option<&str>,
        mut resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Option<NavigationTarget<'a>> {
        let index = forms.partition_point(|form| form.span.end.offset <= offset);
        let form = forms.get(index)?;
        let inside_test = self.test_forms.binary_search(&index).is_ok();
        let inside_method = self.method_forms.binary_search(&index).is_ok();
        let inside_function = self.functions.binary_search(&index).is_ok();
        if !(inside_test || inside_method || inside_function || include_top_level_expressions) {
            return None;
        }
        let top_level_names = self.top_level_names(forms);
        let mut visit = References {
            names: self,
            top_level_names: &top_level_names,
            locals: Vec::new(),
            post_result_scope: None,
            inside_test,
            standard_test,
            template_scope: None,
            resolve: &mut resolve,
            purpose: Purpose::Navigate {
                offset,
                binding_name,
                target: NavigationTarget::default(),
            },
        };
        if inside_test {
            if let Kind::List(items) = &form.without_metadata().kind {
                visit.forms(&items[2..]);
            }
        } else if inside_method {
            visit.method(form);
        } else if inside_function {
            visit.definition(form);
        } else if let Kind::List(items) = &form.without_metadata().kind
            && let [head, name, tail @ ..] = items.as_slice()
            && head.symbol() == Some("def")
        {
            if name.symbol().is_some() {
                visit.select_type_hint(name);
            }
            if let Some(initializer) = tail.last() {
                visit.forms(std::slice::from_ref(initializer));
            }
        } else {
            visit.forms(std::slice::from_ref(form));
        }
        match visit.purpose {
            Purpose::Navigate { target, .. } => Some(target),
            Purpose::Check(_) | Purpose::Collect(_) | Purpose::ProtocolMethods { .. } => {
                unreachable!()
            }
        }
    }

    pub(crate) fn collect_references<'a>(
        &'a self,
        forms: &'a [Form],
        standard_test: bool,
        collect_calls: bool,
        resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> DocumentReferences<'a> {
        let mut occurrences = DocumentReferences {
            calls: collect_calls.then(Vec::new),
            ..DocumentReferences::default()
        };
        self.collect_source_forms(
            forms,
            standard_test,
            resolve,
            Purpose::Collect(&mut occurrences),
            None,
        );
        occurrences
    }

    pub(crate) fn functions_in(&self, forms: &[Form], range: Range<usize>) -> &[usize] {
        let first = self
            .functions
            .partition_point(|index| forms[*index].span.end.offset <= range.start);
        let last = self
            .functions
            .partition_point(|index| forms[*index].span.start.offset < range.end);
        &self.functions[first..last]
    }

    pub(crate) fn collect_function_bindings<'a>(
        &'a self,
        forms: &'a [Form],
        functions: &[usize],
        resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Vec<Span> {
        let mut occurrences = DocumentReferences::default();
        self.collect_source_forms(
            forms,
            false,
            resolve,
            Purpose::Collect(&mut occurrences),
            Some(functions),
        );
        occurrences.bindings
    }

    pub(crate) fn reify_method_targets<'a>(
        &'a self,
        forms: &'a [Form],
        method: &str,
        standard_test: bool,
        resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> Vec<(&'a str, Span)> {
        let mut targets = Vec::new();
        self.collect_source_forms(
            forms,
            standard_test,
            resolve,
            Purpose::ProtocolMethods {
                method,
                targets: &mut targets,
            },
            None,
        );
        targets
    }

    fn collect_source_forms<'a>(
        &'a self,
        forms: &'a [Form],
        standard_test: bool,
        mut resolve: impl FnMut(&str, &str, usize) -> ProjectCall,
        purpose: Purpose<'a, '_>,
        selected_functions: Option<&[usize]>,
    ) {
        let names = self.top_level_names(forms);
        let mut visit = References {
            names: self,
            top_level_names: &names,
            locals: Vec::new(),
            post_result_scope: None,
            inside_test: false,
            standard_test,
            template_scope: None,
            resolve: &mut resolve,
            purpose,
        };
        for &index in selected_functions.unwrap_or(&self.functions) {
            visit.definition(&forms[index]);
        }
        if selected_functions.is_some() {
            return;
        }
        for (bindings, form) in self.scoped_function_forms(forms) {
            for [pattern, _] in bindings.as_chunks::<2>().0 {
                visit.bind_pattern(pattern);
            }
            visit.definition(form);
            visit.locals.clear();
        }
        for &index in self.literal_vars.iter().chain(&self.unchecked_vars) {
            if let Kind::List(items) = &forms[index].without_metadata().kind
                && let [head, _, tail @ ..] = items.as_slice()
            {
                // A defmulti's dispatch function is its only plain expression.
                if head
                    .symbol()
                    .is_some_and(|op| self.names.resolve_parts(op) == ("clojure.core", "defmulti"))
                {
                    if let Some(dispatch) = super::declarations::multimethod_parts(tail).1.first() {
                        visit.forms(std::slice::from_ref(dispatch));
                    }
                } else if let Some(value) = tail.last() {
                    visit.forms(std::slice::from_ref(value));
                }
            }
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
    }

    pub(crate) fn top_level_names<'a>(&'a self, forms: &'a [Form]) -> BTreeSet<&'a str> {
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

impl Model {
    pub(super) fn unchecked_definition_offsets(&self) -> BTreeSet<usize> {
        // A PNX9002 at the definition means no typed root checks this body.
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "PNX9002")
            .map(|diagnostic| diagnostic.span.start.offset)
            .collect()
    }
}

enum Purpose<'source, 'query> {
    Check(&'query mut Vec<Diagnostic>),
    Collect(&'query mut DocumentReferences<'source>),
    ProtocolMethods {
        method: &'query str,
        targets: &'query mut Vec<(&'source str, Span)>,
    },
    Navigate {
        offset: usize,
        binding_name: Option<&'query str>,
        target: NavigationTarget<'source>,
    },
}

#[derive(Default)]
struct NavigationTarget<'source> {
    symbol: Option<ScopedReference<'source>>,
    spec: Option<&'source str>,
    evaluated_symbol: Option<&'source str>,
    lexical_binding: Option<bool>,
    java_class: Option<&'source str>,
}

struct References<'source, 'query, F> {
    names: &'source Declarations,
    top_level_names: &'query BTreeSet<&'source str>,
    locals: Vec<(&'source str, Span)>,
    post_result_scope: Option<usize>,
    inside_test: bool,
    standard_test: bool,
    template_scope: Option<usize>,
    resolve: &'query mut F,
    purpose: Purpose<'source, 'query>,
}

impl<'a, F: FnMut(&str, &str, usize) -> ProjectCall> References<'a, '_, F> {
    fn definition(&mut self, form: &'a Form) {
        if let Kind::List(items) = &form.without_metadata().kind
            && let [_, name, tail @ ..] = items.as_slice()
        {
            let (arms, mut attributes) = function_parts(tail);
            if let Ok(clauses) = lexical_function_clauses(arms, form.span) {
                if matches!(self.purpose, Purpose::Navigate { .. })
                    && name.symbol().is_some()
                    && !attributes.any(|[key, _]| key.keyword() == Some(":tag"))
                {
                    self.select_type_hint(name);
                }
                for clause in clauses.values() {
                    self.clause(clause);
                }
            }
        }
    }

    fn clause(&mut self, clause: &Clause<'a>) {
        let scope = self.locals.len();
        self.select_type_hint(clause.parameter_vector);
        for parameter in clause.parameters() {
            self.bind_pattern(parameter);
        }
        // These forms can be elided by the compiler. Typed checking reports
        // their findings with the activation condition.
        if !matches!(self.purpose, Purpose::Check(_)) {
            self.forms(clause.conditions.pre);
        }
        self.forms(clause.body);
        if !matches!(self.purpose, Purpose::Check(_))
            && let Some(post) = clause.conditions.post
        {
            let previous = self.post_result_scope.replace(self.locals.len());
            self.forms(post);
            self.post_result_scope = previous;
        }
        self.locals.truncate(scope);
    }

    fn bind_pattern(&mut self, pattern: &'a Form) {
        let first = self.locals.len();
        binding_names(pattern, &mut self.locals);
        if matches!(self.purpose, Purpose::Navigate { .. })
            && self.locals.get(first).map(|(_, span)| *span)
                == Some(pattern.without_metadata().span)
        {
            self.select_type_hint(pattern);
        }
        if let Purpose::Collect(occurrences) = &mut self.purpose
            && occurrences.calls.is_none()
        {
            occurrences
                .bindings
                .extend(self.locals[first..].iter().map(|(_, span)| *span));
        }
    }

    fn bind(&mut self, name: &'a str, span: Span) {
        self.locals.push((name, span));
        if let Purpose::Collect(occurrences) = &mut self.purpose
            && occurrences.calls.is_none()
        {
            occurrences.bindings.push(span);
        }
    }

    fn method(&mut self, form: &'a Form) {
        if let Kind::List(items) = &form.without_metadata().kind
            && let [_, declaration @ ..] = items.as_slice()
            && let [target, dispatch, tail @ ..] = declaration
        {
            // The dispatch value is evaluated before the method's bindings exist.
            self.forms(std::slice::from_ref(target));
            self.forms(std::slice::from_ref(dispatch));
            self.function(tail, form.span, false);
        }
    }

    fn forms(&mut self, forms: &'a [Form]) {
        for form in forms {
            if let Purpose::Navigate {
                offset,
                binding_name,
                target,
            } = &mut self.purpose
            {
                if !(form.span.start.offset..form.span.end.offset).contains(offset) {
                    continue;
                }
                if form.without_metadata().span.start.offset == *offset
                    && let Some(name) = binding_name
                {
                    target.lexical_binding = Some(
                        (*name == "%" && self.post_result_scope.is_some())
                            || self.locals.iter().any(|(local, _)| local == name),
                    );
                    return;
                }
            }
            match &form.without_metadata().kind {
                Kind::List(items) => {
                    if items.first().and_then(Form::symbol) != Some("quote") {
                        self.select_type_hint(form);
                    }
                    self.call(items);
                }
                Kind::Vector(items)
                | Kind::Map(items)
                | Kind::Set(items)
                | Kind::AutoNamespacedMap { entries: items, .. } => self.forms(items),
                Kind::AnonymousFunction(function) => self.call(&function.body),
                Kind::Symbol(symbol) => {
                    self.select_type_hint(form);
                    self.select_reference(symbol, form.without_metadata().span);
                }
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
        if matches!(self.purpose, Purpose::Navigate { .. }) {
            if let Some(receiver) = args.first()
                && (symbol == "."
                    || symbol
                        .strip_prefix('.')
                        .is_some_and(|member| !member.is_empty() && !member.contains(['.', '/'])))
            {
                self.select_type_hint(receiver);
            }
            if symbol == "new" {
                if let Some((class, arguments)) = args.split_first() {
                    if let Some(symbol) = class.symbol() {
                        self.select_class(symbol, class.without_metadata().span);
                    }
                    self.forms(arguments);
                }
                return;
            }
            if let Some(class) = symbol.strip_suffix('.')
                && !class.is_empty()
                && !class.starts_with('.')
                && !class.contains('/')
            {
                self.select_class(class, span);
                self.forms(args);
                return;
            }
        }
        match symbol {
            "quote" => return,
            "var" => {
                if let [form] = args
                    && let Some(var) = form.symbol()
                {
                    self.record_reference(var, form.without_metadata().span, false);
                }
                return;
            }
            "if" | "do" | "recur" | "throw" => return self.forms(args),
            "try" => return self.try_body(args),
            "let*" | "loop*" => return self.bindings(args),
            "fn*" => return self.function(args, head.span, true),
            _ => {}
        }
        self.select_reference(symbol, span);
        if matches!(
            self.purpose,
            Purpose::Navigate {
                target: NavigationTarget {
                    symbol: Some(_),
                    ..
                },
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
        let (namespace, local) = self.names.names.resolve_parts(symbol);
        if (namespace, local) == ("clojure.spec.alpha", "valid?")
            && (symbol.contains('/')
                || (!self.top_level_names.contains(symbol)
                    && !self.names.forward_vars.contains_key(symbol)))
            && let [spec, _, ..] = args
            && args.len() <= 3
            && let Purpose::Navigate { offset, target, .. } = &mut self.purpose
        {
            if (spec.span.start.offset..spec.span.end.offset).contains(offset) {
                target.spec = spec.keyword();
            }
            return self.forms(args);
        }
        if self.trusted_core(symbol, namespace) {
            if let Some(thread) = Threading::from_core_name(local) {
                return self.thread(args, thread);
            }
            match local {
                "let" | "loop" => return self.bindings(args),
                "if-let" | "if-some" => return self.conditional_binding(args, true),
                "when-let" | "when-some" | "when-first" => {
                    return self.conditional_binding(args, false);
                }
                "fn" => return self.function(args, head.span, false),
                "letfn" => return self.local_functions(args),
                "case" => return self.case_body(args),
                "as->" => return self.named_thread(args),
                "instance?" if matches!(self.purpose, Purpose::Navigate { .. }) => {
                    return self.forms(args);
                }
                "reify"
                    if !self.names.forward_vars.contains_key(symbol)
                        && let Purpose::ProtocolMethods { method, targets } = &mut self.purpose =>
                {
                    return collect_protocol_group_methods(args, None, method, targets);
                }
                _ if plain_core_arguments(local) => return self.forms(args),
                _ => {}
            }
        }
        self.source_call(symbol, (namespace, local), span, args, 0);
    }

    // Returns whether the source target is known to take an ordinary argument list.
    fn source_call(
        &mut self,
        symbol: &'a str,
        (mut namespace, mut local): (&'a str, &'a str),
        span: Span,
        args: &'a [Form],
        implicit: usize,
    ) -> bool {
        let qualified = self.names.qualified_reference(symbol);
        if namespace == self.names.namespace()
            || (qualified.is_none()
                && (self.top_level_names.contains(symbol)
                    || !self.names.names.refers.contains_key(symbol)))
        {
            if !matches!(
                self.purpose,
                Purpose::Collect(_) | Purpose::ProtocolMethods { .. }
            ) {
                return false;
            }
            namespace = self.names.namespace();
            local = qualified.map_or(symbol, |(_, name)| name);
        }
        let actual = args.len() + implicit;
        match (self.resolve)(namespace, local, actual) {
            // Missing explicit imports already have a finding on the declaration.
            ProjectCall::Missing if qualified.is_some() => {
                if let Purpose::Check(diagnostics) = &mut self.purpose {
                    diagnostics.push(Diagnostic::new(
                        "PNX2304",
                        span,
                        format!("Unresolved or private project var {symbol}"),
                    ));
                }
                false
            }
            ProjectCall::Function { expected_arity } => {
                if let Purpose::Collect(occurrences) = &mut self.purpose
                    && let Some(calls) = &mut occurrences.calls
                {
                    let target = if qualified.is_none() && namespace == self.names.namespace() {
                        ScopedReference::InNamespace(local)
                    } else {
                        ScopedReference::Referred(namespace, local)
                    };
                    calls.push((span, target));
                }
                if let Some(expected) = expected_arity
                    && let Purpose::Check(diagnostics) = &mut self.purpose
                {
                    diagnostics.push(Diagnostic::new(
                        "PNX2554",
                        span,
                        format!("{symbol} expects {expected} arguments, got {actual}"),
                    ));
                }
                self.forms(args);
                true
            }
            _ => false,
        }
    }

    fn select_reference(&mut self, symbol: &'a str, span: Span) {
        self.record_reference(symbol, span, true);
    }

    fn select_class(&mut self, symbol: &'a str, span: Span) {
        if let Purpose::Navigate { offset, target, .. } = &mut self.purpose
            && (span.start.offset..span.end.offset).contains(offset)
        {
            target.java_class = Some(symbol);
        }
    }

    fn select_type_hint(&mut self, form: &'a Form) {
        if let Purpose::Navigate { offset, target, .. } = &mut self.purpose
            && let Some(tag) = java_type_tag_at(form, *offset)
        {
            target.java_class = tag.symbol();
        }
    }

    fn record_reference(&mut self, symbol: &'a str, span: Span, allow_local: bool) {
        match self.purpose {
            Purpose::Check(_) | Purpose::ProtocolMethods { .. } => return,
            Purpose::Collect(ref occurrences) if occurrences.calls.is_some() => return,
            Purpose::Navigate { offset, .. }
                if !(span.start.offset..span.end.offset).contains(&offset) =>
            {
                return;
            }
            _ => {}
        }
        let hit = if let Some((namespace, name)) = self.names.qualified_reference(symbol) {
            ScopedReference::Referred(namespace, name)
        } else if allow_local
            && let Some((_, binding)) = self.locals[self.local_scope(symbol)..]
                .iter()
                .rev()
                .find(|(name, _)| *name == symbol)
        {
            ScopedReference::Local(*binding)
        } else if allow_local && symbol == "%" && self.post_result_scope.is_some() {
            // The macro-introduced result has no source definition or rename target.
            return;
        } else if !self.top_level_names.contains(symbol)
            && let Some((namespace, local)) = self
                .names
                .names
                .refers
                .get(symbol)
                .and_then(|target| target.split_once('/'))
        {
            ScopedReference::Referred(namespace, local)
        } else {
            ScopedReference::InNamespace(symbol)
        };
        match &mut self.purpose {
            Purpose::Navigate { target, .. } => {
                target.symbol = Some(hit);
                if allow_local {
                    target.evaluated_symbol = Some(symbol);
                    if matches!(hit, ScopedReference::InNamespace(_))
                        && !self.top_level_names.contains(symbol)
                        && !self.names.forward_vars.contains_key(symbol)
                    {
                        target.java_class = Some(symbol);
                    }
                }
            }
            Purpose::Collect(found) => found.uses.push((span, hit)),
            Purpose::Check(_) | Purpose::ProtocolMethods { .. } => unreachable!(),
        }
    }

    fn is_local(&self, symbol: &str) -> bool {
        (symbol == "%" && self.post_result_scope.is_some())
            || self.locals.iter().rev().any(|(name, _)| *name == symbol)
    }

    fn local_scope(&self, symbol: &str) -> usize {
        self.post_result_scope
            .filter(|_| symbol == "%")
            .unwrap_or(0)
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
            self.bind(name, parameter.without_metadata().span);
        }
        self.template_scope = Some(scope);
        self.assertion(assertion);
        self.template_scope = None;
        self.locals.truncate(scope);
    }

    fn function(&mut self, args: &'a [Form], span: Span, special_form: bool) {
        // Template substitution can change the names of lexical binders.
        if self.template_scope.is_some() {
            return;
        }
        let scope = self.locals.len();
        let clauses = if let Some((name, tail)) = args.split_first()
            && let Some(symbol) = name.symbol()
        {
            self.bind(symbol, name.without_metadata().span);
            tail
        } else {
            args
        };
        let clauses = if special_form {
            lexical_fn_star_clauses(clauses, span)
        } else {
            lexical_function_clauses(clauses, span)
        };
        if let Ok(clauses) = clauses {
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
        let Some(specs) = letfn_bindings(specs) else {
            return;
        };
        let mut functions = Vec::with_capacity(specs.len());
        for spec in specs {
            let Some([name, tail @ ..]) = letfn_definition(spec) else {
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
        for &((name, span), _) in &functions {
            self.bind(name, span);
        }
        for (binding, clauses) in &functions {
            // A duplicate name can shadow the group binding. The named fn
            // still sees its own definition within its clauses.
            self.locals.push(*binding);
            for clause in clauses.values() {
                self.clause(clause);
            }
            self.locals.pop();
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
                self.bind_pattern(pattern);
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
        self.bind_pattern(pattern);
        self.forms(matched);
        self.locals.truncate(scope);
        self.forms(fallback);
    }

    fn try_body(&mut self, forms: &'a [Form]) {
        if self.template_scope.is_some() {
            return;
        }
        let mut in_clauses = false;
        for form in forms {
            let clause = match &form.without_metadata().kind {
                Kind::List(items) => items.split_first(),
                _ => None,
            };
            match clause.and_then(|(head, args)| Some((head.symbol()?, args))) {
                Some(("catch", [class, binding, body @ ..])) if class.symbol().is_some() => {
                    in_clauses = true;
                    let Some(name) = binding
                        .symbol()
                        .filter(|name| !name.contains('/') || *name == "/")
                    else {
                        return;
                    };
                    let scope = self.locals.len();
                    self.bind(name, binding.without_metadata().span);
                    self.forms(body);
                    self.locals.truncate(scope);
                }
                Some(("finally", body)) => {
                    self.forms(body);
                    return;
                }
                Some(("catch", _)) => return,
                _ if !in_clauses => self.forms(std::slice::from_ref(form)),
                _ => return,
            }
        }
    }

    fn case_body(&mut self, forms: &'a [Form]) {
        let Some((input, clauses)) = forms.split_first() else {
            return;
        };
        self.forms(std::slice::from_ref(input));
        let (pairs, fallback) = clauses.as_chunks::<2>();
        for [constants, result] in pairs {
            // A test position is data, never a call. An empty group has no branch.
            if !matches!(&constants.without_metadata().kind, Kind::List(items) if items.is_empty())
            {
                self.forms(std::slice::from_ref(result));
            }
        }
        self.forms(fallback);
    }

    fn thread(&mut self, forms: &'a [Form], thread: Threading) {
        let Some((input, steps)) = forms.split_first() else {
            return;
        };
        if !thread.is_conditional() && !thread.guards_nil() {
            // These steps nest as arguments. An unknown macro may discard the
            // whole incoming expression, so don't inspect that prefix.
            if steps.iter().rev().all(|step| self.thread_step(step)) {
                self.forms(std::slice::from_ref(input));
            }
            return;
        }
        self.forms(std::slice::from_ref(input));
        if thread.is_conditional() {
            let (pairs, []) = steps.as_chunks::<2>() else {
                return;
            };
            for [test, step] in pairs {
                self.forms(std::slice::from_ref(test));
                self.thread_step(step);
            }
        } else {
            for step in steps {
                self.thread_step(step);
            }
        }
    }

    fn thread_step(&mut self, step: &'a Form) -> bool {
        let step = step.without_metadata();
        let (head, args) = match &step.kind {
            Kind::List(items) => match items.split_first() {
                Some(parts) => parts,
                None => return false,
            },
            _ => (step, &[][..]),
        };
        let Some(symbol) = head.symbol() else {
            if head.keyword().is_some() {
                self.forms(args);
                return true;
            }
            return false;
        };
        let span = head.without_metadata().span;
        self.select_reference(symbol, span);
        if self.template_parameter(symbol) {
            return false;
        }
        let (namespace, local) = self.names.names.resolve_parts(symbol);
        if self.is_local(symbol)
            || (self.trusted_core(symbol, namespace) && plain_core_arguments(local))
        {
            self.forms(args);
            true
        } else {
            // Only resolved function steps have a normal argument list.
            self.source_call(symbol, (namespace, local), span, args, 1)
        }
    }

    fn named_thread(&mut self, forms: &'a [Form]) {
        let [input, binding, steps @ ..] = forms else {
            return;
        };
        if self.template_scope.is_some() {
            return;
        }
        let Some(name) = binding
            .symbol()
            .filter(|name| !name.contains('/') || *name == "/")
        else {
            return;
        };
        self.forms(std::slice::from_ref(input));
        let scope = self.locals.len();
        self.bind(name, binding.without_metadata().span);
        self.forms(steps);
        self.locals.truncate(scope);
    }
}

fn plain_core_arguments(name: &str) -> bool {
    matches!(
        name,
        "when"
            | "when-not"
            | "if-not"
            | "and"
            | "or"
            | "cond"
            | "="
            | "not="
            | "<"
            | "<="
            | ">"
            | ">="
            | "+"
            | "-"
            | "*"
            | "inc"
            | "dec"
            | "not"
            | "nil?"
            | "some?"
            | "name"
            | "map?"
            | "symbol?"
            | "simple-symbol?"
            | "qualified-symbol?"
            | "str"
            | "vector"
            | "list"
            | "hash-map"
            | "set"
            | "get"
            | "get-in"
            | "contains?"
            | "assoc"
            | "conj"
            | "map"
            | "mapv"
            | "filter"
            | "reduce"
            | "apply"
            | "subs"
            | "first"
            | "next"
            | "rest"
            | "count"
    )
}
