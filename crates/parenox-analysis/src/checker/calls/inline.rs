//! Contextual inference for inline and let-bound functions and callable wrappers.
mod letfn;

use std::{collections::HashSet, ptr, sync::Arc};

use super::super::{
    Checker, Env, Value,
    env::{CapturedFn, CapturedInline},
    function_forms::{
        MAX_FIXED_PARAMETERS, fn_star_clauses, function_clauses, letfn_definition, local_fn_name,
        pack_arguments,
    },
    maps::{MapLiteral, literal_map_in_source_order},
    metadata::MetadataTarget,
    nil_part,
    queries::InferredCall,
};
use super::{Callable, ContextualArgument, PreparedCallable, call_returns, combinators::Combiner};
use crate::reader::{AnonymousFunction, Form, Kind, Span};
use crate::types::{Arity, MAX_CALLBACK_MAP_DEPTH, MapType, Signature, Type};
use crate::{ContractOrigin, Diagnostic};

const MAX_CALLBACK_DEPTH: usize = 16;
const MAX_LOCAL_INLINE_STEPS: usize = 128;

pub(super) fn contextual_map_type(expected: &Type) -> Option<&MapType> {
    let Type::Map(map) = expected.nilable_member().unwrap_or(expected) else {
        return None;
    };
    Some(map)
}

fn conditional_arms(form: &Form) -> Option<(&Form, &Form, &Form)> {
    let Kind::List(parts) = &form.without_metadata().kind else {
        return None;
    };
    let [head, test, yes, no] = parts.as_slice() else {
        return None;
    };
    (head.symbol() == Some("if")).then_some((test, yes, no))
}

#[derive(Clone, Copy)]
pub(super) enum InlineFunction<'a> {
    Function { parts: &'a [Form], raw: bool },
    Literal(&'a AnonymousFunction),
}

impl<'a> InlineFunction<'a> {
    fn name(self) -> Option<&'a str> {
        match self {
            Self::Function { parts, raw } => Some(local_fn_name(parts.first()?.symbol()?, raw)),
            Self::Literal(_) => None,
        }
    }

    fn bind_name(self, function: &CapturedInline, env: &mut Env) {
        if let Some(name) = self.name() {
            env.insert(name.into(), Value::new(Type::Unknown));
            env.mark_inline(name, function.clone());
        }
    }
}

impl CapturedInline {
    fn single(form: &Form, scope: &Env) -> Self {
        Self::Single(Arc::new(CapturedFn {
            form: form.clone(),
            scope: scope.clone(),
        }))
    }

    pub(super) fn syntax(&self) -> (&Form, InlineFunction<'_>) {
        match self {
            Self::Single(function) => {
                // Capture construction resolved the head in the definition scope.
                let form = &function.form;
                let function = match &form.without_metadata().kind {
                    Kind::List(parts) => InlineFunction::Function {
                        parts: &parts[1..],
                        raw: parts[0].symbol() == Some("fn*"),
                    },
                    Kind::AnonymousFunction(function) => InlineFunction::Literal(function),
                    _ => unreachable!("captured fn has no function syntax"),
                };
                (form, function)
            }
            Self::Group { group, index } => {
                let form = &group.definitions[*index].1;
                let parts = letfn_definition(form).expect("validated letfn definition");
                (form, InlineFunction::Function { parts, raw: false })
            }
        }
    }

    fn refine_scope(&self, caller: &Env) -> Env {
        match self {
            Self::Single(function) => function.scope.refine_capture(caller),
            Self::Group { group, .. } => {
                let mut scope = group.scope.refine_capture(caller);
                group.bind_to(&mut scope);
                scope
            }
        }
    }
}

enum InlineSyntax<'a> {
    Function(InlineFunction<'a>),
    Fnil(&'a [Form]),
    Partial(&'a [Form]),
    Combiner(Combiner, &'a [Form]),
}

#[derive(Clone, Copy)]
struct InlineSite<'a> {
    constructor: Span,
    call: Span,
    invoked: bool,
    callback: Option<CallbackCheck<'a>>,
}

#[derive(Clone, Copy)]
struct CallbackCheck<'a> {
    argument: Span,
    expected_return: Option<&'a Type>,
    origin: Option<&'a ContractOrigin>,
}

#[derive(Clone, Copy)]
pub(super) enum CallbackReturn {
    Check,
    Infer,
}

struct CallbackBranch<'a> {
    form: &'a Form,
    scope: Env,
    returns: bool,
}

pub(super) struct PreparedChoice<'a> {
    callbacks: Vec<CallbackBranch<'a>>,
    eager: Option<Value>,
}

pub(super) struct ContextualCallback {
    pub value: Value,
    pub rejected_return: Option<Type>,
}

impl ContextualCallback {
    pub(super) fn from_value(value: Value) -> Self {
        Self {
            value,
            rejected_return: None,
        }
    }

    fn unchecked() -> Self {
        Self::from_value(Value::new(Type::Unknown))
    }
}

pub(super) struct Fnil<'form, 'model> {
    pub(super) target: PreparedCallable<'form, 'model>,
    defaults: Vec<(Value, Span)>,
    pub(super) returns: bool,
}

pub(super) struct Partial<'form, 'model> {
    pub(super) target: PreparedCallable<'form, 'model>,
    pub(super) prefix: Vec<(Value, Span)>,
    pub(super) returns: bool,
}

impl<'model> Checker<'model> {
    pub(in crate::checker) fn withhold_captured_facts<'a>(
        &mut self,
        functions: impl IntoIterator<Item = &'a CapturedInline>,
    ) {
        if self.type_query.is_none() || !self.path_reachable {
            return;
        }
        // An unobserved invocation may call any peer or captured closure,
        // including through a branch that the checked calls never reached.
        let mut pending: Vec<_> = functions.into_iter().collect();
        let mut visited = HashSet::new();
        while let Some(function) = pending.pop() {
            let scope = match function {
                CapturedInline::Single(function) => &function.scope,
                CapturedInline::Group { group, .. } => &group.scope,
            };
            if !visited.insert(ptr::from_ref(scope)) {
                continue;
            }
            match function {
                CapturedInline::Single(function) => self.withhold_type_facts(function.form.span),
                CapturedInline::Group { group, .. } => {
                    for (_, form) in &group.definitions {
                        self.withhold_type_facts(form.span);
                    }
                }
            }
            if self.type_query.is_none() {
                break;
            }
            pending.extend(scope.inline_functions());
        }
    }

    pub(super) fn local_call(
        &self,
        captured: &CapturedInline,
        arguments: &[(Value, Span)],
    ) -> Option<InferredCall> {
        if arguments
            .iter()
            .any(|(value, _)| value.known.contains_unknown())
        {
            return None;
        }
        let (definition, function) = captured.syntax();
        let count = arguments.len();
        let labels = match function {
            InlineFunction::Function { parts, raw } => {
                let params = &parts[usize::from(function.name().is_some())..];
                let clauses = if raw {
                    fn_star_clauses(params, definition.span)
                } else {
                    function_clauses(params, definition.span)
                }
                .ok()?;
                clauses
                    .get(&Arity::Fixed(count))?
                    .parameters()
                    .map(|parameter| {
                        self.source
                            .get(parameter.span.start.offset..parameter.span.end.offset)
                            .map(str::to_owned)
                    })
                    .collect::<Option<Vec<_>>>()?
            }
            InlineFunction::Literal(function)
                if !function.variadic && count == function.required =>
            {
                (1..=count).map(|index| format!("%{index}")).collect()
            }
            InlineFunction::Literal(_) => return None,
        };
        Some(InferredCall {
            definition: definition.span,
            labels: Some(labels),
            args: arguments
                .iter()
                .map(|(value, _)| value.known.clone())
                .collect(),
        })
    }

    pub(super) fn prepare_inline<'form>(
        &mut self,
        form: &'form Form,
        env: &Env,
    ) -> Option<PreparedCallable<'form, 'model>> {
        let kind = if let Some(function) = form.symbol().and_then(|name| env.inline_function(name))
        {
            Callable::Captured {
                function: function.clone(),
                compile_when_uninvoked: false,
            }
        } else {
            match self.inline_syntax(form, env)? {
                InlineSyntax::Function(function) => {
                    if let Some(value) = self
                        .literal_metadata_outcome(
                            form,
                            env,
                            env.reachable(),
                            MetadataTarget::Function,
                        )
                        .filter(Value::stops_evaluation)
                    {
                        let closure = CapturedInline::single(form, env);
                        self.call_captured(&closure, Vec::new(), form.span, env, false, true);
                        Callable::Value(value)
                    } else if function.name().is_some() {
                        Callable::Captured {
                            function: CapturedInline::single(form, env),
                            compile_when_uninvoked: true,
                        }
                    } else {
                        Callable::Inline(function)
                    }
                }
                InlineSyntax::Fnil(parts) => self.prepare_fnil(parts, form.span, env),
                InlineSyntax::Partial(parts) => {
                    return Some(self.prepare_partial(parts, form.span, env));
                }
                InlineSyntax::Combiner(combiner, parts) => {
                    return Some(self.prepare_combiner(combiner, parts, form.span, env));
                }
            }
        };
        Some(PreparedCallable::new(kind, form.span))
    }

    pub(in crate::checker) fn bind_inline(
        &mut self,
        binder: &Form,
        init: &Form,
        env: &mut Env,
    ) -> bool {
        let Some(name) = binder.symbol() else {
            return false;
        };
        let (function, value) =
            if let Some(function) = init.symbol().and_then(|name| env.inline_function(name)) {
                (Some(function.clone()), Value::new(Type::Unknown))
            } else {
                let Some(InlineSyntax::Function(_)) = self.inline_syntax(init, env) else {
                    return false;
                };
                let value = self
                    .literal_metadata_outcome(init, env, env.reachable(), MetadataTarget::Function)
                    .unwrap_or_else(|| Value::new(Type::Unknown));
                let function = self.local_inline_step(init.span).then(|| {
                    let function = CapturedInline::single(init, env);
                    // The compiler checks every clause when creating a function.
                    // Its body contributes no runtime effects until it is called.
                    self.call_captured(&function, Vec::new(), init.span, env, false, true);
                    function
                });
                (function, value)
            };
        let returns = !value.stops_evaluation();
        self.bind(binder, value, env);
        if returns && let Some(function) = function {
            env.mark_inline(name, function);
        }
        true
    }

    pub(super) fn call_captured(
        &mut self,
        function: &CapturedInline,
        args: Vec<(Value, Span)>,
        call: Span,
        caller: &Env,
        invoked: bool,
        compile_when_uninvoked: bool,
    ) -> Value {
        let invoked = invoked && self.path_reachable;
        if !invoked && !compile_when_uninvoked {
            return Value::new(Type::Never);
        }
        if invoked && !self.local_inline_step(call) {
            self.may_throw();
            return Value::new(Type::Unknown);
        }
        if invoked && self.callback_depth >= MAX_CALLBACK_DEPTH {
            self.type_query = None;
            return self.unsupported(
                call,
                format!("Local fn nesting limit ({MAX_CALLBACK_DEPTH}) exceeded"),
            );
        }
        let (constructor, inline) = function.syntax();
        // Binding IDs retain refinement only for locals still in the same
        // lexical scope. A later binding with the same name is a new value.
        let mut scope = function.refine_scope(caller);
        inline.bind_name(function, &mut scope);
        self.callback_depth += usize::from(invoked);
        let result = self.call_inline(inline, args, constructor.span, call, &scope, invoked);
        self.callback_depth -= usize::from(invoked);
        result
    }

    fn local_inline_step(&mut self, span: Span) -> bool {
        if self.local_inline_steps >= MAX_LOCAL_INLINE_STEPS {
            if self.local_inline_steps == MAX_LOCAL_INLINE_STEPS {
                self.error(
                    "PNX9001",
                    span,
                    format!("Local fn analysis limit ({MAX_LOCAL_INLINE_STEPS} steps) exceeded"),
                );
                self.local_inline_steps += 1;
            }
            self.type_query = None;
            false
        } else {
            self.local_inline_steps += 1;
            true
        }
    }

    pub(super) fn fn_value(&mut self, parts: &[Form], span: Span, env: &Env, raw: bool) -> Value {
        self.literal_fn_value(
            InlineFunction::Function { parts, raw },
            span,
            env,
            if raw { "fn*" } else { "fn" },
        )
    }

    pub(in crate::checker) fn anonymous_fn_value(
        &mut self,
        function: &AnonymousFunction,
        span: Span,
        env: &Env,
    ) -> Value {
        self.literal_fn_value(InlineFunction::Literal(function), span, env, "#()")
    }

    fn literal_fn_value(
        &mut self,
        function: InlineFunction<'_>,
        span: Span,
        env: &Env,
        name: &str,
    ) -> Value {
        if self.path_reachable {
            self.withhold_captured_facts(env.inline_functions());
        }
        let start = self.diagnostics.len();
        let value = self.check_uninvoked_literal(function, span, env);
        if !self.path_reachable {
            return value;
        }
        if self.diagnostics.len() != start {
            return Value::new(Type::Unknown);
        }
        self.unsupported(
            span,
            format!("{name} is currently supported in direct calls and inline callbacks"),
        )
    }

    fn check_uninvoked_literal(
        &mut self,
        function: InlineFunction<'_>,
        span: Span,
        env: &Env,
    ) -> Value {
        if self.callback_depth >= MAX_CALLBACK_DEPTH {
            self.type_query = None;
            return self.unchecked_form(span, "Local fn nesting limit exceeded");
        }
        let named = function.name().map(|name| {
            let mut scope = env.clone();
            scope.insert(name.into(), Value::new(Type::Unknown));
            scope
        });
        self.callback_depth += 1;
        let value = self.inspect_path(false, |checker| {
            checker.call_inline(
                function,
                Vec::new(),
                span,
                span,
                named.as_ref().unwrap_or(env),
                false,
            )
        });
        self.callback_depth -= 1;
        value
    }

    fn inline_syntax<'form>(&self, form: &'form Form, env: &Env) -> Option<InlineSyntax<'form>> {
        let forms = match &form.without_metadata().kind {
            Kind::List(forms) => forms,
            Kind::AnonymousFunction(function) => {
                return Some(InlineSyntax::Function(InlineFunction::Literal(function)));
            }
            _ => return None,
        };
        let (head, parts) = forms.split_first()?;
        let name = head.symbol()?;
        if name == "fn*" {
            return Some(InlineSyntax::Function(InlineFunction::Function {
                parts,
                raw: true,
            }));
        }
        if env.contains_key(name)
            || self.model.has_unavailable_wildcard_scope(name)
            || self.model.local_function(name).is_some()
            || self
                .model
                .declared_value(name, self.globals, head.span.start.offset)
                .is_some()
        {
            return None;
        }
        match self.model.resolve(name).as_str() {
            "clojure.core/fn" => Some(InlineSyntax::Function(InlineFunction::Function {
                parts,
                raw: false,
            })),
            "clojure.core/fnil" => Some(InlineSyntax::Fnil(parts)),
            "clojure.core/partial" if self.model.can_use_standard_core(name, self.globals) => {
                Some(InlineSyntax::Partial(parts))
            }
            "clojure.core/comp" if self.model.can_use_standard_core(name, self.globals) => {
                Some(InlineSyntax::Combiner(Combiner::Comp, parts))
            }
            "clojure.core/juxt" if self.model.can_use_standard_core(name, self.globals) => {
                Some(InlineSyntax::Combiner(Combiner::Juxt, parts))
            }
            _ => None,
        }
    }

    pub(super) fn inline_callback_literal(&self, form: &Form, env: &Env) -> bool {
        matches!(
            self.inline_syntax(form, env),
            Some(InlineSyntax::Function(_))
        )
    }

    pub(super) fn with_callback_choice(
        &mut self,
        callback: &Form,
        env: &Env,
        mut check: impl FnMut(&mut Self, &Form, &Env) -> Value,
    ) -> Value {
        if let Some((test, yes, no)) = conditional_arms(callback)
            && self.inline_callback_literal(yes, env)
            && self.inline_callback_literal(no, env)
        {
            let start = self.diagnostics.len();
            // Choose the callback before evaluating later arguments, and
            // hold that choice through the entire collection operation.
            let [left, right] = self.check_if_paths(test, env, true, |checker, scope, truth| {
                check(checker, if truth { yes } else { no }, scope)
            });
            self.dedup_diagnostics(start);
            left.join(right)
        } else {
            check(self, callback, env)
        }
    }

    fn contextual_choice<'a>(
        &self,
        form: &'a Form,
        env: &Env,
    ) -> Option<(&'a Form, [(&'a Form, bool); 2])> {
        let (test, yes, no) = conditional_arms(form)?;
        let yes_is_callback = self.contextual_callback_form(yes, env);
        let no_is_callback = self.contextual_callback_form(no, env);
        (yes_is_callback || no_is_callback)
            .then_some((test, [(yes, yes_is_callback), (no, no_is_callback)]))
    }

    #[inline(never)]
    fn prepare_callback_choice<'a>(
        &mut self,
        form: &'a Form,
        env: &Env,
        reachable: bool,
    ) -> Option<(Box<PreparedChoice<'a>>, Value)> {
        let (test, choices) = self.contextual_choice(form, env)?;
        let mut callbacks = Vec::with_capacity(2);
        let branches = self.inspect_path(reachable, |checker| {
            checker.check_if_paths(test, env, true, |checker, scope, truth| {
                let (form, is_callback) = choices[usize::from(!truth)];
                if !is_callback {
                    return checker.infer(form, scope);
                }
                let value =
                    checker.contextual_callback_argument(form, scope, checker.path_reachable);
                callbacks.push(CallbackBranch {
                    form,
                    scope: scope.clone(),
                    returns: !value.stops_evaluation(),
                });
                value
            })
        });
        let returns = reachable && branches.iter().any(|branch| !branch.stops_evaluation());
        let eager = branches
            .into_iter()
            .zip(choices)
            .find_map(|(branch, (_, is_callback))| (!is_callback).then_some(branch));
        let value = Value::new(if returns { Type::Unknown } else { Type::Never });
        Some((Box::new(PreparedChoice { callbacks, eager }), value))
    }

    pub(super) fn prepare_contextual_argument<'a>(
        &mut self,
        form: &'a Form,
        expected: &'a Type,
        env: &Env,
        reachable: bool,
        map_key: Option<&'a Type>,
    ) -> Option<(ContextualArgument<'a>, Value)> {
        expected.contextual_signature()?;
        let (choice, value) = if self.contextual_callback_form(form, env) {
            // Creating or reading a closure does not execute its body.
            (
                None,
                self.contextual_callback_argument(form, env, reachable),
            )
        } else {
            let (choice, value) = self.prepare_callback_choice(form, env, reachable)?;
            (Some(choice), value)
        };
        Some((
            ContextualArgument {
                callback: form,
                expected,
                choice,
                map_key,
                map_ancestors: Box::new([]),
            },
            value,
        ))
    }

    pub(super) fn check_callback_choice(
        &mut self,
        choice: PreparedChoice<'_>,
        expected: &Type,
        call: Span,
        origin: Option<&ContractOrigin>,
        reachable: bool,
        mode: CallbackReturn,
    ) -> ContextualCallback {
        let mut returns = matches!(mode, CallbackReturn::Infer).then(Vec::new);
        let mut rejected = false;
        let mut observe_return = |value: &Value, rejected_return: Option<Type>| {
            let Some(types) = &mut returns else { return };
            rejected |= rejected_return.is_some();
            let ty = rejected_return.or_else(|| {
                let (_, expected) = expected.contextual_signature()?;
                let known = value.known.signature_at(expected.arity())?;
                (known.parameters.is_none() && !known.nil_preserving).then(|| known.ret.clone())
            });
            if let Some(ty) = ty {
                types.push(ty);
            } else {
                returns = None;
            }
        };
        if reachable && let Some(value) = choice.eager.as_ref().filter(|v| !v.stops_evaluation()) {
            observe_return(value, None);
        }
        let start = self.diagnostics.len();
        let callbacks = choice.callbacks.into_iter().map(|branch| {
            let active = reachable && branch.returns;
            self.check_path(active, |checker| {
                let callback = checker
                    .contextual_callback(branch.form, expected, call, origin, &branch.scope, mode)
                    .unwrap_or_else(|| {
                        ContextualCallback::from_value(checker.infer(branch.form, &branch.scope))
                    });
                if active {
                    observe_return(&callback.value, callback.rejected_return);
                }
                callback.value
            })
        });
        let value = Value::union_branches(choice.eager.into_iter().chain(callbacks));
        self.dedup_diagnostics(start);
        // Return observations from failed bodies can explain a redundant
        // inference gap. They never enter the callback choice's value.
        let rejected_return = returns.filter(|_| rejected).map(Type::union);
        ContextualCallback {
            value,
            rejected_return,
        }
    }

    pub(super) fn check_callback_value(
        &mut self,
        form: &Form,
        expected: &Type,
        call: Span,
        origin: Option<&ContractOrigin>,
        env: &Env,
    ) -> Value {
        self.contextual_callback(form, expected, call, origin, env, CallbackReturn::Check)
            .map(|callback| callback.value)
            .unwrap_or_else(|| self.infer(form, env))
    }

    pub(super) fn contextual_callback_form(&self, form: &Form, env: &Env) -> bool {
        form.symbol()
            .and_then(|name| env.inline_function(name))
            .is_some()
            || self.inline_callback_literal(form, env)
    }

    pub(super) fn contextual_map<'a>(
        &self,
        form: &'a Form,
        expected: &'a Type,
        env: &Env,
    ) -> Option<(&'a [Form], &'a MapType)> {
        self.contextual_map_at_depth(form, expected, env, MAX_CALLBACK_MAP_DEPTH)
    }

    fn contextual_map_at_depth<'a>(
        &self,
        form: &'a Form,
        expected: &'a Type,
        env: &Env,
        remaining: usize,
    ) -> Option<(&'a [Form], &'a MapType)> {
        let expected = contextual_map_type(expected)?;
        let entries = form.map()?;
        let direct_callbacks = expected.fields.values().any(|field| {
            field
                .value_constraint()
                .and_then(Type::contextual_signature)
                .is_some()
        });
        let nested_maps = || {
            remaining != 0
                && expected.fields.values().any(|field| {
                    field
                        .value_constraint()
                        .and_then(contextual_map_type)
                        .is_some()
                })
        };
        if !direct_callbacks && !nested_maps() {
            return None;
        }
        let pairs = entries.as_chunks::<2>().0;
        let ordered = literal_map_in_source_order(pairs);
        pairs
            .iter()
            .any(|[key, value]| match self.written_map_key(key) {
                Some(key) => expected
                    .fields
                    .get(&key)
                    .and_then(|field| field.value_constraint())
                    .is_some_and(|expected| {
                        self.contextual_callback_field(value, expected, env)
                            || (remaining != 0
                                && self
                                    .contextual_map_at_depth(value, expected, env, remaining - 1)
                                    .is_some())
                    }),
                None if ordered => {
                    // The map evaluator can prove the key before choosing the field contract.
                    (direct_callbacks
                        && (self.contextual_callback_form(value, env)
                            || self.contextual_choice(value, env).is_some()))
                        || (value.map().is_some() && nested_maps())
                }
                None => false,
            })
            .then_some((entries, expected))
    }

    fn contextual_callback_field(&self, callback: &Form, expected: &Type, env: &Env) -> bool {
        expected.contextual_signature().is_some()
            && (self.contextual_callback_form(callback, env)
                || self.contextual_choice(callback, env).is_some())
    }

    pub(super) fn contextual_map_argument<'a>(
        &mut self,
        form: &'a Form,
        expected: &'a Type,
        env: &Env,
        reachable: bool,
        mut defer: impl FnMut(ContextualArgument<'a>, bool),
    ) -> Option<Value> {
        self.contextual_map_argument_at(form, expected, env, reachable, &mut Vec::new(), &mut defer)
    }

    fn contextual_map_argument_at<'a>(
        &mut self,
        form: &'a Form,
        expected: &'a Type,
        env: &Env,
        reachable: bool,
        ancestors: &mut Vec<&'a Type>,
        defer: &mut dyn FnMut(ContextualArgument<'a>, bool),
    ) -> Option<Value> {
        let remaining = MAX_CALLBACK_MAP_DEPTH - ancestors.len();
        let (entries, expected) = self.contextual_map_at_depth(form, expected, env, remaining)?;
        Some(self.check_path(reachable, |checker| {
            let map = checker.map_literal_with(
                entries,
                MapLiteral::Expression(env),
                |checker, key, field, fields_return| {
                    let reachable = reachable && fields_return;
                    if let Some((key, constraint)) =
                        key.and_then(|key| expected.fields.get_key_value(key))
                        && let Some(expected) = constraint.value_constraint()
                    {
                        if let Some((mut callback, value)) = checker.prepare_contextual_argument(
                            field,
                            expected,
                            env,
                            reachable,
                            Some(key),
                        ) {
                            callback.map_ancestors = ancestors.clone().into_boxed_slice();
                            defer(callback, reachable);
                            return value;
                        }
                        if remaining != 0 && contextual_map_type(expected).is_some() {
                            ancestors.push(key);
                            let value = checker.contextual_map_argument_at(
                                field, expected, env, reachable, ancestors, defer,
                            );
                            ancestors.pop();
                            if let Some(value) = value {
                                return value;
                            }
                        }
                    }
                    checker.infer_path(field, env, reachable, false)
                },
            );
            checker
                .literal_metadata_outcome(
                    form,
                    env,
                    reachable && !map.stops_evaluation(),
                    MetadataTarget::Map,
                )
                .unwrap_or(map)
        }))
    }

    pub(super) fn contextual_callback_argument(
        &mut self,
        form: &Form,
        env: &Env,
        reachable: bool,
    ) -> Value {
        if form.symbol().is_none()
            && let Some(value) =
                self.literal_metadata_outcome(form, env, reachable, MetadataTarget::Function)
        {
            return value;
        }
        Value::new(if reachable {
            Type::Unknown
        } else {
            Type::Never
        })
    }

    pub(super) fn contextual_callback(
        &mut self,
        form: &Form,
        expected: &Type,
        call: Span,
        origin: Option<&ContractOrigin>,
        env: &Env,
        mode: CallbackReturn,
    ) -> Option<ContextualCallback> {
        let (expected, signature) = expected.contextual_signature()?;
        let saved = form.symbol().and_then(|name| env.inline_function(name));
        let (constructor, function) = if let Some(closure) = saved {
            closure.syntax()
        } else {
            let InlineSyntax::Function(function) = self.inline_syntax(form, env)? else {
                return None;
            };
            (form, function)
        };
        if saved.is_some() && !self.path_reachable {
            // The saved closure was already compiled at its binding.
            return Some(ContextualCallback::unchecked());
        }
        let named = (saved.is_none() && function.name().is_some())
            .then(|| CapturedInline::single(form, env));
        let scope = saved.or(named.as_ref()).map(|closure| {
            let mut scope = closure.refine_scope(env);
            function.bind_name(closure, &mut scope);
            scope
        });
        if !self.path_reachable {
            self.call_inline(
                function,
                Vec::new(),
                constructor.span,
                call,
                scope.as_ref().unwrap_or(env),
                false,
            );
            return Some(ContextualCallback::unchecked());
        }
        if signature.rest.is_some() {
            self.error(
                "PNX9001",
                form.span,
                "A contextual callback currently requires a fixed-arity s/fspec",
            );
            return Some(ContextualCallback::unchecked());
        }
        if self.callback_depth >= MAX_CALLBACK_DEPTH {
            if saved.is_some() {
                self.type_query = None;
            }
            self.error(
                "PNX9001",
                form.span,
                format!("Contextual callback nesting limit ({MAX_CALLBACK_DEPTH}) exceeded"),
            );
            return Some(ContextualCallback::unchecked());
        }
        if saved.is_some() && !self.local_inline_step(form.span) {
            return Some(ContextualCallback::unchecked());
        }
        let start = self.diagnostics.len();
        let arguments = signature
            .args
            .iter()
            .cloned()
            .map(|ty| (Value::new(ty), form.span))
            .collect();
        // Constructing a closure does not execute its body. In particular a
        // surrounding try must not catch exceptions from this speculative check.
        let enclosing_throws = self.throws.take();
        let enclosing_unproven = std::mem::take(&mut self.has_unproven_expectation);
        self.callback_depth += 1;
        let result = self.check_inline(
            function,
            arguments,
            scope.as_ref().unwrap_or(env),
            InlineSite {
                constructor: constructor.span,
                call,
                invoked: true,
                callback: Some(CallbackCheck {
                    argument: form.span,
                    expected_return: matches!(mode, CallbackReturn::Check)
                        .then_some(&signature.ret),
                    origin,
                }),
            },
        );
        self.callback_depth -= 1;
        self.throws = enclosing_throws;
        let unproven = self.has_unproven_expectation;
        self.has_unproven_expectation |= enclosing_unproven;
        let findings = &self.diagnostics[start..];
        if !findings.is_empty() {
            // The return type is evidence about a redundant inference gap,
            // never a usable type for an invalid callback.
            let rejected_return = (matches!(mode, CallbackReturn::Infer)
                && !unproven
                && findings.iter().any(|finding| finding.code == "PNX2322")
                && findings
                    .iter()
                    .all(|finding| !finding.code.starts_with("PNX9")))
            .then_some(result.known);
            return Some(ContextualCallback {
                value: Value::new(Type::Unknown),
                rejected_return,
            });
        }
        let ty = match mode {
            CallbackReturn::Check
                if !signature.nil_preserving && result.known.subtype_of(&signature.ret) =>
            {
                expected.clone()
            }
            CallbackReturn::Infer => Type::Function(
                [Signature {
                    ret: result.known,
                    nil_preserving: false,
                    ..signature.clone()
                }]
                .into(),
            ),
            CallbackReturn::Check => Type::Unknown,
        };
        Some(ContextualCallback {
            value: Value::new(ty),
            rejected_return: None,
        })
    }

    pub(super) fn call_inline(
        &mut self,
        callable: InlineFunction<'_>,
        args: Vec<(Value, Span)>,
        constructor: Span,
        call: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        self.check_inline(
            callable,
            args,
            env,
            InlineSite {
                constructor,
                call,
                invoked,
                callback: None,
            },
        )
    }

    fn check_inline(
        &mut self,
        callable: InlineFunction<'_>,
        args: Vec<(Value, Span)>,
        env: &Env,
        site: InlineSite<'_>,
    ) -> Value {
        match callable {
            InlineFunction::Function { parts, raw } => {
                self.call_inline_fn(parts, args, env, site, raw)
            }
            InlineFunction::Literal(function) => {
                if function.required > MAX_FIXED_PARAMETERS {
                    self.error(
                        "PNX2554",
                        site.constructor,
                        "Clojure/JVM supports at most 20 fixed parameters",
                    );
                    return Value::new(Type::Unknown);
                }
                let clauses = literal_clause(function, site.constructor);
                self.call_inline_fn(&clauses, args, env, site, false)
            }
        }
    }

    fn call_inline_fn(
        &mut self,
        parts: &[Form],
        args: Vec<(Value, Span)>,
        env: &Env,
        site: InlineSite<'_>,
        raw: bool,
    ) -> Value {
        let parts = if let Some((head, parts)) = parts.split_first()
            && let Some(name) = head.symbol()
        {
            if !raw && name.contains('/') && name != "/" {
                self.error(
                    "PNX1001",
                    head.span,
                    "fn requires an unqualified local name",
                );
                return Value::new(Type::Unknown);
            }
            parts
        } else {
            parts
        };
        let parsed = if raw {
            fn_star_clauses(parts, site.constructor)
        } else {
            function_clauses(parts, site.constructor)
        };
        let clauses = match parsed {
            Ok(clauses) => clauses,
            Err(diagnostic) => {
                self.diagnostics.push(*diagnostic);
                return Value::new(Type::Unknown);
            }
        };
        let selected_arity = clauses
            .keys()
            .copied()
            .find(|arity| arity.accepts(args.len()));
        let missing_arity = site.invoked && selected_arity.is_none();
        if missing_arity {
            let message = if clauses.is_empty() {
                format!("fn* has no callable arities, got {} arguments", args.len())
            } else {
                format!(
                    "fn expects {} arguments, got {}",
                    super::arity_counts(clauses.keys().copied()),
                    args.len()
                )
            };
            let mut diagnostic = Diagnostic::new(
                "PNX2554",
                site.callback
                    .map_or(site.call, |callback| callback.argument),
                message,
            );
            diagnostic.contract = site.callback.and_then(|callback| callback.origin.cloned());
            self.diagnostics.push(diagnostic);
            self.arity_error();
        }
        let start = self.diagnostics.len();
        let mut result = Value::new(Type::Never);
        for (arity, clause) in clauses {
            let selected = site.invoked && Some(arity) == selected_arity;
            let checked = self.check_path(selected, |checker| {
                let parameters = if selected {
                    pack_arguments(arity, args.iter().map(|(value, _)| value.clone()).collect())
                } else {
                    vec![Value::new(Type::Never); arity.bindings()]
                };
                checker.function_body(
                    &clause.bindings(),
                    clause.conditions,
                    clause.body,
                    parameters,
                    env,
                    clause.span,
                )
            });
            if selected {
                if let Some(CallbackCheck {
                    expected_return: Some(expected),
                    origin,
                    ..
                }) = site.callback
                {
                    self.expect_contract(
                        &checked,
                        expected,
                        clause.body.last().map_or(clause.span, |form| form.span),
                        "Return value of callback",
                        origin,
                    );
                }
                result = checked;
            }
        }
        let context = if site.callback.is_some() {
            "callback passed"
        } else if site.constructor == site.call && !site.invoked {
            "fn literal"
        } else {
            "fn called"
        };
        for diagnostic in &mut self.diagnostics[start..] {
            if let Some(origin) = site.callback.and_then(|callback| callback.origin)
                && diagnostic.contract.as_ref() != Some(origin)
                && diagnostic.callback_contract.is_none()
            {
                diagnostic.callback_contract = Some(origin.clone());
            }
            diagnostic.notes.push(format!(
                "While checking inline {context} at {}:{}",
                site.call.start.line, site.call.start.column
            ));
        }
        result
    }

    fn prepare_fnil<'form>(
        &mut self,
        parts: &'form [Form],
        span: Span,
        env: &Env,
    ) -> Callable<'form, 'model> {
        if !(2..=4).contains(&parts.len()) {
            self.error(
                "PNX2554",
                span,
                "fnil requires a function and one to three defaults",
            );
            return Callable::Value(Value::new(Type::Unknown));
        }
        let target = self.prepare_callable(&parts[0], env);
        let target_returns = target.returns();
        let defaults: Vec<_> = self.eval_args(&parts[1..], env, target_returns).collect();
        let returns = target_returns && call_returns(&defaults);
        Callable::Fnil(Box::new(Fnil {
            target,
            defaults,
            returns,
        }))
    }

    fn prepare_partial<'form>(
        &mut self,
        parts: &'form [Form],
        span: Span,
        env: &Env,
    ) -> PreparedCallable<'form, 'model> {
        let [callee, bound @ ..] = parts else {
            self.error("PNX2554", span, "partial requires a function");
            self.arity_error();
            return PreparedCallable::new(Callable::Value(Value::new(Type::Never)), span);
        };
        let target = self.prepare_callable(callee, env);
        // The one-argument arity returns the original value, including
        // non-callable values. Other arities return a deferred call wrapper.
        if bound.is_empty() {
            return target;
        }
        let prefix: Vec<_> = self.eval_args(bound, env, target.returns()).collect();
        let returns = target.returns() && call_returns(&prefix);
        PreparedCallable::new(
            Callable::Partial(Box::new(Partial {
                target,
                prefix,
                returns,
            })),
            span,
        )
    }

    pub(super) fn call_fnil(
        &mut self,
        wrapper: &Fnil<'_, '_>,
        mut args: Vec<(Value, Span)>,
        call: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        // The three-default version also has a two-argument overload.
        let required = wrapper.defaults.len().min(2);
        if invoked && args.len() < required {
            self.error(
                "PNX2554",
                call,
                format!("fnil wrapper expects at least {required} arguments"),
            );
            return Value::new(Type::Unknown);
        }
        if invoked {
            for ((arg, span), (default, default_span)) in args.iter_mut().zip(&wrapper.defaults) {
                if nil_part(&arg.known) == Type::Never {
                    continue;
                }
                let non_nil = arg.known.without_nil();
                if non_nil == Type::Never {
                    *arg = default.clone();
                    *span = *default_span;
                } else {
                    arg.narrow(Type::union([non_nil, default.known.clone()]));
                    arg.notes.extend(default.notes.iter().cloned());
                }
            }
        }
        self.call_prepared(&wrapper.target, args, call, env, invoked)
    }
}

fn literal_clause(function: &AnonymousFunction, span: Span) -> [Form; 2] {
    let symbol = |name| Form {
        kind: Kind::Symbol(name),
        span,
    };
    let mut bindings: Vec<_> = (1..=function.required)
        .map(|index| symbol(format!("%{index}")))
        .collect();
    if function.variadic {
        bindings.extend([symbol("&".into()), symbol("%&".into())]);
    }
    [
        Form {
            kind: Kind::Vector(bindings),
            span,
        },
        Form {
            kind: Kind::List(function.body.clone()),
            span,
        },
    ]
}
