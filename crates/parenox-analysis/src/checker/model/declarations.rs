//! Source declarations and the namespace context used to read them.
mod method_bodies;
mod outline;
mod protocol_implementations;
mod reader_names;
mod var_documentation;

pub(super) use protocol_implementations::collect_protocol_group_methods;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::Globals;
use super::contracts::function_parts;
use super::references::ProjectCall;
use super::require::{ExplicitReferral, RuntimeLoads};
use crate::Diagnostic;
use crate::checker::bindings::binding_names;
use crate::checker::function_forms::{Clause, function_clauses, lexical_function_clauses};
use crate::reader::{Form, Kind, OmittedDispatch, Span, UncheckedDispatches};
use crate::types::Arity;

pub(super) enum SourceDeclaration<'a> {
    Function(&'a [Form]),
    Value(&'a [Form]),
    TypeFactory { name: &'a Form, arity: Arity },
    Protocol(ProtocolVar<'a>),
}

impl<'a> SourceDeclaration<'a> {
    fn name(&self) -> &'a Form {
        match self {
            Self::Function(items) | Self::Value(items) => &items[1],
            Self::TypeFactory { name, .. } => name,
            Self::Protocol(var) => var.name,
        }
    }
}

pub(crate) struct FunctionParameters<'a> {
    pub documentation: Option<&'a str>,
    pub clauses: BTreeMap<Arity, Vec<Span>>,
}

pub(super) struct ProtocolVar<'a> {
    pub name: &'a Form,
    pub method: bool,
    owner: &'a Form,
    tail: &'a [Form],
}

impl ProtocolVar<'_> {
    pub fn private(&self) -> bool {
        self.name.metadata(":private").is_some_and(|value| {
            !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
        })
    }
}

struct TypeDeclaration {
    form: usize,
    has_map_factory: bool,
}

struct ScopedFunction {
    enclosing: usize,
    definition: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Names {
    pub(super) namespace: String,
    pub(super) refers: BTreeMap<String, String>,
    pub(super) aliases: BTreeMap<String, String>,
    pub(super) imports: BTreeMap<String, String>,
    pub(super) required_libraries: BTreeSet<String>,
    pub(super) aliased_libraries: BTreeSet<String>,
    pub(super) proven_alias_libraries: BTreeSet<String>,
    pub(super) core_only: Option<BTreeSet<String>>,
    pub(super) core_hidden: BTreeSet<String>,
    pub(super) source_macros: BTreeMap<String, Vec<SourceMacro>>,
    pub(super) spec_core_shadows: BTreeSet<String>,
    pub(super) spec_core_scope_known: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourceMacro {
    defined: usize,
    pub(super) replaced: Option<usize>,
    pub(super) private: bool,
}

impl SourceMacro {
    pub(super) fn active_at(&self, offset: usize) -> bool {
        self.defined <= offset && self.replaced.is_none_or(|end| offset < end)
    }
}

impl Names {
    pub(super) fn local_source_macros(&self, name: &str) -> Option<(&str, &[SourceMacro])> {
        let local = if name.contains('/') && name != "/" {
            let (namespace, local) = self.resolve_parts(name);
            (namespace == self.namespace).then_some(local)?
        } else {
            name
        };
        self.source_macros
            .get_key_value(local)
            .map(|(name, declarations)| (name.as_str(), declarations.as_slice()))
    }

    pub(super) fn resolve(&self, name: &str) -> String {
        let (namespace, local) = self.resolve_parts(name);
        format!("{namespace}/{local}")
    }

    pub(super) fn resolve_parts<'a>(&'a self, name: &'a str) -> (&'a str, &'a str) {
        if let Some((prefix, local)) = name
            .split_once('/')
            .filter(|(prefix, _)| !prefix.is_empty())
        {
            (
                self.aliases.get(prefix).map_or(prefix, String::as_str),
                local,
            )
        } else if let Some(referred) = self.refers.get(name) {
            referred.split_once('/').unwrap()
        } else if self.core_visible(name) {
            ("clojure.core", name)
        } else {
            (&self.namespace, name)
        }
    }

    pub(super) fn runtime_alias_gap(&self, symbol: &str) -> Option<&str> {
        if self.aliased_libraries.is_empty() {
            return None;
        }
        let (namespace, _) = self.resolve_parts(symbol);
        self.runtime_alias_gap_in(namespace)
    }

    fn runtime_alias_gap_in(&self, namespace: &str) -> Option<&str> {
        let namespace = self.aliased_libraries.get(namespace)?.as_str();
        (!self.directly_loads(namespace) && !self.proven_alias_libraries.contains(namespace))
            .then_some(namespace)
    }

    pub(super) fn directly_loads(&self, namespace: &str) -> bool {
        namespace == self.namespace
            || namespace == "clojure.core"
            || self.required_libraries.contains(namespace)
    }

    pub(super) fn loads_library(&self, namespace: &str) -> bool {
        self.required_libraries.contains(namespace)
            || self.proven_alias_libraries.contains(namespace)
    }

    pub(super) fn core_visible(&self, name: &str) -> bool {
        self.core_only
            .as_ref()
            .is_none_or(|only| only.contains(name))
            && !self.core_hidden.contains(name)
    }

    pub(super) fn key(&self, key: &str) -> String {
        if let Some(tail) = key.strip_prefix("::") {
            if let Some((alias, name)) = tail.split_once('/') {
                format!(
                    ":{}/{name}",
                    self.aliases.get(alias).map_or(alias, String::as_str)
                )
            } else {
                format!(":{}/{tail}", self.namespace)
            }
        } else {
            key.to_string()
        }
    }

    pub(super) fn validation_key(&self, keyword: &str) -> Option<String> {
        if let Some((alias, _)) = keyword
            .strip_prefix("::")
            .and_then(|key| key.split_once('/'))
            && !self.aliases.contains_key(alias)
        {
            return None;
        }
        let key = self.key(keyword);
        let (namespace, name) = key.strip_prefix(':')?.split_once('/')?;
        (!namespace.is_empty() && !name.is_empty()).then_some(key)
    }
}

pub(super) struct SpecDeclaration {
    pub name: Span,
    pub body: Form,
}

pub(super) struct DeclaredVar {
    pub span: Span,
    pub private: bool,
}

#[derive(Default)]
pub(crate) struct Declarations {
    pub(super) names: Names,
    namespace_end: Option<usize>,
    pub(super) namespace_read_end: Option<usize>,
    pub dependencies: Vec<String>,
    pub(super) all_refers: BTreeSet<String>,
    pub(super) referrals: Vec<ExplicitReferral>,
    pub(super) namespace_usages: Vec<(String, Span)>,
    pub(super) namespace_alias_sites: BTreeMap<String, Span>,
    pub(super) java_import_sites: Vec<(Span, String)>,
    pub(crate) has_top_level_require: bool,
    has_top_level_in_ns: bool,
    pub(super) specs: BTreeMap<String, SpecDeclaration>,
    pub(super) duplicate_specs: BTreeSet<String>,
    pub(super) reflection_flags: Vec<Span>,
    pub(super) fdefs: Vec<usize>,
    pub(super) functions: Vec<usize>,
    pub(super) test_forms: Vec<usize>,
    pub(crate) refers_to_assert_expr: bool,
    pub(super) method_forms: Vec<usize>,
    pub(super) static_method_scope: bool,
    pub(super) literal_vars: Vec<usize>,
    pub(super) unchecked_vars: Vec<usize>,
    pub(super) discarded_comments: Vec<usize>,
    pub(super) unchecked_dispatches: Option<Arc<UncheckedDispatches>>,
    scoped_functions: Vec<ScopedFunction>,
    macros: Vec<usize>,
    unmodeled_calls: Vec<usize>,
    pub(super) forward_vars: BTreeMap<String, DeclaredVar>,
    types: Vec<TypeDeclaration>,
    protocols: Vec<usize>,
    protocol_extensions: Vec<usize>,
    pub(super) opaque_var_surface: bool,
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl Declarations {
    pub fn namespace(&self) -> &str {
        &self.names.namespace
    }

    pub(crate) fn runtime_alias_gap_in(&self, namespace: &str) -> Option<&str> {
        self.names.runtime_alias_gap_in(namespace)
    }

    pub(super) fn runtime_alias_gap(&self, symbol: &str) -> Option<&str> {
        self.names.runtime_alias_gap(symbol)
    }

    pub(crate) fn has_runtime_aliases(&self) -> bool {
        !self.names.aliased_libraries.is_empty()
    }

    pub(crate) fn proven_alias_libraries(&self) -> &BTreeSet<String> {
        &self.names.proven_alias_libraries
    }

    fn static_namespace_end(&self, forms: &[Form]) -> Option<usize> {
        let end = self.namespace_end?;
        let first = forms.first()?;
        (first.span.end.offset == end && static_namespace_prelude(first))
            .then_some(self.namespace_read_end.unwrap_or(end))
    }

    pub(super) fn has_static_requirements(&self, forms: &[Form]) -> bool {
        self.static_namespace_end(forms).is_some_and(|end| {
            self.diagnostics
                .iter()
                .all(|finding| finding.span.start.offset >= end)
        })
    }

    pub(super) fn initializer_scope_end(&self, forms: &[Form]) -> usize {
        let Some(namespace_end) = self.static_namespace_end(forms) else {
            return 0;
        };
        self.diagnostics
            .iter()
            .filter(|finding| {
                finding.severity == "error" || finding.span.start.offset >= namespace_end
            })
            .map(|finding| finding.span.start.offset)
            .min()
            .unwrap_or(usize::MAX)
    }

    pub(crate) fn runtime_alias_evidence(
        &self,
        forms: &[Form],
        loads: &RuntimeLoads,
    ) -> BTreeSet<String> {
        if self.has_runtime_aliases() && self.has_static_requirements(forms) {
            loads.proven_aliases(&self.names)
        } else {
            BTreeSet::new()
        }
    }

    pub(crate) fn with_runtime_loads(&self, forms: &[Form], loads: &RuntimeLoads) -> Option<Self> {
        let proven = self.runtime_alias_evidence(forms, loads);
        if proven == self.names.proven_alias_libraries {
            return None;
        }
        let mut declarations =
            Self::read_with_imports(forms, None, proven, self.unchecked_dispatches.clone());
        declarations.namespace_read_end = self.namespace_read_end;
        declarations.refers_to_assert_expr = self.refers_to_assert_expr;
        Some(declarations)
    }

    pub(crate) fn has_namespace_declaration(&self) -> bool {
        self.namespace_end.is_some()
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub(crate) fn reflection_flag_gaps(&self) -> impl Iterator<Item = Diagnostic> + '_ {
        self.reflection_flags.iter().copied().map(compiler_flag_gap)
    }

    pub(crate) fn spec_site(&self, key: &str) -> Option<Result<Span, ()>> {
        self.specs.get(key).map(|spec| {
            if self.duplicate_specs.contains(key) {
                Err(())
            } else {
                Ok(spec.name)
            }
        })
    }

    pub(crate) fn spec_names(&self) -> impl Iterator<Item = &str> {
        self.specs.keys().map(String::as_str)
    }

    pub(crate) fn qualified_reference<'a>(&'a self, symbol: &'a str) -> Option<(&'a str, &'a str)> {
        let (prefix, local) = symbol.split_once('/')?;
        if prefix.is_empty() || local.is_empty() || local.contains('/') {
            return None;
        }
        Some(self.names.resolve_parts(symbol))
    }

    pub(crate) fn qualifier<'a>(&'a self, name: &'a str) -> &'a str {
        self.names.aliases.get(name).map_or(name, String::as_str)
    }

    pub(crate) fn test_declarations<'a>(
        &'a self,
        forms: &'a [Form],
    ) -> impl Iterator<Item = (&'a Form, &'a Form, &'a [Form])> + 'a {
        self.test_forms.iter().filter_map(|&index| {
            let form = &forms[index];
            let Kind::List(items) = &form.without_metadata().kind else {
                return None;
            };
            let [operator, name, body @ ..] = items.as_slice() else {
                return None;
            };
            let operator_name = operator.symbol()?;
            if self.names.resolve_parts(operator_name) != ("clojure.test", "deftest")
                || (!operator_name.contains('/')
                    && self
                        .source_var_before(forms, operator_name, operator.span.start.offset)
                        .is_some())
            {
                return None;
            }
            name.symbol()?;
            Some((form, name, body))
        })
    }

    pub(crate) fn top_level_call<'a>(&'a self, symbol: &'a str) -> (&'a str, &'a str) {
        self.names
            .refers
            .get(symbol)
            .and_then(|target| target.split_once('/'))
            .unwrap_or((self.namespace(), symbol))
    }

    pub(crate) fn is_source_multimethod(&self, forms: &[Form], name: &str, span: Span) -> bool {
        let mut declarations = self.source_declarations(forms, name);
        let Some(SourceDeclaration::Value([head, declared, ..])) = declarations.next() else {
            return false;
        };
        declarations.next().is_none()
            && declared.without_metadata().span == span
            && self.is_unshadowed_core_operator(forms, head, "defmulti")
    }

    fn is_unshadowed_core_operator(&self, forms: &[Form], head: &Form, name: &str) -> bool {
        head.symbol().is_some_and(|operator| {
            self.names.resolve_parts(operator) == ("clojure.core", name)
                && (operator.contains('/')
                    || (self.all_refers.is_empty()
                        && !self.forward_vars.contains_key(operator)
                        && self.source_declarations(forms, operator).next().is_none()))
        })
    }

    pub(super) fn core_operator_shadow_start(
        &self,
        forms: &[Form],
        operator: &str,
        name: &str,
    ) -> Option<usize> {
        if self.names.resolve_parts(operator) != ("clojure.core", name)
            || self.runtime_alias_gap(operator).is_some()
        {
            return None;
        }
        if operator.contains('/') {
            return Some(usize::MAX);
        }
        if !self.all_refers.is_empty() || self.forward_vars.contains_key(operator) {
            return None;
        }
        Some(
            self.source_declarations(forms, operator)
                .map(|definition| definition.name().span.start.offset)
                .min()
                .unwrap_or(usize::MAX),
        )
    }

    fn multimethod_clauses<'a>(
        &self,
        forms: &[Form],
        tail: &'a [Form],
    ) -> Option<BTreeMap<Arity, Clause<'a>>> {
        let dispatcher = multimethod_parts(tail).1.first()?;
        let Kind::List(items) = &dispatcher.without_metadata().kind else {
            return None;
        };
        let [operator, body @ ..] = items.as_slice() else {
            return None;
        };
        if !self.is_unshadowed_core_operator(forms, operator, "fn") {
            return None;
        }
        let parameters = match body {
            [head, remainder @ ..] if head.symbol().is_some() => remainder,
            _ => body,
        };
        lexical_function_clauses(parameters, dispatcher.span).ok()
    }

    pub(crate) fn method_targets<'a>(
        &'a self,
        forms: &'a [Form],
    ) -> impl Iterator<Item = (&'a str, Span)> + 'a {
        self.method_forms.iter().filter_map(|&index| {
            let form = forms[index].without_metadata();
            let Kind::List(items) = &form.kind else {
                return None;
            };
            let [_, target, _, arities @ ..] = items.as_slice() else {
                return None;
            };
            lexical_function_clauses(arities, form.span).ok()?;
            let target = target.without_metadata();
            Some((target.symbol()?, target.span))
        })
    }

    pub(crate) fn local_var_registrations(&self, forms: &[Form], name: &str) -> Vec<Span> {
        let matching_fdefs = self.fdefs.iter().filter_map(|&index| {
            let Kind::List(items) = &forms[index].without_metadata().kind else {
                return None;
            };
            let symbol = items.get(1)?;
            let raw = symbol.symbol()?;
            let target = self.qualified_reference(raw).or_else(|| {
                self.names
                    .refers
                    .get(raw)
                    .and_then(|referred| referred.split_once('/'))
            });
            (target.unwrap_or((self.namespace(), raw)) == (self.namespace(), name))
                .then(|| symbol.without_metadata().span)
        });
        matching_fdefs
            .chain(
                self.forward_vars
                    .get(name)
                    .map(|declaration| declaration.span),
            )
            .collect()
    }

    pub(crate) fn function_parameters<'a>(
        &'a self,
        forms: &'a [Form],
        name: &'a str,
    ) -> Option<FunctionParameters<'a>> {
        let mut declarations = self.source_declarations(forms, name);
        let declaration = declarations.next()?;
        if declarations.next().is_some() {
            return None;
        }
        let (documentation, clauses) = match declaration {
            SourceDeclaration::Function([head, declared, tail @ ..]) => (
                var_documentation::function_docstring(declared, tail),
                lexical_function_clauses(function_parts(tail).0, head.span).ok()?,
            ),
            SourceDeclaration::Value([head, declared, tail @ ..])
                if self.is_unshadowed_core_operator(forms, head, "defmulti") =>
            {
                (
                    var_documentation::multimethod_docstring(declared, tail),
                    self.multimethod_clauses(forms, tail)?,
                )
            }
            _ => return None,
        };
        Some(FunctionParameters {
            documentation,
            clauses: clauses
                .into_iter()
                .map(|(arity, clause)| (arity, clause.parameters().map(|form| form.span).collect()))
                .collect(),
        })
    }

    pub(crate) fn core_completion_visible(
        &self,
        forms: &[Form],
        name: &str,
        offset: usize,
    ) -> bool {
        self.names.core_visible(name)
            && !self.names.refers.contains_key(name)
            && self.source_var_before(forms, name, offset).is_none()
    }

    pub(crate) fn source_var(
        &self,
        forms: &[Form],
        local: &str,
        public_only: bool,
    ) -> Option<Span> {
        let mut matches = self.source_declarations(forms, local);
        let Some(declaration) = matches.next() else {
            return self
                .forward_vars
                .get(local)
                .filter(|var| !public_only || !var.private)
                .map(|var| var.span);
        };
        if matches.next().is_some() || (public_only && self.source_private(&declaration)) {
            return None;
        }
        Some(declaration.name().without_metadata().span)
    }

    pub(crate) fn source_var_before(
        &self,
        forms: &[Form],
        local: &str,
        offset: usize,
    ) -> Option<Span> {
        let target = self.source_var(forms, local, false)?;
        (target.start.offset < offset
            || self
                .forward_vars
                .get(local)
                .is_some_and(|var| var.span.start.offset < offset))
        .then_some(target)
    }

    pub(crate) fn missing_public_var(&self, forms: &[Form], local: &str) -> bool {
        let mut definitions = self.source_declarations(forms, local).peekable();
        if definitions.peek().is_some() {
            definitions.all(|var| self.source_private(&var))
        } else {
            self.forward_vars.get(local).is_none_or(|var| var.private)
        }
    }

    pub(super) fn source_declarations<'a>(
        &'a self,
        forms: &'a [Form],
        local: &'a str,
    ) -> impl Iterator<Item = SourceDeclaration<'a>> + 'a {
        let named = self
            .functions
            .iter()
            .map(|index| (index, true))
            .chain(self.literal_vars.iter().map(|index| (index, false)))
            .chain(self.unchecked_vars.iter().map(|index| (index, false)))
            .chain(self.macros.iter().map(|index| (index, false)))
            .filter_map(move |(&index, function)| {
                let Kind::List(items) = &forms[index].without_metadata().kind else {
                    return None;
                };
                let name = items.get(1)?;
                (name.symbol() == Some(local)).then_some(if function {
                    SourceDeclaration::Function(items)
                } else {
                    SourceDeclaration::Value(items)
                })
            });
        let positional = local.strip_prefix("->");
        let named_map = local.strip_prefix("map->");
        let factories = self
            .type_declarations(forms)
            .filter_map(move |(kind, name, fields)| {
                let symbol = name.symbol()?;
                let arity = if positional == Some(symbol) {
                    Arity::Fixed(fields)
                } else if kind.has_map_factory && named_map == Some(symbol) {
                    Arity::Fixed(1)
                } else {
                    return None;
                };
                Some(SourceDeclaration::TypeFactory { name, arity })
            });
        let scoped = self
            .scoped_functions(forms)
            .filter(move |items| items[1].symbol() == Some(local))
            .map(SourceDeclaration::Function);
        let protocols = self
            .protocol_vars(forms)
            .filter(move |var| var.name.symbol() == Some(local))
            .map(SourceDeclaration::Protocol);
        named.chain(factories).chain(scoped).chain(protocols)
    }

    fn type_declarations<'a>(
        &'a self,
        forms: &'a [Form],
    ) -> impl Iterator<Item = (&'a TypeDeclaration, &'a Form, usize)> {
        self.types.iter().map(move |kind| {
            let Kind::List(items) = &forms[kind.form].without_metadata().kind else {
                unreachable!();
            };
            (kind, &items[1], items[2].vector().unwrap().len())
        })
    }

    pub(super) fn type_factories<'a>(
        &'a self,
        forms: &'a [Form],
    ) -> impl Iterator<Item = (String, Span, Arity)> {
        self.type_declarations(forms)
            .flat_map(|(kind, name, fields)| {
                let symbol = name.symbol().unwrap();
                let span = name.without_metadata().span;
                let positional = (format!("->{symbol}"), span, Arity::Fixed(fields));
                let mapped = kind
                    .has_map_factory
                    .then(|| (format!("map->{symbol}"), span, Arity::Fixed(1)));
                std::iter::once(positional).chain(mapped)
            })
    }

    pub(super) fn protocol_vars<'a>(
        &'a self,
        forms: &'a [Form],
    ) -> impl Iterator<Item = ProtocolVar<'a>> {
        self.protocols.iter().flat_map(move |&index| {
            let Kind::List(items) = &forms[index].without_metadata().kind else {
                unreachable!();
            };
            let owner = &items[1];
            let protocol = ProtocolVar {
                name: owner,
                method: false,
                owner,
                tail: &items[2..],
            };
            let methods = protocol_methods(&items[2..]).iter().filter_map(|form| {
                let Kind::List(items) = &form.without_metadata().kind else {
                    return None;
                };
                let [name, params, ..] = items.as_slice() else {
                    return None;
                };
                (name.symbol().is_some_and(unqualified)
                    && params.vector().is_some_and(|args| !args.is_empty()))
                .then_some(ProtocolVar {
                    name,
                    method: true,
                    owner,
                    tail: &items[1..],
                })
            });
            std::iter::once(protocol).chain(methods)
        })
    }

    pub(super) fn scoped_functions<'a>(
        &'a self,
        forms: &'a [Form],
    ) -> impl Iterator<Item = &'a [Form]> {
        self.scoped_function_forms(forms).map(|(_, form)| {
            let Kind::List(items) = &form.without_metadata().kind else {
                unreachable!();
            };
            items.as_slice()
        })
    }

    pub(super) fn scoped_function_forms<'a>(
        &'a self,
        forms: &'a [Form],
    ) -> impl Iterator<Item = (&'a [Form], &'a Form)> {
        self.scoped_functions.iter().map(|var| {
            let Kind::List(enclosing) = &forms[var.enclosing].without_metadata().kind else {
                unreachable!();
            };
            (enclosing[1].vector().unwrap(), &enclosing[var.definition])
        })
    }

    pub(super) fn source_private(&self, declaration: &SourceDeclaration<'_>) -> bool {
        match declaration {
            SourceDeclaration::Function(items) | SourceDeclaration::Value(items) => {
                self.private_var(items)
            }
            SourceDeclaration::TypeFactory { .. } => false,
            SourceDeclaration::Protocol(var) => var.private(),
        }
    }

    pub(crate) fn source_macro(&self, name: &str, before: Option<usize>) -> bool {
        let Some(declarations) = self.names.source_macros.get(name) else {
            return false;
        };
        match before {
            Some(offset) => declarations.iter().any(|entry| entry.active_at(offset)),
            None => declarations
                .last()
                .is_some_and(|entry| entry.replaced.is_none() && !entry.private),
        }
    }

    pub(crate) fn top_level_call_diagnostics(
        &self,
        forms: &[Form],
        mut public_macro: impl FnMut(&str, &str) -> bool,
        mut public_call: impl FnMut(&str, &str, usize) -> ProjectCall,
    ) -> BTreeMap<usize, Diagnostic> {
        self.unmodeled_calls
            .iter()
            .filter_map(|&index| {
                let form = &forms[index];
                let Kind::List(items) = &form.without_metadata().kind else {
                    return None;
                };
                let op = items[0].symbol()?;
                let (namespace, name) = self.names.resolve_parts(op);
                let offset = form.span.start.offset;
                let local = namespace == self.namespace()
                    || (!op.contains('/') && !self.names.refers.contains_key(op));
                let known_macro = if local && self.source_macro(name, Some(offset)) {
                    true
                } else {
                    namespace != self.namespace() && public_macro(namespace, name)
                };
                let span = items[0].without_metadata().span;
                let diagnostic = if known_macro {
                    Diagnostic::analysis_gap(
                        "PNX9001",
                        span,
                        format!("Expansion of source macro {op} is unchecked"),
                    )
                } else if !local {
                    let actual = items.len() - 1;
                    match public_call(namespace, name, actual) {
                        ProjectCall::Function {
                            expected_arity: Some(expected),
                            ..
                        } => Diagnostic::new(
                            "PNX2554",
                            span,
                            format!("{op} expects {expected} arguments, got {actual}"),
                        ),
                        ProjectCall::Function {
                            expected_arity: None,
                            ..
                        } => Diagnostic::analysis_gap(
                            "PNX9001",
                            span,
                            format!(
                                "Analysis of top-level call to source function {op} is unavailable"
                            ),
                        ),
                        _ => return None,
                    }
                } else {
                    return None;
                };
                Some((offset, diagnostic))
            })
            .collect()
    }

    pub(super) fn private_var(&self, items: &[Form]) -> bool {
        let private =
            |form: &Form| !matches!(form.without_metadata().kind, Kind::Nil | Kind::Bool(false));
        let op = items[0].symbol().unwrap();
        if op == "def" {
            return items[1].metadata(":private").is_some_and(private);
        }
        let (namespace, name) = self.names.resolve_parts(op);
        if (namespace, name) == ("clojure.core", "defmulti") {
            return multimethod_parts(&items[2..])
                .0
                .as_chunks::<2>()
                .0
                .iter()
                .find_map(|[key, value]| (key.keyword() == Some(":private")).then_some(value))
                .or_else(|| items[1].metadata(":private"))
                .is_some_and(private);
        }
        if (namespace, name) == ("clojure.core", "defonce") {
            return items[1].metadata(":private").is_some_and(private);
        }
        if let Some(value) = Self::function_privacy_attribute(&items[2..]) {
            return private(value);
        }
        (namespace, name) == ("clojure.core", "defn-")
            || items[1].metadata(":private").is_some_and(private)
    }

    pub(crate) fn function_privacy_attribute(tail: &[Form]) -> Option<&Form> {
        function_parts(tail)
            .1
            .find_map(|[key, value]| (key.keyword() == Some(":private")).then_some(value))
    }

    pub(crate) fn privacy_operator(&self, forms: &[Form], operator: &Form) -> Option<&'static str> {
        let spelling = operator.symbol()?;
        if spelling == "def" {
            return Some("def");
        }
        if !spelling.contains('/')
            && !self.core_completion_visible(forms, spelling, operator.span.start.offset)
        {
            return None;
        }
        match self.names.resolve_parts(spelling) {
            ("clojure.core", "defn") => Some("defn"),
            ("clojure.core", "defn-") => Some("defn-"),
            ("clojure.core", "defonce") => Some("defonce"),
            _ => None,
        }
    }

    pub(super) fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.opaque_var_surface = true;
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    fn references_assert_expr(&self, forms: &[Form]) -> bool {
        forms.iter().enumerate().any(|(index, form)| {
            if self.discarded_comments.binary_search(&index).is_ok() {
                return false;
            }
            !symbols_match_inner::<true>(form, &mut |name| {
                name != "assert-expr"
                    && (!name.ends_with("/assert-expr")
                        || self.names.resolve_parts(name) != ("clojure.test", "assert-expr"))
            })
        })
    }

    pub(super) fn read_with_imports(
        forms: &[Form],
        globals: Option<&Globals>,
        proven_alias_libraries: BTreeSet<String>,
        unchecked_dispatches: Option<Arc<UncheckedDispatches>>,
    ) -> Self {
        let mut model = Self {
            names: Names {
                namespace: "user".into(),
                proven_alias_libraries,
                ..Names::default()
            },
            unchecked_dispatches,
            ..Self::default()
        };
        if let Some(dispatch) = model
            .unchecked_dispatches
            .as_ref()
            .and_then(|dispatches| dispatches.first_omitted)
        {
            let (span, message) = match dispatch {
                OmittedDispatch::Tag(span) => (span, "A data reader call inside #_ is unchecked"),
                OmittedDispatch::Regex(span) => (
                    span,
                    "JVM regex validation in a discarded or inactive form is unchecked",
                ),
                OmittedDispatch::MetadataTag(span) => (
                    span,
                    "A data reader call in a metadata descriptor is unchecked",
                ),
                OmittedDispatch::MetadataRegex(span) => (
                    span,
                    "JVM regex validation in a metadata descriptor is unchecked",
                ),
            };
            model
                .diagnostics
                .push(Diagnostic::analysis_gap("PNX9001", span, message));
        }
        let mut seen_ns = false;
        for form in forms {
            if let Kind::List(xs) = &form.without_metadata().kind
                && matches!(
                    xs.first().and_then(Form::symbol),
                    Some("ns" | "clojure.core/ns")
                )
            {
                if seen_ns {
                    model.error("PNX9001", form.span, "Only one ns declaration is supported");
                    if xs
                        .get(1)
                        .and_then(Form::symbol)
                        .is_some_and(|name| name != model.namespace())
                    {
                        model.names.aliases.clear();
                    }
                }
                seen_ns = true;
                model.namespace_end = Some(form.span.end.offset);
                model.read_ns(xs, form.span, globals);
            }
        }
        if !model.opaque_var_surface
            && model.all_refers.is_empty()
            && let [namespace, first, rest @ ..] = forms
            && let Some(first) = reflection_flag_assignment(first, &model.names)
            && static_namespace_prelude(namespace)
        {
            model.reflection_flags = std::iter::once(first)
                .chain(
                    rest.iter()
                        .map_while(|form| reflection_flag_assignment(form, &model.names)),
                )
                .collect();
        }
        for (index, form) in forms.iter().enumerate() {
            if index > 0 && index <= model.reflection_flags.len() {
                continue;
            }
            let Kind::List(xs) = &form.without_metadata().kind else {
                model.error("PNX9001", form.span, "Unsupported top-level form");
                continue;
            };
            let Some(op) = xs.first().and_then(Form::symbol) else {
                model.error("PNX9001", form.span, "Expected a top-level declaration");
                continue;
            };
            model.has_top_level_require |= matches!(op, "require" | "clojure.core/require");
            model.has_top_level_in_ns |= matches!(op, "in-ns" | "clojure.core/in-ns");
            match op {
                "ns" | "clojure.core/ns" => continue,
                "def" => {
                    model.literal_vars.push(index);
                    if let [_, name, ..] = xs.as_slice()
                        && xs.len() <= 4
                    {
                        model.replace_source_macro(name, None, form.span);
                    }
                    continue;
                }
                "set!" if compiler_flag_assignment(xs, &model.names) => {
                    model.diagnostics.push(compiler_flag_gap(xs[0].span));
                    continue;
                }
                _ => {}
            }
            if model
                .names
                .local_source_macros(op)
                .and_then(|(_, declarations)| declarations.last())
                .is_some_and(|entry| entry.active_at(form.span.start.offset))
            {
                model.read_unmodeled_call(index, op, form.span);
                continue;
            }
            if let Some(namespace) = model.runtime_alias_gap(op) {
                let finding =
                    Diagnostic::runtime_alias_gap(op, namespace, xs[0].without_metadata().span);
                model.opaque_var_surface = true;
                model.diagnostics.push(finding);
                continue;
            }
            let resolved = model.names.resolve(op);
            match resolved.as_str() {
                "clojure.core/ns" => {}
                "clojure.core/comment"
                    if !model.opaque_var_surface
                        && !model.has_unchecked_dispatch(form.span)
                        && model.namespace() != "clojure.core"
                        && globals
                            .is_none_or(|g| !g.project_namespaces.contains("clojure.core"))
                        && (op.contains('/')
                            || (model.all_refers.is_empty()
                                && model
                                    .source_var_before(forms, op, form.span.start.offset)
                                    .is_none())) =>
                {
                    // The standard macro expands to nil before compilation.
                    model.discarded_comments.push(index);
                }
                "clojure.spec.alpha/def" if xs.len() == 3 => {
                    if let Some(key) = xs[1].keyword() {
                        let key = model.names.key(key);
                        let declaration = SpecDeclaration {
                            name: xs[1].span,
                            body: xs[2].clone(),
                        };
                        if model.specs.insert(key.clone(), declaration).is_some() {
                            model.error("PNX2790", xs[1].span, format!("Duplicate spec {key}"));
                            model.duplicate_specs.insert(key);
                        }
                    } else {
                        model.error("PNX9001", xs[1].span, "Spec name must be a keyword");
                    }
                }
                "clojure.core/defn" | "clojure.core/defn-" => {
                    model.functions.push(index);
                    if let [_, name, tail @ ..] = xs.as_slice()
                        && name.symbol().is_some_and(|name| {
                            model.source_macro(name, Some(form.span.start.offset))
                        })
                    {
                        let (body, mut attributes) = function_parts(tail);
                        if !function_clauses(body, form.span)
                            .is_err_and(|diagnostic| diagnostic.severity == "error")
                        {
                            let marker = attributes.find_map(|[key, value]| {
                                (key.keyword() == Some(":macro")).then_some(value)
                            });
                            model.replace_source_macro(name, marker, form.span);
                        }
                    }
                }
                "clojure.core/declare" => {
                    model.read_declare(&xs[1..]);
                    for name in &xs[1..] {
                        model.replace_source_macro(name, None, form.span);
                    }
                }
                "clojure.spec.alpha/fdef" => model.fdefs.push(index),
                qualified if known_unsupported_declaration(qualified, xs) => {
                    match qualified {
                        "clojure.core/defrecord" | "clojure.core/deftype" => {
                            model.types.push(TypeDeclaration {
                                form: index,
                                has_map_factory: qualified == "clojure.core/defrecord",
                            });
                        }
                        "clojure.core/defprotocol" => {
                            model.protocols.push(index);
                            model.opaque_var_surface = true;
                        }
                        "clojure.core/extend-type" | "clojure.core/extend-protocol" => {
                            model.protocol_extensions.push(index);
                            model.opaque_var_surface = true;
                        }
                        "clojure.core/defmethod" => {
                            if op.contains('/')
                                || (model.all_refers.is_empty()
                                    && model.source_declarations(forms, op).next().is_none()
                                    && !model.forward_vars.contains_key(op))
                            {
                                model.method_forms.push(index);
                            } else {
                                model.opaque_var_surface = true;
                            }
                        }
                        "clojure.core/let" => {
                            if op.contains('/')
                                || (model.all_refers.is_empty()
                                    && model.source_declarations(forms, op).next().is_none())
                            {
                                model.read_scoped_functions(forms, index, xs);
                            }
                            model.opaque_var_surface = true;
                        }
                        "clojure.core/defmacro" => {
                            model.macros.push(index);
                            if let Some(name) = xs.get(1).and_then(Form::symbol) {
                                let private = model.private_var(xs);
                                let offset = form.span.end.offset;
                                let declarations =
                                    model.names.source_macros.entry(name.into()).or_default();
                                if let Some(previous) = declarations.last_mut() {
                                    previous.replaced.get_or_insert(offset);
                                }
                                declarations.push(SourceMacro {
                                    defined: offset,
                                    replaced: None,
                                    private,
                                });
                            }
                            model.opaque_var_surface = true;
                        }
                        "clojure.core/defonce" | "clojure.core/defmulti" => {
                            model.unchecked_vars.push(index);
                            let marker = if qualified == "clojure.core/defmulti" {
                                multimethod_parts(&xs[2..])
                                    .0
                                    .as_chunks::<2>()
                                    .0
                                    .iter()
                                    .find_map(|[key, value]| {
                                        (key.keyword() == Some(":macro")).then_some(value)
                                    })
                            } else {
                                None
                            };
                            model.replace_source_macro(&xs[1], marker, form.span);
                            model.opaque_var_surface = true;
                        }
                        "clojure.test/deftest" | "clojure.test/deftest-" => {
                            model.test_forms.push(index);
                            model.opaque_var_surface = true;
                        }
                        _ => model.opaque_var_surface = true,
                    }
                    let message =
                        if matches!(qualified, "clojure.test/deftest" | "clojure.test/deftest-") {
                            format!("Type checking for {op} declarations is unavailable")
                        } else if qualified == "clojure.core/defmethod" {
                            format!("Dispatch and type checking for {op} are unavailable")
                        } else {
                            format!("Analysis for {op} declarations is unavailable")
                        };
                    model
                        .diagnostics
                        .push(Diagnostic::analysis_gap("PNX9001", xs[0].span, message));
                }
                _ => model.read_unmodeled_call(index, op, form.span),
            }
        }
        if model.read_dependencies(forms)
            || model.has_inline_spec_composition(forms)
            || model.has_preserving_spec(forms)
        {
            model.read_spec_core_scope(forms);
        }
        model.static_method_scope = model.has_static_method_scope(forms);
        model
    }

    pub(super) fn completed_before_last_tag(&self) -> usize {
        self.unchecked_dispatches
            .as_ref()
            .map_or(0, |dispatches| dispatches.completed_before_last_tag)
    }

    pub(super) fn first_unchecked_dispatch(&self) -> Option<usize> {
        let dispatches = self.unchecked_dispatches.as_ref()?;
        dispatches
            .first_tag
            .into_iter()
            .map(|tag| tag.start.offset)
            .chain(dispatches.regexes.first().copied())
            .min()
    }

    pub(super) fn has_unchecked_dispatch(&self, span: Span) -> bool {
        self.unchecked_dispatches
            .as_ref()
            .is_some_and(|dispatches| {
                // Earlier tags can execute code before this macro is resolved.
                let regex = dispatches
                    .regexes
                    .partition_point(|offset| *offset < span.start.offset);
                dispatches
                    .first_tag
                    .is_some_and(|tag| tag.start.offset < span.end.offset)
                    || dispatches
                        .regexes
                        .get(regex)
                        .is_some_and(|offset| *offset < span.end.offset)
            })
    }

    fn read_dependencies(&mut self, forms: &[Form]) -> bool {
        let mut dependencies: BTreeSet<_> = self
            .names
            .required_libraries
            .iter()
            .map(String::as_str)
            .collect();
        let mut uses_spec_composition = false;
        let mut comments = self.discarded_comments.iter().copied().peekable();
        for (index, form) in forms.iter().enumerate() {
            if comments.next_if_eq(&index).is_some() {
                dependencies.insert("clojure.core");
                continue;
            }
            symbols_match(form, &mut |name| {
                let (namespace, local) = self.names.resolve_parts(name);
                dependencies.insert(namespace);
                uses_spec_composition |= (namespace == "clojure.spec.alpha"
                    && (matches!(local, "and" | "or" | "conformer" | "coll-of")
                        || (local == "keys" && !name.contains('/'))))
                    || (namespace == "clojure.core" && matches!(local, "integer?" | "int?"))
                    || (namespace == "clojure.set" && local == "map-invert");
                true
            });
        }
        dependencies.remove(self.namespace());
        self.dependencies = dependencies.into_iter().map(str::to_owned).collect();
        uses_spec_composition
    }

    fn read_unmodeled_call(&mut self, index: usize, op: &str, span: Span) {
        self.unmodeled_calls.push(index);
        self.error(
            "PNX9001",
            span,
            format!("Unsupported top-level declaration {op}"),
        );
    }

    fn replace_source_macro(&mut self, name: &Form, marker: Option<&Form>, span: Span) {
        let Some(previous) = name
            .symbol()
            .and_then(|name| self.names.source_macros.get_mut(name))
            .and_then(|declarations| declarations.last_mut())
        else {
            return;
        };
        if marker
            .or_else(|| name.metadata(":macro"))
            .is_none_or(|value| {
                matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
            })
        {
            // The old macro is still visible while compiling the replacement.
            previous.replaced.get_or_insert(span.end.offset);
        }
    }

    fn read_scoped_functions(&mut self, forms: &[Form], enclosing: usize, items: &[Form]) {
        let bindings = items[1].vector().unwrap();
        let mut locals = Vec::new();
        for [binding, _] in bindings.as_chunks::<2>().0 {
            binding_names(binding, &mut locals);
        }
        for (definition, form) in items.iter().enumerate().skip(2) {
            let Kind::List(parts) = &form.without_metadata().kind else {
                continue;
            };
            let [head, name, tail @ ..] = parts.as_slice() else {
                continue;
            };
            let (Some(op), Some(_)) = (
                head.symbol(),
                name.symbol().filter(|name| unqualified(name)),
            ) else {
                continue;
            };
            if !matches!(
                self.names.resolve_parts(op),
                ("clojure.core", "defn" | "defn-")
            ) {
                continue;
            }
            let shadowed = !op.contains('/')
                && (!self.all_refers.is_empty()
                    || locals.iter().any(|(local, _)| *local == op)
                    || self.source_declarations(forms, op).next().is_some());
            if !shadowed
                && !function_clauses(function_parts(tail).0, form.span)
                    .is_err_and(|diagnostic| diagnostic.severity == "error")
            {
                self.scoped_functions.push(ScopedFunction {
                    enclosing,
                    definition,
                });
            }
        }
    }

    fn read_ns(&mut self, xs: &[Form], span: Span, globals: Option<&Globals>) {
        let Some(name) = xs.get(1).and_then(Form::symbol) else {
            self.error("PNX9001", span, "ns requires a namespace name");
            return;
        };
        self.names.namespace = name.to_string();
        let mut has_core_clause = false;
        for clause in xs.iter().skip(2) {
            // Docstrings and attribute maps do not introduce namespace clauses.
            if clause.is_string() || matches!(clause.kind, Kind::Map(_)) {
                continue;
            }
            let Kind::List(items) = &clause.without_metadata().kind else {
                self.error("PNX9001", clause.span, "Unsupported ns clause");
                continue;
            };
            match items.first().and_then(Form::keyword) {
                Some(clause @ (":require" | ":use")) => {
                    for lib in &items[1..] {
                        self.read_libspec(lib, globals, clause == ":use");
                    }
                }
                Some(":import") => {
                    for form in &items[1..] {
                        self.read_import(form);
                    }
                }
                Some(":refer-clojure") if !has_core_clause => {
                    has_core_clause = true;
                    self.read_core_refers(&items[1..], clause.span);
                }
                Some(":refer-clojure") => self.error(
                    "PNX9001",
                    clause.span,
                    "Only one :refer-clojure clause is supported",
                ),
                Some(":gen-class") if items.len() == 1 => {
                    self.diagnostics.push(Diagnostic::analysis_gap(
                        "PNX9001",
                        items[0].span,
                        "JVM class generation for :gen-class is unchecked",
                    ));
                }
                _ => self.error("PNX9001", clause.span, "Unsupported ns clause"),
            }
        }
    }

    fn read_declare(&mut self, names: &[Form]) {
        for name in names {
            let Some(symbol) = name
                .symbol()
                .filter(|name| !name.contains('/') || *name == "/")
            else {
                self.error("PNX1001", name.span, "declare requires unqualified symbols");
                continue;
            };
            let private = name.metadata(":private").is_some_and(|value| {
                !matches!(value.without_metadata().kind, Kind::Nil | Kind::Bool(false))
            });
            self.forward_vars.insert(
                symbol.into(),
                DeclaredVar {
                    span: name.without_metadata().span,
                    private,
                },
            );
        }
    }
}

fn known_unsupported_declaration(qualified: &str, items: &[Form]) -> bool {
    match qualified {
        "clojure.core/comment" => return true,
        "clojure.core/let" => {
            return items
                .get(1)
                .and_then(Form::vector)
                .is_some_and(|bindings| bindings.len().is_multiple_of(2));
        }
        "clojure.core/require" | "clojure.test/testing" => return items.len() >= 2,
        "clojure.test/use-fixtures" => {
            return matches!(
                items.get(1).and_then(Form::keyword),
                Some(":once" | ":each")
            );
        }
        "clojure.core/extend" => return items.len() >= 2 && items.len().is_multiple_of(2),
        _ => {}
    }
    if matches!(
        qualified,
        "clojure.core/extend-type" | "clojure.core/extend-protocol"
    ) {
        let Some((target, specs)) = items[1..].split_first() else {
            return false;
        };
        let extend_type = qualified == "clojure.core/extend-type";
        return (target.symbol().is_some()
            || (extend_type && matches!(target.without_metadata().kind, Kind::Nil)))
            && extension_specs(specs, !extend_type);
    }
    let Some(name) = items.get(1).and_then(Form::symbol) else {
        return false;
    };
    match qualified {
        "clojure.core/defrecord" | "clojure.core/deftype" => {
            !name.contains('/') && items.get(2).and_then(Form::vector).is_some()
        }
        "clojure.core/definterface" => !name.contains('/') && items[2..].iter().all(|method| {
            matches!(&method.without_metadata().kind, Kind::List(xs)
                if matches!(xs.as_slice(), [name, params] if name.symbol().is_some() && params.vector().is_some()))
        }),
        "clojure.core/defprotocol" => !name.contains('/'),
        "clojure.core/defonce" => unqualified(name) && items.len() == 3,
        "clojure.core/defmulti" => {
            let (_, clauses) = multimethod_parts(&items[2..]);
            unqualified(name)
                && clauses.split_first().is_some_and(|(_, options)| {
                    let (pairs, remainder) = options.as_chunks::<2>();
                    remainder.is_empty()
                        && pairs.iter().all(|[key, _]| {
                            matches!(key.keyword(), Some(":default" | ":hierarchy"))
                        })
                })
        }
        "clojure.core/defmacro" => !function_clauses(function_parts(&items[2..]).0, items[1].span)
            .is_err_and(|diagnostic| diagnostic.severity == "error"),
        "clojure.core/defmethod" => items.len() >= 4,
        "clojure.test/deftest" | "clojure.test/deftest-" => true,
        "clojure.test.check.clojure-test/defspec" => items.len() >= 3,
        _ => false,
    }
}

fn compiler_flag_gap(span: Span) -> Diagnostic {
    Diagnostic::analysis_gap(
        "PNX9001",
        span,
        "Analysis for compiler flag assignments is unavailable",
    )
}

fn reflection_flag_assignment(form: &Form, names: &Names) -> Option<Span> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [head, target, value] = items.as_slice() else {
        return None;
    };
    let (Kind::Symbol(op), Kind::Symbol(target), Kind::Bool(_)) =
        (&head.kind, &target.kind, &value.kind)
    else {
        return None;
    };
    (op == "set!" && names.resolve_parts(target) == ("clojure.core", "*warn-on-reflection*"))
        .then_some(head.span)
}

pub(super) fn static_namespace_prelude(form: &Form) -> bool {
    let Kind::List(items) = &form.kind else {
        return false;
    };
    let [head, name, clauses @ ..] = items.as_slice() else {
        return false;
    };
    if !matches!(&head.kind, Kind::Symbol(op) if matches!(op.as_str(), "ns" | "clojure.core/ns")) {
        return false;
    }
    // The JVM ns macro evaluates the merged name metadata and attribute map.
    let mut name = name;
    while let Kind::Metadata(metadata, target) = &name.kind {
        if !matches!(metadata.kind, Kind::Map(_)) || !inert_namespace_attribute(metadata) {
            return false;
        }
        name = target;
    }
    if !matches!(&name.kind, Kind::Symbol(symbol) if !symbol.contains('/')) {
        return false;
    }
    let clauses = match clauses {
        [doc, rest @ ..] if doc.is_string() => rest,
        _ => clauses,
    };
    let clauses = match clauses {
        [attrs, rest @ ..] if matches!(attrs.kind, Kind::Map(_)) => {
            if !inert_namespace_attribute(attrs) {
                return false;
            }
            rest
        }
        _ => clauses,
    };
    clauses.iter().all(|clause| {
        let Kind::List(items) = &clause.kind else {
            return false;
        };
        let Some(Kind::Keyword(key)) = items.first().map(|head| &head.kind) else {
            return false;
        };
        match key.as_str() {
            ":require" | ":use" => {
                items.len() > 1
                    && items[1..].iter().all(|libspec| {
                        !matches!(&libspec.without_metadata().kind, Kind::List(group) if group.len() < 2)
                    })
            }
            ":import" | ":refer-clojure" | ":gen-class" => true,
            _ => false,
        }
    })
}

fn inert_namespace_attribute(form: &Form) -> bool {
    match &form.kind {
        Kind::Nil
        | Kind::Bool(_)
        | Kind::Number
        | Kind::SymbolicNumber(_)
        | Kind::Character(_)
        | Kind::String(_)
        | Kind::Utf16String(_) => true,
        Kind::Keyword(keyword) => !keyword.starts_with("::"),
        Kind::Vector(items) => items.iter().all(inert_namespace_attribute),
        Kind::Map(entries) => {
            let (pairs, remainder) = entries.as_chunks::<2>();
            remainder.is_empty()
                && pairs.iter().all(|[key, value]| {
                    matches!(&key.kind, Kind::Keyword(keyword) if !keyword.starts_with("::"))
                        && inert_namespace_attribute(value)
                })
        }
        _ => false,
    }
}

fn compiler_flag_assignment(items: &[Form], names: &Names) -> bool {
    let [_, target, value] = items else {
        return false;
    };
    let Some(symbol) = target.symbol() else {
        return false;
    };
    let boolean = matches!(value.without_metadata().kind, Kind::Bool(_));
    match names.resolve_parts(symbol) {
        ("clojure.core", "*assert*" | "*warn-on-reflection*") => boolean,
        ("clojure.core", "*unchecked-math*") => {
            boolean || value.keyword() == Some(":warn-on-boxed")
        }
        _ => false,
    }
}

fn extension_specs(specs: &[Form], allow_nil: bool) -> bool {
    let mut in_group = false;
    for spec in specs {
        if spec.symbol().is_some()
            || (allow_nil && matches!(spec.without_metadata().kind, Kind::Nil))
        {
            in_group = true;
        } else if !in_group
            || !matches!(&spec.without_metadata().kind, Kind::List(parts)
            if matches!(parts.split_first(), Some((name, clauses)) if name.symbol().is_some()
                && !function_clauses(clauses, spec.span).is_err_and(|diagnostic| diagnostic.severity == "error")))
        {
            return false;
        }
    }
    true
}

fn unqualified(name: &str) -> bool {
    !name.contains('/') || name == "/"
}

pub(super) fn multimethod_parts(tail: &[Form]) -> (&[Form], &[Form]) {
    let tail = if tail.first().is_some_and(Form::is_string) {
        &tail[1..]
    } else {
        tail
    };
    match tail.split_first() {
        Some((first, rest)) if first.map().is_some() => (first.map().unwrap(), rest),
        _ => (&[], tail),
    }
}

fn protocol_methods(mut tail: &[Form]) -> &[Form] {
    loop {
        match tail {
            [head, rest @ ..] if head.is_string() => tail = rest,
            [head, _, rest @ ..] if head.keyword().is_some() => tail = rest,
            _ => return tail,
        }
    }
}

pub(super) fn symbols_match<'a>(form: &'a Form, test: &mut impl FnMut(&'a str) -> bool) -> bool {
    symbols_match_inner::<false>(form, test)
}

fn symbols_match_inner<'a, const QUOTED: bool>(
    form: &'a Form,
    test: &mut impl FnMut(&'a str) -> bool,
) -> bool {
    match &form.kind {
        Kind::Symbol(name) => test(name),
        Kind::List(forms) if !QUOTED && forms.first().and_then(Form::symbol) == Some("quote") => {
            true
        }
        Kind::List(forms) | Kind::Vector(forms) | Kind::Map(forms) | Kind::Set(forms) => forms
            .iter()
            .all(|form| symbols_match_inner::<QUOTED>(form, test)),
        Kind::AnonymousFunction(function) => function
            .body
            .iter()
            .all(|form| symbols_match_inner::<QUOTED>(form, test)),
        Kind::Metadata(metadata, target) => {
            symbols_match_inner::<QUOTED>(metadata, test)
                && symbols_match_inner::<QUOTED>(target, test)
        }
        Kind::Quote(form) | Kind::SyntaxQuote(form) | Kind::Unquote { form, .. } if QUOTED => {
            symbols_match_inner::<QUOTED>(form, test)
        }
        Kind::AutoNamespacedMap { entries, .. } if QUOTED => entries
            .iter()
            .all(|form| symbols_match_inner::<QUOTED>(form, test)),
        Kind::Tagged(tag, value) if QUOTED => {
            symbols_match_inner::<QUOTED>(tag, test) && symbols_match_inner::<QUOTED>(value, test)
        }
        _ => true,
    }
}
