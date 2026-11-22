//! Function resolution, call-site inference and built-in calls.
mod apply;
mod arithmetic;
mod combinators;
mod comprehensions;
mod diagnostics;
mod forms;
mod generics;
mod identifiers;
mod indexing;
mod inline;
mod interop;
mod macro_scopes;
mod reduction;
mod search;
mod sequences;
mod strings;
mod vectors;
pub(super) use vectors::{narrow_vector_predicate, vector_predicate};

pub(super) use identifiers::identifier_name;
pub(super) use sequences::{iteration_may_throw, narrow_emptiness, narrow_seq_emptiness};
pub(super) use strings::concatenate_literal_strings;

use super::flow::Conditional;
use super::{
    Checker, Env, LiteralContainer, Value,
    env::CapturedInline,
    exceptions::Throws,
    function_forms::pack_arguments,
    model::{DeclaredValue, Function},
    nil_part,
    queries::InferredCall,
};
use crate::reader::{Form, Span};
use crate::types::{Arity, Compatibility, IntegerType, Signature, Type};
use crate::{ContractOrigin, Diagnostic};
use combinators::CombinedCallable;
use forms::CoreForm;
use inline::{CallbackReturn, Fnil, InlineFunction, Partial};

enum Callable<'form, 'model> {
    Inline(InlineFunction<'form>),
    Captured {
        function: CapturedInline,
        compile_when_uninvoked: bool,
    },
    Fnil(Box<Fnil<'form, 'model>>),
    Partial(Box<Partial<'form, 'model>>),
    Combined(CombinedCallable<'form, 'model>),
    Private(&'model Function),
    Builtin(String),
    Value(Value),
}

impl Callable<'_, '_> {
    fn returns(&self) -> bool {
        match self {
            Self::Value(value) => !value.stops_evaluation(),
            Self::Fnil(wrapper) => wrapper.returns,
            Self::Partial(wrapper) => wrapper.returns,
            Self::Combined(combined) => combined.returns,
            _ => true,
        }
    }
}

pub(super) struct PreparedCallable<'form, 'model> {
    kind: Callable<'form, 'model>,
    span: Span,
    literal_keyword: Option<&'form Form>,
}

impl<'form, 'model> PreparedCallable<'form, 'model> {
    fn new(kind: Callable<'form, 'model>, span: Span) -> Self {
        Self {
            kind,
            span,
            literal_keyword: None,
        }
    }

    pub(super) fn returns(&self) -> bool {
        self.kind.returns()
    }
}

pub(super) struct Specialization {
    value: Value,
    diagnostics: Vec<Diagnostic>,
    throws: Option<Throws>,
}

#[derive(Clone, Copy)]
pub(super) struct CallArguments<'a> {
    pub(super) values: &'a [(Value, Span)],
    forms: ArgumentForms<'a>,
    output_gap_explained: bool,
}

#[derive(Clone, Copy)]
enum ArgumentForms<'a> {
    Written(&'a [Form]),
    Projected(&'a [Option<&'a Form>]),
    Unavailable,
}

impl<'a> CallArguments<'a> {
    pub(super) fn written_forms(self) -> Option<&'a [Form]> {
        match self.forms {
            ArgumentForms::Written(forms) => Some(forms),
            _ => None,
        }
    }

    fn form_at(self, index: usize) -> Option<&'a Form> {
        match self.forms {
            ArgumentForms::Written(forms) => forms.get(index),
            ArgumentForms::Projected(forms) => forms.get(index).copied().flatten(),
            ArgumentForms::Unavailable => None,
        }
    }
}

struct EvaluatedArguments {
    values: Vec<(Value, Span)>,
    output_gap_explained: bool,
}

struct ContextualArgument<'a> {
    callback: &'a Form,
    expected: &'a Type,
    map_key: Option<&'a Type>,
    map_ancestors: Box<[&'a Type]>,
}

impl ContextualArgument<'_> {
    fn expected_in<'a>(&self, mut instantiated: &'a Type) -> Option<&'a Type> {
        for key in self.map_ancestors.iter().copied().chain(self.map_key) {
            let Type::Map(map) = instantiated else {
                return None;
            };
            instantiated = map.fields.get(key)?.value_constraint()?;
        }
        Some(instantiated)
    }

    fn apply(self, argument: &mut Value, callback: Value) {
        let Some(key) = self.map_key else {
            *argument = callback;
            return;
        };
        'shape: for (mut shape, ty) in [
            (&mut argument.known, callback.known),
            (&mut argument.ty, callback.ty),
        ] {
            for ancestor in self.map_ancestors.iter().copied() {
                let Type::Map(map) = shape else {
                    continue 'shape;
                };
                let Some(field) = map.fields.get_mut(ancestor) else {
                    continue 'shape;
                };
                shape = &mut field.ty;
            }
            if let Type::Map(map) = shape
                && let Some(field) = map.fields.get_mut(key)
            {
                field.replace_type(ty);
            }
        }
    }
}

impl From<Vec<(Value, Span)>> for EvaluatedArguments {
    fn from(values: Vec<(Value, Span)>) -> Self {
        Self {
            values,
            output_gap_explained: false,
        }
    }
}

impl<'model> Checker<'model> {
    #[cold]
    pub(super) fn dotted_function_class_use(
        &mut self,
        name: &str,
        span: Span,
        callable: bool,
    ) -> Option<Value> {
        let qualified =
            self.model
                .ordinary_dotted_function_at(name, self.globals, span.start.offset)?;
        let spelling = self.model.qualified_var_spelling(qualified);
        let message = super::Model::dotted_class_use_message(name, qualified, spelling.as_deref());
        if callable {
            self.dotted_callable_sites.push(span);
        }
        Some(self.unchecked_form(span, &message))
    }

    fn inferred_call(&mut self, function: &Function, args: Vec<Value>, span: Span) -> Value {
        let returns = args.iter().all(|arg| arg.known != Type::Never);
        let Some((&arity, clause)) = function
            .arities
            .iter()
            .find(|(arity, _)| arity.accepts(args.len()))
        else {
            if self.path_reachable && returns {
                self.error(
                    "PNX2554",
                    span,
                    format!(
                        "{} expects {} arguments, got {}",
                        function.name,
                        arity_counts(function.arities.keys().copied()),
                        args.len()
                    ),
                );
                self.arity_error();
            }
            return Value::new(Type::Never);
        };
        if !returns {
            return Value::new(Type::Never);
        }
        if self.active_helpers.contains(&function.name) {
            self.may_throw();
            self.error(
                "PNX9002",
                span,
                format!(
                    "Recursive private function {} needs a :t/sig annotation",
                    function.name
                ),
            );
            return Value::new(Type::Unknown);
        }
        // Compiler checks on dead paths cannot establish coverage, including for nested calls.
        let key = (function.name.clone(), args, self.path_reachable);
        let start = self.diagnostics.len();
        let tracking = self.throws.is_some();
        let result = if let Some(cached) = self
            .specializations
            .get(&key)
            .filter(|cached| !tracking || cached.throws.is_some())
        {
            self.diagnostics.extend(cached.diagnostics.iter().cloned());
            if let Some((enclosing, throws)) = self.throws.as_mut().zip(cached.throws.as_ref()) {
                enclosing.extend(throws.clone());
            }
            cached.value.clone()
        } else {
            if self.active_helpers.len() >= 32
                || (self.specializations.len() + self.active_helpers.len() >= 256
                    && !self.specializations.contains_key(&key))
            {
                self.may_throw();
                self.error("PNX9002", span,
                    "Private inference limit reached (32 nested calls or 256 specializations); add :t/sig annotations");
                return Value::new(Type::Unknown);
            }
            self.active_helpers.push(function.name.clone());
            let enclosing = std::mem::replace(&mut self.throws, tracking.then(Throws::default));
            let result = self.function_body(
                &clause.params,
                clause.conditions(),
                &clause.body,
                pack_arguments(arity, key.1.clone()),
                &Env::new(),
                clause.span,
            );
            self.active_helpers.pop();
            let throws = std::mem::replace(&mut self.throws, enclosing);
            if let Some((enclosing, throws)) = self.throws.as_mut().zip(throws.as_ref()) {
                enclosing.extend(throws.clone());
            }
            self.specializations.insert(
                key,
                Specialization {
                    value: result.clone(),
                    diagnostics: self.diagnostics[start..].to_vec(),
                    throws,
                },
            );
            result
        };
        for diagnostic in &mut self.diagnostics[start..] {
            diagnostic.notes.push(format!(
                "While checking call to {} at {}:{}",
                function.name, span.start.line, span.start.column
            ));
        }
        result
    }

    pub(super) fn list(&mut self, xs: &[Form], span: Span, env: &Env, tail: bool) -> Value {
        self.observe_completion(xs[0].without_metadata().span, env);
        if let Some(name) = xs[0].symbol()
            && !env.contains_key(name)
            && let Some(value) =
                self.unchecked_alias_value(name, xs[0].without_metadata().span, env)
        {
            return value;
        }
        if !self.allows_partial_call(&xs[0], env) {
            self.partial_method_opaque |= self.partial_method_body;
            return Value::new(Type::Unknown);
        }
        match xs[0].symbol() {
            Some("recur") => return self.recur_form(&xs[1..], span, env, tail),
            Some("let*") => return self.let_form(xs, span, env, tail, true),
            Some("loop*") => return self.loop_form(xs, span, env, true),
            Some("fn*") => return self.fn_value(&xs[1..], span, env, true),
            Some("if") => return self.if_form(xs, span, env, Conditional::If, tail),
            Some("do") => return self.body(&xs[1..], env, tail),
            Some("quote") => return self.quote_form(&xs[1..], span),
            Some("var") => {
                if let [target] = &xs[1..]
                    && target.symbol().is_some()
                {
                    return self.unchecked_form(span, "Var object types are not supported");
                }
                self.error("PNX1001", span, "var requires one symbol");
                return Value::new(Type::Unknown);
            }
            Some("throw") => return self.throw_form(&xs[1..], span, env),
            Some("try") => return self.try_form(&xs[1..], env, tail),
            Some("new") => return self.jvm_new_form(&xs[1..], span, env),
            Some(name) if !name.contains('/') && (name.starts_with('.') || name.ends_with('.')) => {
                if let Some(value) = self.jvm_call(name, xs[0].span, &xs[1..], span, env) {
                    return value;
                }
            }
            _ => {}
        }
        if let Some(name) = xs[0].symbol()
            && name.contains('.')
            && !env.contains_key(name)
            && let Some(value) =
                self.dotted_function_class_use(name, xs[0].without_metadata().span, true)
        {
            self.eval_args(&xs[1..], env, false).for_each(drop);
            return value;
        }
        if let Some(name) = xs[0].symbol()
            && !env.contains_key(name)
            && let Some(function) = self
                .model
                .local_function_at(name, xs[0].span.start.offset)
                .filter(|f| f.inferred())
        {
            let start = self.diagnostics.len();
            let args: Vec<_> = self
                .eval_args(&xs[1..], env, true)
                .map(|(value, _)| value)
                .collect();
            let observe = self.observes_inferred_call_at(span)
                && args.iter().all(|arg| arg.known != Type::Never);
            let inferred = (observe
                && function.arities.contains_key(&Arity::Fixed(args.len()))
                && args.iter().all(|arg| !arg.known.contains_unknown()))
            .then(|| InferredCall {
                definition: function.span,
                labels: None,
                args: args.iter().map(|arg| arg.known.clone()).collect(),
            });
            let result = self.inferred_call(function, args, span);
            if observe {
                let inferred = inferred.filter(|_| start == self.diagnostics.len());
                self.observe_inferred_call(span, &xs[1..], inferred, &result);
            }
            return result;
        }
        if let Some(name) = xs[0].symbol()
            && !env.contains_key(name)
        {
            match self
                .model
                .declared_value(name, self.globals, xs[0].span.start.offset)
            {
                Some(DeclaredValue::Macro(scope)) => {
                    self.withhold_captured_facts(env.inline_functions());
                    self.withhold_literals_after_opaque_form();
                    return self.macro_scope(scope, &xs[1..], span, env);
                }
                Some(DeclaredValue::OpaqueMacro) => {
                    return self.unchecked_macro(name, xs[0].span, env);
                }
                Some(DeclaredValue::PrivateMacro) => {
                    self.error(
                        "PNX2304",
                        xs[0].span,
                        format!("Unresolved or private value {name}"),
                    );
                    return Value::new(Type::Unknown);
                }
                Some(DeclaredValue::UncheckedFunction(function)) => {
                    if let Some(arities) = &function.arities {
                        if self
                            .eval_args(&xs[1..], env, true)
                            .last()
                            .is_some_and(|(arg, _)| arg.known == Type::Never)
                        {
                            return Value::new(Type::Never);
                        }
                        let count = xs.len() - 1;
                        if !arities.iter().any(|arity| arity.accepts(count)) {
                            self.error(
                                "PNX2554",
                                xs[0].span,
                                format!(
                                    "{name} expects {} arguments, got {count}",
                                    arity_counts(arities.iter().copied())
                                ),
                            );
                            self.may_throw();
                            return Value::new(Type::Unknown);
                        }
                    } else {
                        self.withhold_captured_facts(env.inline_functions());
                        self.withhold_literals_after_opaque_form();
                    }
                    return self.unchecked_function(name, xs[0].span);
                }
                None => {
                    let resolved = self.model.resolve(name);
                    if let Some(value) = self.spec_operation_call(&resolved, &xs[1..], span, env) {
                        return value;
                    }
                    if resolved == "clojure.core/deref" {
                        let stock = self.model.can_use_standard_core(name, self.globals);
                        return self.deref_call(&xs[1..], span, env, stock);
                    }
                    if resolved == "clojure.core/into"
                        && let Some(result) = self.collect_vector(xs, span, env)
                    {
                        return result;
                    }
                    if let Some(form) = CoreForm::resolve(&resolved) {
                        return self.core_form(form, &resolved, xs, span, env, tail);
                    }
                    if self.is_available_builtin(name, &resolved) {
                        if matches!(resolved.as_str(), "clojure.core/min" | "clojure.core/max") {
                            return self.direct_extremum(&resolved, &xs[1..], span, env);
                        }
                        let args = self.typed_args(&xs[1..], env);
                        return self.builtin(&resolved, &args, span, Some(name));
                    }
                    if self
                        .model
                        .known_standard_core_function(name, &resolved, self.globals)
                    {
                        return if self
                            .eval_args(&xs[1..], env, true)
                            .last()
                            .is_some_and(|(argument, _)| argument.stops_evaluation())
                        {
                            Value::new(Type::Never)
                        } else {
                            self.unchecked_core_function(&resolved, xs[0].span)
                        };
                    }
                    if self
                        .model
                        .standard_core_binding(name, &resolved, self.globals)
                        == Some(crate::core_names::CoreBinding::Macro)
                    {
                        return self.unchecked_macro(&resolved, xs[0].span, env);
                    }
                }
                _ => {}
            }
        }
        let start = self.diagnostics.len();
        if let Some(callable) = self.prepare_inline(&xs[0], env) {
            let args: Vec<_> = self.eval_args(&xs[1..], env, callable.returns()).collect();
            let invoked = call_returns(&args);
            // Prepared callbacks elsewhere use synthetic arguments. Only
            // this call has the written forms for signature help.
            if invoked
                && self.observes_inferred_call_at(span)
                && let Callable::Captured { function, .. } = &callable.kind
            {
                let local = self.local_call(function, &args);
                let result = self.call_prepared(&callable, args, span, env, true);
                let local = local.filter(|_| start == self.diagnostics.len());
                self.observe_inferred_call(span, &xs[1..], local, &result);
                return result;
            }
            return self.call_prepared(&callable, args, span, env, invoked);
        }
        let function = self.infer(&xs[0], env);
        // An unresolved var might be a macro. Its arguments are inspected for
        // findings, but may not be evaluated in this lexical scope at runtime.
        if function.known == Type::Unknown
            && xs[0].symbol().is_some_and(|name| !env.contains_key(name))
        {
            self.withhold_captured_facts(env.inline_functions());
            self.withhold_literals_after_opaque_form();
            let observation = self.type_query.take();
            let enclosing_throws = self.throws.take();
            // An unconfirmed macro may reorder or discard operands. Check
            // each provisionally without taking its effects as call evidence.
            for operand in &xs[1..] {
                self.infer_path(operand, env, true, false);
            }
            self.type_query = observation;
            self.throws = enclosing_throws;
            self.may_throw();
            return Value::new(Type::Unknown);
        }
        let args = self.call_args(&function, &xs[1..], span, env);
        let value = self.call_value(
            &function,
            CallArguments {
                values: &args.values,
                forms: ArgumentForms::Written(&xs[1..]),
                output_gap_explained: args.output_gap_explained,
            },
            xs[0].span,
            span,
        );
        if self.type_query.is_some() && xs[0].keyword().is_some() {
            self.observe_lookup(xs[0].without_metadata().span, &value);
        }
        value
    }

    fn unchecked_macro(&mut self, name: &str, span: Span, env: &Env) -> Value {
        self.withhold_captured_facts(env.inline_functions());
        self.withhold_literals_after_opaque_form();
        self.unchecked_form(span, &format!("Macro {name} has no checked scope"))
    }

    pub(super) fn unchecked_function(&mut self, name: &str, span: Span) -> Value {
        self.may_throw();
        self.diagnostics.push(Diagnostic::analysis_gap(
            "PNX9002",
            span,
            format!("No checked signature for source function {name}"),
        ));
        Value::new(Type::Unknown)
    }

    pub(super) fn unchecked_core_function(&mut self, resolved: &str, span: Span) -> Value {
        if !self.path_reachable {
            return Value::new(Type::Never);
        }
        self.unchecked_form(
            span,
            &format!("Type analysis for {resolved} is unavailable"),
        )
    }

    fn deref_call(&mut self, args: &[Form], span: Span, env: &Env, stock: bool) -> Value {
        let returns = self
            .eval_args(args, env, true)
            .last()
            .is_none_or(|(value, _)| value.known != Type::Never);
        if stock && (!returns || !self.path_reachable) {
            return Value::new(Type::Never);
        }
        let actual = args.len();
        if !matches!(actual, 1 | 3) {
            self.error(
                "PNX2554",
                span,
                format!("deref expects 1 or 3 arguments, got {actual}"),
            );
            if stock {
                self.arity_error();
                return Value::new(Type::Never);
            }
            if returns {
                self.may_throw();
            }
        } else if returns {
            return self.unchecked_form(span, "Dereferenced value types are not supported");
        }
        Value::new(if returns { Type::Unknown } else { Type::Never })
    }

    fn call_args(
        &mut self,
        function: &Value,
        forms: &[Form],
        call: Span,
        env: &Env,
    ) -> EvaluatedArguments {
        let signature = match &function.known {
            Type::Function(signatures) => signatures
                .iter()
                .find(|sig| sig.arity().accepts(forms.len())),
            _ => None,
        };
        if let Some(signature) = signature.filter(|sig| sig.parameters.is_some()) {
            return self.generic_arguments(function, signature, forms, call, env);
        }
        let Some(signature) = signature.filter(|sig| {
            sig.args
                .iter()
                .chain(&sig.rest)
                .any(|ty| matches!(ty, Type::Function(_)))
                || forms.iter().enumerate().any(|(index, form)| {
                    sig.args
                        .get(index)
                        .or(sig.rest.as_ref())
                        .is_some_and(|expected| self.contextual_map(form, expected, env).is_some())
                })
        }) else {
            return self
                .eval_args(forms, env, !function.stops_evaluation())
                .collect::<Vec<_>>()
                .into();
        };
        self.call_contextual_arguments(function, signature, forms, call, env)
    }

    #[inline(never)]
    fn call_contextual_arguments(
        &mut self,
        function: &Value,
        signature: &Signature,
        forms: &[Form],
        call: Span,
        env: &Env,
    ) -> EvaluatedArguments {
        let infer_argument = |checker: &mut Self, form: &Form, expected: &Type, reachable| {
            checker.check_path(reachable, |checker| {
                checker
                    .contextual_callback(
                        form,
                        expected,
                        call,
                        function.contract.as_deref(),
                        env,
                        CallbackReturn::Check,
                    )
                    .map(|callback| callback.value)
                    .unwrap_or_else(|| checker.infer(form, env))
            })
        };
        let mut values = Vec::with_capacity(forms.len());
        let mut deferred = Vec::new();
        let mut reachable = env.reachable();
        for (index, form) in forms.iter().enumerate() {
            let expected = signature
                .args
                .get(index)
                .or(signature.rest.as_ref())
                .unwrap();
            if matches!(expected, Type::Function(signatures) if signatures.len() == 1)
                && self.contextual_callback_form(form, env)
            {
                // Reading or creating an ordinary closure does not run its body.
                let argument = self.contextual_callback_argument(form, env, reachable);
                reachable = !argument.stops_evaluation();
                deferred.push((
                    index,
                    ContextualArgument {
                        callback: form,
                        expected,
                        map_key: None,
                        map_ancestors: Box::new([]),
                    },
                ));
                values.push((argument, form.span));
                continue;
            }
            let mut value = self
                .contextual_map_argument(form, expected, env, reachable, |callback, _| {
                    deferred.push((index, callback))
                })
                .unwrap_or_else(|| infer_argument(self, form, expected, reachable));
            reachable = !value.stops_evaluation();
            if !reachable {
                value.narrow(Type::Never);
            }
            values.push((value, form.span));
        }
        for (index, contextual) in deferred {
            let value = infer_argument(self, contextual.callback, contextual.expected, reachable);
            if reachable {
                contextual.apply(&mut values[index].0, value);
            }
        }
        values.into()
    }

    fn typed_args(&mut self, args: &[Form], env: &Env) -> Vec<(Value, Span)> {
        self.eval_args(args, env, true).collect()
    }

    pub(super) fn reject_runtime_arity(
        &mut self,
        forms: &[Form],
        span: Span,
        env: &Env,
        message: impl Into<String>,
    ) -> Value {
        let operands_return = self
            .eval_args(forms, env, self.path_reachable)
            .last()
            .is_none_or(|(value, _)| !value.stops_evaluation());
        if self.path_reachable && operands_return {
            self.error("PNX2554", span, message);
            self.arity_error();
        }
        Value::new(Type::Never)
    }

    fn call_value(
        &mut self,
        function: &Value,
        arguments: CallArguments<'_>,
        head: Span,
        span: Span,
    ) -> Value {
        let args = arguments.values;
        if !self.path_reachable || function.stops_evaluation() || !call_returns(args) {
            return Value::new(Type::Never);
        }
        if matches!(
            function.known,
            Type::Key(_) | Type::Keyword | Type::SpecKeyword
        ) && !(1..=2).contains(&args.len())
        {
            self.error(
                "PNX2554",
                span,
                format!(
                    "Keyword invocation expects 1 or 2 arguments, got {}",
                    args.len()
                ),
            );
            self.arity_error();
            return Value::new(Type::Never);
        }
        if function.known != Type::Never {
            self.may_throw();
        }
        match &function.known {
            Type::Function(signatures) => {
                let Some(signature) = signatures
                    .iter()
                    .find(|sig| sig.arity().accepts(args.len()))
                else {
                    if self.type_query.is_some()
                        && let Some(nearest) = signatures
                            .iter()
                            .find(|sig| sig.arity().required() >= args.len())
                            .or_else(|| signatures.last())
                    {
                        self.observe_call(
                            span,
                            arguments,
                            nearest,
                            nearest,
                            function.contract.as_ref(),
                        );
                    }
                    let mut diagnostic = Diagnostic::new(
                        "PNX2554",
                        span,
                        format!(
                            "Expected {} arguments, got {}",
                            arity_counts(signatures.iter().map(Signature::arity)),
                            args.len()
                        ),
                    );
                    diagnostic.contract = function.contract.as_deref().cloned();
                    self.diagnostics.push(diagnostic);
                    return Value::new(Type::Unknown);
                };
                if signature.parameters.is_some() {
                    return self.call_generic(signature, function, arguments, span);
                }
                let specialized = args
                    .first()
                    .and_then(|(value, _)| signature.specialize_nil_result(&value.known));
                let active = specialized.as_ref().unwrap_or(signature);
                self.observe_call(
                    span,
                    arguments,
                    signature,
                    active,
                    function.contract.as_ref(),
                );
                let mismatch = self.check_call_arguments(signature, function, arguments);
                if mismatch {
                    return Value::rejected_call();
                }
                if signature.nil_preserving
                    && args[0].0.known.compatibility(&signature.args[0]) != Compatibility::Proven
                {
                    return Value::new(Type::Unknown);
                }
                Value::new(active.ret.clone())
            }
            kind @ (Type::Key(_) | Type::Keyword | Type::SpecKeyword | Type::Map(_)) => {
                let is_map = matches!(kind, Type::Map(_));
                if is_map && !(1..=2).contains(&args.len()) {
                    self.error(
                        "PNX2554",
                        span,
                        format!(
                            "Map invocation expects 1 or 2 arguments, got {}",
                            args.len()
                        ),
                    );
                    return Value::new(Type::Unknown);
                }
                let default = args.get(1).map(|(value, _)| value);
                if is_map {
                    self.lookup_default(function, &args[0].0.known, default, args[0].1)
                } else {
                    self.lookup_default(&args[0].0, kind, default, head)
                }
            }
            Type::Union(parts) => parts.iter().fold(Value::new(Type::Never), |result, part| {
                let alternative = Value {
                    contract: function.contract.clone(),
                    guard: function.guard.clone(),
                    ..Value::with_notes(part.clone(), function.notes.clone())
                };
                result.join(self.call_value(&alternative, arguments, head, span))
            }),
            Type::Vector(_) | Type::Tuple(_) | Type::MapEntry(_) => {
                self.vector_call(function, head, args, span)
            }
            Type::Set(element) => {
                if args.len() != 1 {
                    self.error(
                        "PNX2554",
                        span,
                        format!(
                            "A set used as a function expects 1 argument, got {}",
                            args.len()
                        ),
                    );
                    Value::new(Type::Unknown)
                } else {
                    Value::with_notes(function.set_lookup_type(element, &args[0].0.known, Type::Nil), vec![
                        "A set used as a function returns the matching value or Nil, not a Boolean.".into(),
                    ])
                }
            }
            Type::Unknown | Type::Never => Value::new(function.known.clone()),
            _ => self.unsupported(head, format!("Calling {} is not supported", function.ty)),
        }
    }

    fn check_call_arguments(
        &mut self,
        signature: &Signature,
        function: &Value,
        args: CallArguments<'_>,
    ) -> bool {
        let mut mismatch = false;
        for (index, (actual, span)) in args.values.iter().enumerate() {
            let expected = signature
                .args
                .get(index)
                .or(signature.rest.as_ref())
                .unwrap();
            let start = self.diagnostics.len();
            let argument = index + 1;
            self.expect_contract(
                actual,
                expected,
                *span,
                &format!("Argument {argument}"),
                function.contract.as_deref(),
            );
            if self
                .diagnostics
                .get(start)
                .is_some_and(|d| d.code == "PNX2322")
            {
                mismatch = true;
                if let Some(form) = args.form_at(index).filter(|form| form.span == *span) {
                    self.refine_argument_mismatch(start, argument, form, &actual.known, expected);
                }
            }
        }
        mismatch
    }

    pub(super) fn prepare_callable<'form>(
        &mut self,
        callee: &'form Form,
        env: &Env,
    ) -> PreparedCallable<'form, 'model> {
        let prepare = |kind| PreparedCallable {
            kind,
            span: callee.span,
            literal_keyword: callee.keyword().map(|_| callee.without_metadata()),
        };
        if let Some(callable) = self.prepare_inline(callee, env) {
            return callable;
        }
        if let Some(name) = callee.symbol()
            && !env.contains_key(name)
        {
            if name.contains('.')
                && let Some(value) =
                    self.dotted_function_class_use(name, callee.without_metadata().span, true)
            {
                return prepare(Callable::Value(value));
            }
            if let Some(value) =
                self.unchecked_alias_value(name, callee.without_metadata().span, env)
            {
                return prepare(Callable::Value(value));
            }
            if let Some(function) = self
                .model
                .local_function_at(name, callee.span.start.offset)
                .filter(|f| f.inferred())
            {
                return prepare(Callable::Private(function));
            }
            if self
                .model
                .declared_value(name, self.globals, callee.span.start.offset)
                .is_none()
            {
                let resolved = self.model.resolve(name);
                if self.is_available_builtin(name, &resolved) {
                    return prepare(Callable::Builtin(resolved));
                }
            }
        }
        prepare(Callable::Value(self.infer(callee, env)))
    }

    fn is_available_builtin(&self, spelling: &str, resolved: &str) -> bool {
        is_builtin(resolved)
            && (!matches!(
                resolved,
                "clojure.core/count"
                    | "clojure.core/first"
                    | "clojure.core/nth"
                    | "clojure.core/subvec"
                    | "clojure.core/str"
                    | "clojure.core/inc"
                    | "clojure.core/dec"
                    | "clojure.core/conj"
                    | "clojure.core/peek"
                    | "clojure.core/min"
                    | "clojure.core/max"
            ) || self.model.can_use_standard_core(spelling, self.globals))
    }

    pub(super) fn standard_core_name(&self, head: &Form, env: &Env) -> Option<String> {
        let name = head.symbol()?;
        let resolved = self.model.resolve(name);
        (resolved.starts_with("clojure.core/")
            && self.model.can_use_standard_core(name, self.globals)
            && !env.contains_key(name)
            && self
                .model
                .local_function_at(name, head.span.start.offset)
                .is_none()
            && self
                .model
                .declared_value(name, self.globals, head.span.start.offset)
                .is_none())
        .then_some(resolved)
    }

    pub(super) fn call_prepared(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        args: Vec<(Value, Span)>,
        span: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        self.call_prepared_with_forms(
            callable,
            args,
            ArgumentForms::Unavailable,
            span,
            env,
            invoked,
        )
    }

    pub(super) fn withhold_callback_facts(
        &mut self,
        callback: &PreparedCallable<'_, '_>,
        env: &Env,
    ) {
        if self.type_query.is_none() || !self.path_reachable {
            return;
        }
        match &callback.kind {
            Callable::Inline(_) => {
                self.withhold_type_facts(callback.span);
                self.withhold_captured_facts(env.inline_functions());
            }
            Callable::Captured { function, .. } => {
                self.withhold_captured_facts(Some(function));
            }
            Callable::Private(function) => self.withhold_type_facts(function.span),
            Callable::Fnil(wrapper) => self.withhold_callback_facts(&wrapper.target, env),
            Callable::Partial(wrapper) => self.withhold_callback_facts(&wrapper.target, env),
            Callable::Combined(combined) => {
                for stage in &combined.stages {
                    self.withhold_callback_facts(stage, env);
                }
            }
            Callable::Value(_) | Callable::Builtin(_) => {
                if let Some(keyword) = callback.literal_keyword {
                    self.withhold_type_facts(keyword.span);
                }
            }
        }
    }

    fn call_prepared_with_forms(
        &mut self,
        callable: &PreparedCallable<'_, '_>,
        args: Vec<(Value, Span)>,
        forms: ArgumentForms<'_>,
        span: Span,
        env: &Env,
        invoked: bool,
    ) -> Value {
        let invoked = invoked && callable.returns();
        match &callable.kind {
            Callable::Inline(parts) => {
                self.call_inline(*parts, args, callable.span, span, env, invoked)
            }
            Callable::Captured {
                function,
                compile_when_uninvoked,
            } => self.call_captured(function, args, span, env, invoked, *compile_when_uninvoked),
            Callable::Fnil(wrapper) => self.call_fnil(wrapper, args, span, env, invoked),
            Callable::Partial(wrapper) => {
                let args = wrapper.prefix.iter().cloned().chain(args).collect();
                self.call_prepared(&wrapper.target, args, span, env, invoked)
            }
            Callable::Combined(combined) => self.call_combiner(combined, args, span, env, invoked),
            _ if !invoked => Value::new(Type::Never),
            Callable::Private(function) => self.inferred_call(
                function,
                args.into_iter().map(|(value, _)| value).collect(),
                span,
            ),
            Callable::Builtin(name)
                if name == "clojure.core/str" && matches!(forms, ArgumentForms::Unavailable) =>
            {
                // Structural spreads and aggregate callbacks can supply values
                // through a custom traversal with different elements.
                self.may_throw();
                Value::new(Type::String)
            }
            Callable::Builtin(name) => self.builtin(name, &args, span, None),
            Callable::Value(function) => {
                let result = self.call_value(
                    function,
                    CallArguments {
                        values: &args,
                        forms,
                        output_gap_explained: false,
                    },
                    callable.span,
                    span,
                );
                if let Some(keyword) = callable.literal_keyword {
                    self.observe_lookup(keyword.span, &result);
                }
                result
            }
        }
    }

    fn builtin(
        &mut self,
        name: &str,
        args: &[(Value, Span)],
        span: Span,
        written_operator: Option<&str>,
    ) -> Value {
        let start = self.diagnostics.len();
        let value = self.check_builtin(name, args, span, written_operator);
        self.note_builtin_contract(start, name);
        value
    }

    fn note_builtin_contract(&mut self, start: usize, name: &str) {
        for diagnostic in &mut self.diagnostics[start..] {
            if matches!(
                diagnostic.code.as_str(),
                "PNX2322" | "PNX2554" | "PNX2801" | "PNX9004"
            ) {
                diagnostic
                    .contract
                    .get_or_insert_with(|| ContractOrigin::builtin(name));
            }
        }
    }

    fn check_builtin(
        &mut self,
        name: &str,
        args: &[(Value, Span)],
        span: Span,
        written_operator: Option<&str>,
    ) -> Value {
        let arity_ok = builtin_arities(name)
            .iter()
            .any(|arity| arity.accepts(args.len()));
        let returns = call_returns(args);
        if !arity_ok {
            let rejected_at_compile_time = written_operator.is_some_and(|operator| {
                name == "clojure.lang.MapEntry/create"
                    || (unconditional_core_inliner(name)
                        && self.model.can_use_standard_core(operator, self.globals))
            });
            if !rejected_at_compile_time && (!returns || !self.path_reachable) {
                return Value::new(Type::Never);
            }
            self.error(
                "PNX2554",
                span,
                format!("Invalid arity for {name}: {}", args.len()),
            );
            if rejected_at_compile_time {
                return Value::new(Type::Never);
            }
            if name.starts_with("clojure.core/") || name.starts_with("clojure.string/") {
                self.arity_error();
                return Value::new(Type::Never);
            }
            self.may_throw();
            return Value::new(Type::Unknown);
        }
        if !returns {
            return Value::new(Type::Never);
        }
        if matches!(name, "clojure.core/min" | "clojure.core/max") {
            return self.extremum(name, args);
        }
        let arithmetic = arithmetic::fixed_result(name, args);
        if arithmetic.is_none()
            && matches!(name, "clojure.core/inc" | "clojure.core/dec")
            && arithmetic::excludes_number(&args[0].0)
        {
            let (argument, span) = &args[0];
            self.expect(argument, &Type::Number, *span, name);
            if nil_part(&argument.known) != Type::Never {
                self.null_pointer();
            }
            if !argument.known.subtype_of(&Type::Nil) {
                self.class_cast();
            }
            return Value::new(Type::Never);
        }
        if self.throws.is_some()
            && arithmetic.is_none()
            && !matches!(
                name,
                "clojure.core/ex-info"
                    | "clojure.lang.MapEntry/create"
                    | "clojure.core/ex-data"
                    | "clojure.core/identity"
                    | "clojure.core/str"
                    | "clojure.core/name"
                    | "clojure.core/map?"
                    | "clojure.core/vector?"
                    | "clojure.core/any?"
                    | "clojure.core/disj"
                    | "clojure.core/dissoc"
                    | "clojure.core/find"
                    | "clojure.core/list"
                    | "clojure.core/cons"
                    | "clojure.core/conj"
                    | "clojure.core/peek"
                    | "clojure.core/set"
                    | "clojure.core/seq"
                    | "clojure.core/subvec"
                    | "clojure.core/first"
                    | "clojure.core/next"
                    | "clojure.core/rest"
                    | "clojure.core/keys"
                    | "clojure.core/vals"
                    | "clojure.core/zipmap"
                    | "clojure.core/reduced"
                    | "clojure.core/reduced?"
                    | "clojure.core/unreduced"
                    | "clojure.core/ensure-reduced"
                    | "clojure.core/nil?"
                    | "clojure.core/some?"
                    | "clojure.core/true?"
                    | "clojure.core/false?"
                    | "clojure.core/boolean"
                    | "clojure.core/not"
            )
            && predicate_type(name).is_none()
        {
            self.may_throw();
        }
        let ty = match name {
            "clojure.core/reduced"
            | "clojure.core/reduced?"
            | "clojure.core/unreduced"
            | "clojure.core/ensure-reduced" => {
                return reduction::reduced_operation(name, &args[0].0);
            }
            "clojure.core/=" => equality(args),
            "clojure.core/empty?" | "clojure.core/not-empty" => {
                return self.emptiness_call(name, &args[0]);
            }
            "clojure.core/nth" => return self.nth(args),
            "clojure.core/subvec" => return self.subvec(args),
            "clojure.core/list" => return sequences::list_value(args),
            "clojure.core/cons" => return self.cons(args),
            "clojure.core/conj" => return self.conj(args),
            "clojure.core/peek" => return self.peek(&args[0]),
            "clojure.core/merge" => return self.merge(args, span),
            "clojure.core/set" => return self.set_from_collection(&args[0]),
            "clojure.core/disj" => return self.disj(args),
            "clojure.core/dissoc" => return self.dissoc(args),
            "clojure.core/find" => return self.find_entry(&args[0], &args[1]),
            "clojure.core/select-keys" => return self.select_keys(&args[0], &args[1]),
            "clojure.core/zipmap" => return self.zipmap(&args[0], &args[1], span),
            "clojure.lang.MapEntry/create" => {
                let mut value = Value::new(Type::MapEntry(Box::new([
                    args[0].0.known.clone(),
                    args[1].0.known.clone(),
                ])));
                if args
                    .iter()
                    .any(|(value, _)| value.literal_container.is_some())
                {
                    value.literal_container = Some(LiteralContainer::Positions(
                        args.iter()
                            .map(|(value, _)| value.literal_container.clone())
                            .collect(),
                    ));
                }
                return value;
            }
            "clojure.core/ex-info" => return self.ex_info(args),
            "clojure.core/ex-data" => return self.ex_data(&args[0].0),
            "clojure.core/seq" | "clojure.core/first" | "clojure.core/next"
            | "clojure.core/rest" => {
                return self.sequence_call(name, &args[0].0, args[0].1);
            }
            "clojure.core/keys" | "clojure.core/vals" => {
                return self.map_sequence_call(name, &args[0]);
            }
            "clojure.core/+" | "clojure.core/-" | "clojure.core/*" | "clojure.core/inc"
            | "clojure.core/dec" => {
                for (value, span) in args {
                    self.expect(value, &Type::Number, *span, name);
                }
                if let Some(result) = arithmetic {
                    if result.may_overflow {
                        self.arithmetic_exception();
                    }
                    result.ty
                } else {
                    Type::Number
                }
            }
            "clojure.core/long" => {
                self.expect(
                    &args[0].0,
                    &Type::union([Type::Number, Type::Character]),
                    args[0].1,
                    name,
                );
                Type::Number
            }
            "clojure.string/upper-case"
            | "clojure.string/lower-case"
            | "clojure.string/capitalize" => self.case_string(name, &args[0]),
            "clojure.string/trim" => {
                self.expect(&args[0].0, &Type::String, args[0].1, name);
                Type::String
            }
            "clojure.string/blank?" => self.blank_string(&args[0]),
            "clojure.string/join" => self.join_string(args.last().unwrap()),
            "clojure.string/replace" => self.replace_string(args, span),
            "clojure.core/re-pattern" => {
                self.expect(
                    &args[0].0,
                    &Type::union([Type::String, Type::Pattern]),
                    args[0].1,
                    name,
                );
                if args[0].0.known == Type::Pattern {
                    return args[0].0.clone();
                }
                Type::Pattern
            }
            "clojure.string/split" => {
                self.expect(
                    &args[0].0,
                    &Type::String,
                    args[0].1,
                    "Argument 1 of clojure.string/split",
                );
                self.expect(
                    &args[1].0,
                    &Type::Pattern,
                    args[1].1,
                    "Argument 2 of clojure.string/split",
                );
                if let Some((limit, span)) = args.get(2) {
                    self.expect(
                        limit,
                        &Type::Number,
                        *span,
                        "Argument 3 of clojure.string/split",
                    );
                }
                Type::Vector(Box::new(Type::String))
            }
            "clojure.core/str" => self.str_result(args),
            "clojure.core/name" if self.globals.project_namespaces.contains("clojure.core") => {
                self.may_throw();
                return self.unsupported(span, "name requires the standard clojure.core function");
            }
            "clojure.core/name" => self.name_result(&args[0].0.known),
            "clojure.core/map?" if self.globals.project_namespaces.contains("clojure.core") => {
                self.may_throw();
                return self.unsupported(span, "map? requires the standard clojure.core predicate");
            }
            "clojure.core/map?" => super::maps::map_predicate(&args[0].0.known),
            "clojure.core/vector?" if self.globals.project_namespaces.contains("clojure.core") => {
                self.may_throw();
                return self
                    .unsupported(span, "vector? requires the standard clojure.core predicate");
            }
            "clojure.core/vector?" if args[0].0.is_literal_map() => Type::Bool(false),
            "clojure.core/vector?" => vector_predicate(&args[0].0.known),
            "clojure.core/identity" => return args[0].0.clone(),
            "clojure.core/any?" => Type::Bool(true),
            "clojure.core/key" | "clojure.core/val" => {
                let (entry, span) = &args[0];
                let index = usize::from(name == "clojure.core/val");
                if let Some(item) = entry.known.entry_item(index) {
                    return entry.position_value(item, index);
                }
                let mut diagnostic =
                    Diagnostic::type_mismatch(*span, name, "MapEntry".into(), entry.ty.to_string());
                diagnostic.notes = entry.notes.clone();
                self.diagnostics.push(diagnostic);
                Type::Unknown
            }
            "clojure.core/count" => return self.count(&args[0]),
            "clojure.core/nil?" => {
                if args[0].0.ty == Type::Nil {
                    Type::Bool(true)
                } else if nil_part(&args[0].0.ty) == Type::Never {
                    Type::Bool(false)
                } else {
                    Type::Boolean
                }
            }
            "clojure.core/some?" => {
                if args[0].0.ty == Type::Nil {
                    Type::Bool(false)
                } else if nil_part(&args[0].0.ty) == Type::Never {
                    Type::Bool(true)
                } else {
                    Type::Boolean
                }
            }
            // These predicates test JVM object identity with the canonical Booleans.
            "clojure.core/true?" | "clojure.core/false?" => Type::Boolean,
            "clojure.core/boolean" => boolean_cast_result(&args[0].0.known),
            "clojure.core/not" => {
                if args[0].0.ty.truthy() == Type::Never {
                    Type::Bool(true)
                } else if args[0].0.ty.falsey() == Type::Never {
                    Type::Bool(false)
                } else {
                    Type::Boolean
                }
            }
            _ => predicate_type(name).map_or(Type::Boolean, |expected| {
                predicate_result(&args[0].0.known, &expected)
            }),
        };
        Value::new(ty)
    }
}

fn unconditional_core_inliner(name: &str) -> bool {
    // The pinned JVM Vars have :inline and no :inline-arities predicate.
    matches!(
        name,
        "clojure.core/inc"
            | "clojure.core/dec"
            | "clojure.core/long"
            | "clojure.core/nil?"
            | "clojure.core/boolean"
            | "clojure.core/count"
    )
}

fn builtin_arities(name: &str) -> &'static [Arity] {
    match name {
        "clojure.core/+" | "clojure.core/*" | "clojure.core/str" | "clojure.core/conj"
        | "clojure.core/list" | "clojure.core/merge" => &[Arity::Variadic(0)],
        "clojure.core/-"
        | "clojure.core/="
        | "clojure.core/disj"
        | "clojure.core/dissoc"
        | "clojure.core/min"
        | "clojure.core/max" => &[Arity::Variadic(1)],
        "clojure.core/nth"
        | "clojure.core/subvec"
        | "clojure.core/ex-info"
        | "clojure.string/split" => &[Arity::Fixed(2), Arity::Fixed(3)],
        "clojure.string/replace" => &[Arity::Fixed(3)],
        "clojure.string/join" => &[Arity::Fixed(1), Arity::Fixed(2)],
        "clojure.core/cons"
        | "clojure.lang.MapEntry/create"
        | "clojure.core/find"
        | "clojure.core/select-keys"
        | "clojure.core/zipmap" => &[Arity::Fixed(2)],
        _ => &[Arity::Fixed(1)],
    }
}

pub(super) fn predicate_type(name: &str) -> Option<Type> {
    match name {
        "clojure.core/string?" => Some(Type::String),
        "clojure.core/char?" => Some(Type::Character),
        "clojure.core/number?" => Some(Type::Number),
        "clojure.core/integer?" => Some(Type::Integral(IntegerType::ANY)),
        "clojure.core/int?" => Some(Type::Integral(IntegerType::FIXED)),
        "clojure.core/boolean?" => Some(Type::Boolean),
        "clojure.core/keyword?" => Some(Type::Keyword),
        "clojure.core/symbol?" => Some(Type::Symbol),
        "clojure.core/simple-symbol?" => Some(Type::SimpleSymbol),
        "clojure.core/qualified-symbol?" => Some(Type::QualifiedSymbol),
        _ => None,
    }
}

pub(super) fn call_returns(args: &[(Value, Span)]) -> bool {
    args.iter().all(|(value, _)| value.known != Type::Never)
}

fn equality(args: &[(Value, Span)]) -> Type {
    let mut result = Type::Bool(true);
    for pair in args.windows(2) {
        match literal_equality(&pair[0].0.known, &pair[1].0.known) {
            Some(false) => return Type::Bool(false),
            Some(true) => {}
            None => result = Type::Boolean,
        }
    }
    result
}

pub(super) fn literal_equality(left: &Type, right: &Type) -> Option<bool> {
    fn alternatives(ty: &Type) -> &[Type] {
        match ty {
            Type::Union(parts) => parts,
            ty => std::slice::from_ref(ty),
        }
    }

    fn is_atom(ty: &Type) -> bool {
        matches!(
            ty,
            Type::Nil | Type::Bool(_) | Type::Key(_) | Type::Str(_) | Type::Char(_) | Type::Sym(_)
        ) || ty.exact_long_value().is_some()
    }

    let (left, right) = (alternatives(left), alternatives(right));
    if left.len().saturating_mul(right.len()) > 64 {
        return None;
    }
    let mut known = None;
    for a in left {
        for b in right {
            let equal = match (a.exact_long_value(), b.exact_long_value()) {
                (Some(a), Some(b)) => a == b,
                _ if is_atom(a) && is_atom(b) => a == b,
                _ => return None,
            };
            if known.is_some_and(|previous| previous != equal) {
                return None;
            }
            known = Some(equal);
        }
    }
    known
}

fn boolean_cast_result(actual: &Type) -> Type {
    match actual {
        Type::Nil | Type::Bool(false) => Type::Bool(false),
        Type::Never => Type::Never,
        // A separately boxed false is truthy in if, but boolean unboxes it.
        Type::Unknown | Type::Any | Type::SpecValue | Type::SpecTruthy | Type::Boolean => {
            Type::Boolean
        }
        Type::Parameter(parameter) => parameter
            .upper
            .as_ref()
            .map_or(Type::Boolean, boolean_cast_result),
        Type::Union(parts) => Type::union(parts.iter().map(boolean_cast_result)),
        _ => Type::Bool(true),
    }
}

fn predicate_result(actual: &Type, expected: &Type) -> Type {
    match actual {
        Type::Unknown => Type::Boolean,
        Type::Parameter(parameter) => parameter
            .upper
            .as_ref()
            .map_or(Type::Boolean, |upper| predicate_result(upper, expected)),
        Type::Never => Type::Never,
        Type::Union(parts) => {
            Type::union(parts.iter().map(|part| predicate_result(part, expected)))
        }
        Type::Symbol if matches!(expected, Type::SimpleSymbol | Type::QualifiedSymbol) => {
            Type::Boolean
        }
        actual => match actual.compatibility(expected) {
            Compatibility::Proven => Type::Bool(true),
            Compatibility::Mismatch => Type::Bool(false),
            Compatibility::Unproven => Type::Boolean,
        },
    }
}

pub(super) fn is_modeled_call(name: &str) -> bool {
    is_builtin(name) || CoreForm::resolve(name).is_some()
}

pub(super) fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "clojure.lang.MapEntry/create"
            | "clojure.core/+"
            | "clojure.core/-"
            | "clojure.core/*"
            | "clojure.core/inc"
            | "clojure.core/dec"
            | "clojure.core/min"
            | "clojure.core/max"
            | "clojure.core/long"
            | "clojure.core/str"
            | "clojure.core/name"
            | "clojure.core/map?"
            | "clojure.core/vector?"
            | "clojure.core/identity"
            | "clojure.core/any?"
            | "clojure.core/nil?"
            | "clojure.core/some?"
            | "clojure.core/true?"
            | "clojure.core/false?"
            | "clojure.core/boolean"
            | "clojure.core/not"
            | "clojure.core/="
            | "clojure.core/key"
            | "clojure.core/val"
            | "clojure.core/count"
            | "clojure.core/empty?"
            | "clojure.core/not-empty"
            | "clojure.core/nth"
            | "clojure.core/subvec"
            | "clojure.core/seq"
            | "clojure.core/first"
            | "clojure.core/next"
            | "clojure.core/rest"
            | "clojure.core/keys"
            | "clojure.core/vals"
            | "clojure.core/conj"
            | "clojure.core/peek"
            | "clojure.core/set"
            | "clojure.core/merge"
            | "clojure.core/disj"
            | "clojure.core/dissoc"
            | "clojure.core/find"
            | "clojure.core/select-keys"
            | "clojure.core/zipmap"
            | "clojure.core/list"
            | "clojure.core/cons"
            | "clojure.core/reduced"
            | "clojure.core/reduced?"
            | "clojure.core/unreduced"
            | "clojure.core/ensure-reduced"
            | "clojure.core/ex-info"
            | "clojure.core/ex-data"
            | "clojure.core/re-pattern"
            | "clojure.string/upper-case"
            | "clojure.string/lower-case"
            | "clojure.string/capitalize"
            | "clojure.string/trim"
            | "clojure.string/blank?"
            | "clojure.string/replace"
            | "clojure.string/split"
            | "clojure.string/join"
    ) || predicate_type(name).is_some()
}

pub(super) fn arity_counts(counts: impl Iterator<Item = Arity>) -> String {
    let counts: Vec<_> = counts.collect();
    let minimum = counts.last().and_then(|arity| match arity {
        Arity::Variadic(minimum) => Some(*minimum),
        Arity::Fixed(_) => None,
    });
    counts
        .into_iter()
        .filter(|arity| !matches!(arity, Arity::Fixed(count) if Some(*count) == minimum))
        .map(|count| count.to_string())
        .collect::<Vec<_>>()
        .join(" or ")
}
