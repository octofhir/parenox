//! Namespace resolution, project interfaces and cache dependencies.
mod contracts;
mod data_functions;
mod declarations;
mod externs;
mod fdefs;
mod guards;
mod imports;
mod interfaces;
mod multimethods;
mod partial;
mod references;
mod require;
mod spec_navigation;
mod specs;
mod test_bodies;
mod type_forms;
mod type_queries;
mod vars;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) use declarations::Declarations;
use declarations::Names;
pub(super) use externs::MacroScope;
pub(crate) use externs::{EncodeError, ExternCache, SnapshotWriter};
use externs::{ExternFile, ExternalDeclaration, Externs, LiteralExtern};
pub(super) use guards::{GuardMode, VerifiedGuard};
pub(crate) use imports::{bare_dotted_name, java_type_tag_at};
pub(crate) use interfaces::InterfaceKey;
use interfaces::{
    Exports, LocalInterface, VarInventory, proves_missing_var, source_macro_value, visible_value,
};
pub(crate) use references::{DocumentReferences, ProjectCall, ScopedReference};
pub(crate) use require::RuntimeLoads;
pub(crate) use specs::{
    GuardedMapProjection, MapFieldConversion, SpecCache, SpecConversion, SpecFingerprint,
    SpecProvider, SpecSource, StructuredConversion,
};
use vars::{CapturedInitializer, LiteralVar};

use crate::reader::{Form, Kind, Span};
use crate::types::{Arity, Signature, Type};
use crate::{ContractKind, ContractOrigin, Diagnostic};

#[derive(Clone, Debug)]
pub(crate) struct Function {
    pub name: String,
    pub private: bool,
    pub arities: BTreeMap<Arity, Span>,
    pub contract: Option<Contract>,
    pub(super) contract_span: Option<Span>,
    data_body_eligible: bool,
    ordinary_dotted: bool,
    pub span: Span,
    // Index in the immutable source forms shared with the file snapshot.
    source: declarations::FunctionSource,
}

impl Function {
    // Private definitions without a signature are analyzed through direct typed callers.
    pub fn inferred(&self) -> bool {
        self.contract.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Contract {
    pub signatures: Arc<[Signature]>,
    pub origin: Arc<ContractOrigin>,
    pub guard: Option<Arc<VerifiedGuard>>,
}

impl Contract {
    fn new(signatures: Vec<Signature>, name: &str, kind: ContractKind, path: &str) -> Self {
        Self {
            signatures: signatures.into(),
            origin: Arc::new(ContractOrigin {
                name: name.into(),
                kind,
                path: Some(path.into()),
            }),
            guard: None,
        }
    }

    fn same_interface(&self, other: &Self) -> bool {
        self.origin == other.origin
            && self.guard == other.guard
            && self.signatures.len() == other.signatures.len()
            && self
                .signatures
                .iter()
                .zip(other.signatures.iter())
                .all(|(a, b)| a.same_type(b))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UncheckedFunction {
    pub arities: Option<Arc<[Arity]>>,
    private: bool,
}

#[derive(PartialEq, Eq)]
pub(super) enum DeclaredValue<'a> {
    Literal(&'a LiteralVar),
    ExternLiteral(&'a LiteralExtern),
    Function(&'a Contract),
    UncheckedFunction(&'a UncheckedFunction),
    Macro(&'a MacroScope),
    OpaqueMacro,
    PrivateMacro,
}

#[derive(Clone, Default)]
pub(crate) struct Globals {
    specs: Arc<BTreeMap<String, Type>>,
    spec_conversions: Arc<BTreeMap<String, SpecConversion>>,
    spec_unchecked_inputs: Arc<BTreeSet<String>>,
    spec_providers: Arc<BTreeMap<String, Arc<SpecProvider>>>,
    pub(super) project_namespaces: BTreeSet<String>,
    pub(crate) runtime_loads: RuntimeLoads,
    exports: BTreeMap<String, Arc<Exports>>,
    root_writes: BTreeMap<String, BTreeSet<String>>,
    externs: Option<Arc<Externs>>,
}

impl Globals {
    pub fn with_specs(cache: &SpecCache, runtime_loads: RuntimeLoads) -> Self {
        Self {
            specs: cache.types.clone(),
            spec_conversions: cache.conversions.clone(),
            spec_unchecked_inputs: cache.unchecked_inputs.clone(),
            spec_providers: cache.providers.clone(),
            runtime_loads,
            ..Self::default()
        }
    }

    pub fn register_namespace(&mut self, namespace: &str) {
        self.project_namespaces.insert(namespace.into());
    }

    pub(crate) fn register_root_write(&mut self, namespace: &str, name: &str) {
        if self.project_namespaces.contains(namespace) {
            self.root_writes
                .entry(namespace.into())
                .or_default()
                .insert(name.into());
        }
    }

    pub fn spec(&self, key: &str) -> Option<&Type> {
        self.specs.get(key)
    }

    pub fn spec_conversion(&self, key: &str) -> Option<SpecConversion> {
        self.spec_conversions.get(key).cloned()
    }

    /// Registration sources used to check exact registry operations.
    pub(super) fn spec_provider(&self, key: &str) -> Option<&Arc<SpecProvider>> {
        self.spec_providers.get(key)
    }

    fn declared_value(&self, name: &str) -> Option<DeclaredValue<'_>> {
        let (namespace, _) = name.split_once('/')?;
        visible_value(
            self.exports.get(namespace).map(Arc::as_ref),
            || {
                self.externs
                    .as_ref()
                    .and_then(|table| table.namespaces.get(namespace))
                    .map(Arc::as_ref)
            },
            name,
        )
    }

    fn has_competing_class_namespace(&self, name: &str) -> bool {
        self.project_namespaces.contains(name)
            || self
                .externs
                .as_ref()
                .is_some_and(|table| table.namespaces.contains_key(name))
    }

    fn extern_declaration(&self, name: &str) -> Option<&ExternalDeclaration> {
        let (namespace, _) = name.split_once('/')?;
        self.externs.as_ref()?.namespaces.get(namespace)?.get(name)
    }
}

#[derive(Default)]
pub(crate) struct Model {
    names: Names,
    path: String,
    source_forms: Arc<[Form]>,
    externs: Option<Arc<ExternFile>>,
    literals: BTreeMap<String, LiteralVar>,
    root_writes: BTreeSet<String>,
    captured_initializers: BTreeMap<usize, CapturedInitializer>,
    initializer_macros: BTreeMap<String, bool>,
    initializer_class_names: BTreeMap<String, bool>,
    unchecked_functions: BTreeMap<String, UncheckedFunction>,
    exports: Arc<Exports>,
    local_interface: Arc<LocalInterface>,
    external_specs: BTreeMap<String, (Option<Type>, bool)>,
    external_refers: BTreeMap<String, Option<Arc<Exports>>>,
    unavailable_all_refers: bool,
    core_in_project: Option<bool>,
    set_namespace_in_project: Option<bool>,
    pub functions: BTreeMap<String, Function>,
    multimethod_contracts: BTreeMap<String, (usize, Contract)>,
    checked_data_functions: BTreeSet<String>,
    // Conservative references to unannotated helpers, including transitive cache dependencies.
    helper_dependencies: BTreeMap<String, BTreeSet<String>>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Model {
    pub(crate) fn check_top_level_contract_call(
        &self,
        form: &Form,
        name: &str,
        globals: &Globals,
        source: &str,
    ) -> Option<Vec<Diagnostic>> {
        if self.has_unavailable_wildcard_scope(name) || crate::checker::has_evaluated_metadata(form)
        {
            return None;
        }
        let Some(DeclaredValue::Function(contract)) =
            self.declared_value(name, globals, form.span.start.offset)
        else {
            return None;
        };
        if contract
            .origin
            .name
            .split_once('/')
            .is_some_and(|(namespace, local)| {
                globals
                    .root_writes
                    .get(namespace)
                    .is_some_and(|writes| writes.contains(local))
            })
        {
            return None;
        }
        let mut checker = super::Checker::new(self, globals, source);
        checker.infer(form, &super::Env::new());
        Some(checker.diagnostics)
    }

    pub(super) fn loads_spec(&self, provider: &SpecProvider) -> bool {
        let loaded = |source: &specs::RegistrationSource| {
            self.path == source.path || self.names.loads_library(&source.namespace)
        };
        match provider {
            SpecProvider::Direct(source) | SpecProvider::Imported { source, .. } => loaded(source),
            SpecProvider::LoadChecked { sources, .. } => {
                sources.iter().all(|source| loaded(source))
            }
        }
    }

    pub(crate) fn resolved_referral(&self, name: &str) -> Option<(&str, &str)> {
        self.names.refers.get(name)?.split_once('/')
    }

    pub fn build(
        declarations: &Declarations,
        source_forms: &Arc<[Form]>,
        source: &str,
        path: &str,
        globals: &Globals,
    ) -> Self {
        let forms = source_forms.as_ref();
        let resolved;
        let comment_core_in_project = (!declarations.discarded_comments.is_empty())
            .then(|| globals.project_namespaces.contains("clojure.core"));
        let declarations =
            if declarations.all_refers.is_empty() && comment_core_in_project != Some(true) {
                declarations
            } else {
                let mut imports = Declarations::read_with_imports(
                    forms,
                    Some(globals),
                    declarations.proven_alias_libraries().clone(),
                    declarations.unchecked_dispatches.clone(),
                );
                imports.namespace_read_end = declarations.namespace_read_end;
                if imports.specs.keys().ne(declarations.specs.keys()) {
                    let span = forms.first().map_or(Span::default(), |form| form.span);
                    imports.error(
                        "PNX9001",
                        span,
                        "Use qualified spec declarations when :refer :all changes their resolution",
                    );
                }
                resolved = imports;
                &resolved
            };
        let mut model = Self {
            names: declarations.names.clone(),
            path: path.into(),
            root_writes: globals
                .root_writes
                .get(declarations.namespace())
                .cloned()
                .unwrap_or_default(),
            source_forms: source_forms.clone(),
            core_in_project: comment_core_in_project,
            diagnostics: declarations.diagnostics.clone(),
            external_refers: declarations
                .all_refers
                .iter()
                .map(|namespace| (namespace.clone(), globals.exports.get(namespace).cloned()))
                .collect(),
            unavailable_all_refers: declarations.unavailable_all_refers,
            ..Self::default()
        };
        if !declarations.reflection_flags.is_empty() && model.check_project_core(globals) {
            model
                .diagnostics
                .extend(declarations.reflection_flag_gaps());
        }
        let mut specs = BTreeMap::new();
        for key in declarations.specs.keys() {
            let ty = globals.specs.get(key).cloned();
            specs.insert(key.clone(), ty.clone().unwrap_or(Type::Unknown));
            model.external_specs.insert(
                key.clone(),
                (ty, globals.spec_unchecked_inputs.contains(key)),
            );
        }
        let project_set_namespace = globals.project_namespaces.contains("clojure.set");
        let mut initializer_scope = vars::LiteralInitializerScope::new(source);
        let mut opaque_initializer = false;
        for &index in &declarations.literal_vars {
            let form = &forms[index];
            let Kind::List(xs) = &form.without_metadata().kind else {
                unreachable!()
            };
            let initializer = xs.get(2..).and_then(vars::def_initializer);
            let inverted = initializer
                .and_then(|value| type_forms::inverted_literal(declarations, forms, source, value));
            if inverted.is_some() {
                model.set_namespace_in_project = Some(project_set_namespace);
            }
            let captured = inverted
                .filter(|_| !project_set_namespace)
                .map(CapturedInitializer::from_form)
                .or_else(|| {
                    initializer.and_then(|value| {
                        model
                            .capture_literal_root(declarations, forms, source, index, value)
                            .or_else(|| {
                                initializer_scope
                                    .fold(
                                        declarations,
                                        forms,
                                        globals,
                                        index,
                                        value,
                                        &model.captured_initializers,
                                    )
                                    .filter(|_| !model.check_project_core(globals))
                            })
                    })
                });
            model.read_def(xs, form.span, source, captured.as_ref(), globals);
            // Load-time initialization can call code that interns additional Vars.
            opaque_initializer = opaque_initializer
                || initializer.is_some_and(|form| {
                    super::has_evaluated_metadata(form)
                        || (captured.is_none() && !vars::literal(form))
                });
            if let Some(captured) = captured {
                model.captured_initializers.insert(index, captured);
            }
        }
        model.withhold_literals_before_last_tag(declarations, forms);
        model.initializer_macros = initializer_scope.macro_inputs;
        model.initializer_class_names = initializer_scope.class_inputs;
        model.set_namespace_in_project = model
            .set_namespace_in_project
            .or(initializer_scope.set_namespace_in_project);
        for &index in &declarations.unchecked_vars {
            let Kind::List(xs) = &forms[index].without_metadata().kind else {
                unreachable!()
            };
            model.read_unchecked_var(
                &xs[1],
                xs[0].symbol().unwrap(),
                declarations.private_var(xs),
            );
        }
        let mut fdefs = model.collect_fdefs(&declarations.fdefs, forms);
        let mut guards = BTreeMap::new();
        let opaque_definition = opaque_initializer || declarations.has_opaque_var_metadata(forms);
        let source_arities_known = !opaque_definition && declarations.has_known_var_surface(forms);
        for &index in &declarations.functions {
            model.read_function(
                (declarations.function_source(index), forms),
                source,
                globals,
                &mut fdefs,
                &mut guards,
                source_arities_known,
            );
        }
        for items in declarations.scoped_functions(forms) {
            model.read_unchecked_function(&items[1], "defn", declarations.private_var(items));
        }
        for (symbol, span, arity) in declarations.type_factories(forms) {
            model.read_factory(&symbol, span, source_arities_known.then_some(arity));
        }
        for var in declarations.protocol_vars(forms) {
            if var.method {
                model.read_unchecked_function(var.name, "defprotocol", var.private());
            } else {
                model.read_unchecked_var(var.name, "defprotocol", var.private());
            }
        }
        for (local, var) in &declarations.forward_vars {
            let qualified = format!("{}/{local}", model.namespace());
            if !model.literals.contains_key(&qualified)
                && !model.functions.contains_key(&qualified)
                && !model.unchecked_functions.contains_key(&qualified)
            {
                model.names.refers.remove(local);
                model
                    .literals
                    .insert(qualified.clone(), LiteralVar::unknown(var.private));
                model.diagnostics.push(Diagnostic::analysis_gap(
                    "PNX9001",
                    var.span,
                    format!("Declared var {qualified} has no source definition"),
                ));
            }
        }
        for (name, parts) in fdefs {
            if model.read_multimethod_contract(declarations, &name, parts, source, globals) {
                continue;
            }
            if model.unchecked_functions.contains_key(&name) {
                model.diagnostics.push(Diagnostic::analysis_gap(
                    "PNX9001",
                    parts[1].span,
                    format!("s/fdef body checks are unavailable for source function {name}"),
                ));
            } else {
                model.error(
                    "PNX9001",
                    parts[1].span,
                    format!("s/fdef has no supported function definition for {name}"),
                );
            }
        }
        if model
            .functions
            .values()
            .any(|function| function.ordinary_dotted)
            && declarations.has_var_metadata_writes(forms)
        {
            for function in model.functions.values_mut() {
                function.ordinary_dotted = false;
            }
        }
        model.verify_guards(guards, declarations, forms, globals);
        model.check_data_functions(source, globals);
        model.collect_helpers();
        // With an otherwise known surface, absent arity proof means a possible Var mutation.
        let var_inventory = if opaque_definition
            || declarations.has_unmodeled_var_exports()
            || (!declarations.opaque_var_surface && !source_arities_known)
        {
            VarInventory::Incomplete
        } else if source_arities_known {
            VarInventory::Complete(
                declarations
                    .top_level_names(forms)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            )
        } else {
            VarInventory::DeclaredOnly
        };
        model.freeze_interfaces(specs, declarations.dependencies.clone(), var_inventory);
        model
    }

    fn error(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(code, span, message));
    }

    pub(super) fn namespace(&self) -> &str {
        &self.names.namespace
    }

    pub(super) fn can_use_standard_core(&self, name: &str, globals: &Globals) -> bool {
        !globals.project_namespaces.contains("clojure.core")
            // Loaded referrals have already been applied in namespace order.
            && (name != "/" && name.contains('/')
                || self.external_refers.values().all(Option::is_some))
    }

    pub(super) fn known_standard_core_function(
        &self,
        spelling: &str,
        resolved: &str,
        globals: &Globals,
    ) -> bool {
        resolved
            .strip_prefix("clojure.core/")
            .is_some_and(crate::core_names::known_core_function)
            && self.can_use_standard_core(spelling, globals)
    }

    pub(super) fn standard_core_binding(
        &self,
        spelling: &str,
        resolved: &str,
        globals: &Globals,
    ) -> Option<crate::core_names::CoreBinding> {
        let name = resolved.strip_prefix("clojure.core/")?;
        if !self.can_use_standard_core(spelling, globals) {
            return None;
        }
        crate::core_names::core_binding(name)
    }

    pub(super) fn resolve(&self, name: &str) -> String {
        self.names.resolve(name)
    }

    pub(super) fn class_name<'a>(&'a self, symbol: &'a str) -> &'a str {
        self.names
            .imports
            .get(symbol)
            .map_or(symbol, String::as_str)
    }

    pub(super) fn key(&self, key: &str) -> String {
        self.names.key(key)
    }

    pub(super) fn validation_key(&self, keyword: &str) -> Option<String> {
        self.names.validation_key(keyword)
    }

    pub(super) fn function_name(&self, name: &str) -> String {
        if name.contains('/') && name != "/" {
            self.resolve(name)
        } else {
            self.names
                .refers
                .get(name)
                .cloned()
                .unwrap_or_else(|| format!("{}/{name}", self.names.namespace))
        }
    }

    pub(super) fn has_unavailable_wildcard_scope(&self, name: &str) -> bool {
        self.unavailable_all_refers && (name == "/" || !name.contains('/'))
    }

    pub(super) fn has_unavailable_value_scope(&self, name: &str, globals: &Globals) -> bool {
        if self.has_unavailable_wildcard_scope(name) {
            return true;
        }
        if !(name != "/" && name.contains('/') || self.names.refers.contains_key(name)) {
            return false;
        }
        let (namespace, _) = self.names.resolve_parts(name);
        namespace != self.namespace()
            && (!globals.project_namespaces.contains(namespace)
                || globals.has_incomplete_var_inventory(namespace))
    }

    fn check_project_core(&mut self, globals: &Globals) -> bool {
        let present = globals.project_namespaces.contains("clojure.core");
        self.core_in_project = Some(present);
        present
    }

    pub fn refresh_external_inputs(&mut self, globals: &Globals) -> bool {
        globals.root_writes.get(self.namespace()).map_or_else(
            || self.root_writes.is_empty(),
            |writes| *writes == self.root_writes,
        ) && self
            .core_in_project
            .is_none_or(|present| present == globals.project_namespaces.contains("clojure.core"))
            && self
                .set_namespace_in_project
                .is_none_or(|present| present == globals.project_namespaces.contains("clojure.set"))
            && self.initializer_macros.iter().all(|(qualified, previous)| {
                globals.may_expand_initializer_macro(qualified) == *previous
            })
            && self
                .initializer_class_names
                .iter()
                .all(|(name, previous)| globals.has_competing_class_namespace(name) == *previous)
            && self
                .external_specs
                .iter()
                .all(|(key, (previous, unchecked))| {
                    globals.specs.get(key) == previous.as_ref()
                        && globals.spec_unchecked_inputs.contains(key) == *unchecked
                })
            && self
                .external_refers
                .iter_mut()
                .all(|(namespace, previous)| {
                    match (previous.as_ref(), globals.exports.get(namespace)) {
                        (Some(old), Some(current)) if Arc::ptr_eq(old, current) => true,
                        (Some(old), Some(current))
                            if old.var_inventory.is_incomplete()
                                == current.var_inventory.is_incomplete()
                                && old.public_var_names().eq(current.public_var_names()) =>
                        {
                            *previous = Some(current.clone());
                            true
                        }
                        (None, None) => true,
                        _ => false,
                    }
                })
    }

    pub(super) fn local_function(&self, name: &str) -> Option<&Function> {
        self.functions.get(&self.function_name(name))
    }

    fn collect_helpers(&mut self) {
        if !self.functions.values().any(Function::inferred) {
            return;
        }
        fn visit(forms: &[Form], model: &Model, names: &mut BTreeSet<String>) {
            for form in forms {
                match &form.kind {
                    Kind::Symbol(name) => {
                        if let Some(function) = model.local_function(name).filter(|f| f.inferred())
                        {
                            names.insert(function.name.clone());
                        }
                    }
                    Kind::List(xs) if xs.first().and_then(Form::symbol) == Some("quote") => {}
                    Kind::List(xs) | Kind::Vector(xs) | Kind::Map(xs) | Kind::Set(xs) => {
                        visit(xs, model, names)
                    }
                    Kind::AnonymousFunction(function) => visit(&function.body, model, names),
                    Kind::Metadata(_, target) => {
                        for condition in [":pre", ":post"] {
                            if let Some(forms) = form.metadata(condition) {
                                visit(std::slice::from_ref(forms), model, names);
                            }
                        }
                        visit(std::slice::from_ref(target), model, names);
                    }
                    _ => {}
                }
            }
        }
        fn visit_defaults(form: &Form, model: &Model, names: &mut BTreeSet<String>) {
            match &form.without_metadata().kind {
                Kind::Map(entries) => {
                    for pair in entries.as_chunks::<2>().0 {
                        match pair[0].keyword() {
                            Some(":or") => {
                                if let Some(defaults) = pair[1].map() {
                                    for default in defaults.as_chunks::<2>().0 {
                                        visit(std::slice::from_ref(&default[1]), model, names);
                                    }
                                }
                            }
                            Some(":req!") => visit_defaults(&pair[1], model, names),
                            _ => visit_defaults(&pair[0], model, names),
                        }
                    }
                }
                Kind::Vector(forms) => {
                    for form in forms {
                        visit_defaults(form, model, names);
                    }
                }
                _ => {}
            }
        }
        for function in self.functions.values() {
            let mut names = BTreeSet::new();
            for &arity in function.arities.keys() {
                let clause = self.source_clause(function, arity);
                for param in clause.parameters() {
                    visit_defaults(param, self, &mut names);
                }
                visit(clause.conditions.pre, self, &mut names);
                if let Some(postconditions) = clause.conditions.post {
                    visit(postconditions, self, &mut names);
                }
                visit(clause.body, self, &mut names);
            }
            self.helper_dependencies
                .insert(function.name.clone(), names);
        }
        let reachable: BTreeSet<_> = self
            .functions
            .values()
            .filter(|f| !f.inferred())
            .flat_map(|f| self.helpers(f))
            .collect();
        let unused: Vec<_> = self
            .functions
            .values()
            .filter(|f| {
                f.inferred()
                    && !self.checked_data_functions.contains(&f.name)
                    && !reachable.contains(f.name.as_str())
            })
            .map(|f| (f.span, f.name.clone()))
            .collect();
        for (span, name) in unused {
            self.error("PNX9002", span, format!(
                "Cannot infer unused private function {name}; add a :t/sig annotation or a direct typed caller"
            ));
        }
    }

    pub fn diagnose_unchecked_helpers<'a>(
        &self,
        checked: impl Iterator<Item = &'a str>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if self.helper_dependencies.is_empty() {
            return;
        }
        let checked: BTreeSet<_> = checked.collect();
        let already_reported: BTreeSet<_> = self
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "PNX9002")
            .map(|diagnostic| diagnostic.span.start.offset)
            .collect();
        for function in self
            .functions
            .values()
            .filter(|function| function.inferred())
        {
            if !checked.contains(function.name.as_str())
                && !self.checked_data_functions.contains(&function.name)
                && !already_reported.contains(&function.span.start.offset)
            {
                diagnostics.push(Diagnostic::new(
                    "PNX9002",
                    function.span,
                    format!(
                        "Cannot infer private function {} without a reachable typed call; add a :t/sig annotation or a direct typed caller",
                        function.name
                    ),
                ));
            }
        }
    }

    fn helpers(&self, function: &Function) -> BTreeSet<&str> {
        let mut pending: Vec<_> = self
            .helper_dependencies
            .get(&function.name)
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        let mut names = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if names.insert(name) {
                pending.extend(
                    self.helper_dependencies
                        .get(name)
                        .into_iter()
                        .flatten()
                        .map(String::as_str),
                );
            }
        }
        names
    }

    pub fn function_ranges(&self, function: &Function) -> Vec<Span> {
        std::iter::once(function.span)
            .chain(
                self.helpers(function)
                    .into_iter()
                    .map(|name| self.functions[name].span),
            )
            .collect()
    }

    pub(super) fn runtime_alias_gap(&self, symbol: &str) -> Option<&str> {
        self.names.runtime_alias_gap(symbol)
    }

    pub(super) fn missing_var_operand(
        &self,
        name: &str,
        span: Span,
        globals: &Globals,
    ) -> Option<Diagnostic> {
        if self.runtime_alias_gap(name).is_some() {
            return None;
        }
        let (namespace, local) = if name != "/" && name.contains('/') {
            self.names.resolve_parts(name)
        } else {
            // The compiler looks in the namespace mappings, even inside a let.
            // Its ns and in-ns Vars remain available after a core exclusion.
            if matches!(name, "ns" | "in-ns")
                || self.unavailable_all_refers
                || self.names.refers.contains_key(name)
                || (self.names.core_visible(name)
                    && (globals.project_namespaces.contains("clojure.core")
                        || crate::core_names::core_binding(name).is_some()))
            {
                return None;
            }
            (self.namespace(), name)
        };
        if namespace == "user" && namespace == self.namespace() {
            // The default runtime namespace can have startup mappings.
            return None;
        }
        let source = globals.exports.get(namespace)?;
        let externs = globals
            .externs
            .as_ref()
            .and_then(|table| table.namespaces.get(namespace));
        proves_missing_var(
            Some(source),
            externs.map(Arc::as_ref),
            &format!("{namespace}/{local}"),
        )
        .then(|| Diagnostic::new("PNX2304", span, format!("Could not resolve Var {name}")))
    }

    pub(super) fn declared_value<'a>(
        &'a self,
        name: &str,
        globals: &'a Globals,
        offset: usize,
    ) -> Option<DeclaredValue<'a>> {
        if self.names.runtime_alias_gap(name).is_some() {
            return None;
        }
        let local_macro = self.names.local_source_macros(name);
        if let Some((local, declarations)) = local_macro
            && declarations.iter().any(|entry| entry.active_at(offset))
        {
            let qualified = format!("{}/{local}", self.namespace());
            return Some(source_macro_value(globals.extern_declaration(&qualified)));
        }
        let qualified = self.function_name(name);
        if let Some(value) = self.literals.get(&qualified) {
            return Some(DeclaredValue::Literal(value));
        }
        if let Some(function) = self.functions.get(&qualified) {
            return function.contract.as_ref().map(DeclaredValue::Function);
        }
        let local = self.unchecked_functions.get(&qualified);
        if let Some(function) = local.filter(|function| function.private) {
            return Some(DeclaredValue::UncheckedFunction(function));
        }
        let global_value = |candidate: &str| {
            // The published table includes this file's later macro declarations.
            if local_macro.is_some_and(|(_, declarations)| {
                declarations
                    .last()
                    .is_some_and(|entry| entry.replaced.is_none())
            }) && candidate
                .split_once('/')
                .is_some_and(|(ns, _)| ns == self.namespace())
            {
                None
            } else {
                globals.declared_value(candidate)
            }
        };
        global_value(&qualified)
            .or_else(|| local.map(DeclaredValue::UncheckedFunction))
            .or_else(|| global_value(&self.resolve(name)))
    }

    pub(super) fn local_function_at(&self, name: &str, offset: usize) -> Option<&Function> {
        if self
            .names
            .local_source_macros(name)
            .is_some_and(|(_, declarations)| {
                declarations.iter().any(|entry| entry.active_at(offset))
            })
        {
            None
        } else {
            self.local_function(name)
        }
    }

    pub(super) fn ordinary_dotted_function_at<'a>(
        &'a self,
        name: &str,
        globals: &'a Globals,
        offset: usize,
    ) -> Option<&'a str> {
        if !bare_dotted_name(name) {
            return None;
        }
        if let Some(function) = self.local_function_at(name, offset) {
            return function.ordinary_dotted.then_some(function.name.as_str());
        }
        if self.names.runtime_alias_gap(name).is_some() {
            return None;
        }
        let referred = self.names.refers.get(name)?;
        let (namespace, _) = referred.split_once('/')?;
        globals
            .exports
            .get(namespace)?
            .ordinary_dotted_functions
            .get(referred)
            .map(String::as_str)
    }

    pub(super) fn qualified_var_spelling<'a>(&self, qualified: &'a str) -> Option<Cow<'a, str>> {
        let (namespace, name) = qualified.split_once('/')?;
        if self.names.resolve_parts(qualified) == (namespace, name)
            && self.names.runtime_alias_gap(qualified).is_none()
        {
            return Some(Cow::Borrowed(qualified));
        }
        self.names.aliases.iter().find_map(|(alias, target)| {
            if target != namespace {
                return None;
            }
            let spelling = format!("{alias}/{name}");
            self.names
                .runtime_alias_gap(&spelling)
                .is_none()
                .then_some(Cow::Owned(spelling))
        })
    }

    pub(super) fn dotted_class_use_message(
        name: &str,
        qualified: &str,
        spelling: Option<&str>,
    ) -> String {
        let hint = spelling.map_or_else(
            || format!("The matching var is {qualified}."),
            |spelling| format!("Use {spelling} to refer to the var."),
        );
        format!("Clojure/JVM resolves unqualified {name} as a class name. {hint}")
    }

    pub(crate) fn dotted_var_fix<'a>(
        &'a self,
        name: &str,
        globals: &'a Globals,
        offset: usize,
        message: &str,
    ) -> Option<Cow<'a, str>> {
        if self.known_java_class(name) {
            return None;
        }
        let qualified = self.ordinary_dotted_function_at(name, globals, offset)?;
        let spelling = self.qualified_var_spelling(qualified)?;
        let expected = Self::dotted_class_use_message(name, qualified, Some(&spelling));
        (message == expected).then_some(spelling)
    }
}
