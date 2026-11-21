//! Names of core forms with their own evaluation rules.
use super::{Conditional, search::Search};
use crate::checker::function_forms::fn_literal_binding_error;
use crate::checker::{Checker, Env, Value};
use crate::expand::Threading;
use crate::reader::{Form, Span};
use crate::types::Type;

pub(super) enum CoreForm {
    Thread(Threading),
    NamedThread,
    Let,
    LetFn,
    Loop,
    Comprehension { lazy: bool },
    Cond,
    Condp,
    Case,
    If(Conditional),
    BindingConditional,
    ShortCircuit(bool),
    Lookup,
    Contains,
    GetIn,
    Assoc,
    AssocIn,
    Fn,
    Fnil,
    Update,
    MapTransform,
    GroupBy,
    MergeWith,
    Apply,
    Reduce,
    ReduceKv,
    PredicateSearch(Search),
    VectorTransform,
    RegexMatch,
    Substring,
}

impl CoreForm {
    pub(super) fn resolve(name: &str) -> Option<Self> {
        Some(match name.strip_prefix("clojure.core/")? {
            "as->" => Self::NamedThread,
            "let" => Self::Let,
            "letfn" => Self::LetFn,
            "loop" => Self::Loop,
            "for" => Self::Comprehension { lazy: true },
            "doseq" => Self::Comprehension { lazy: false },
            "cond" => Self::Cond,
            "condp" => Self::Condp,
            "case" => Self::Case,
            "when" => Self::If(Conditional::When),
            "if-not" => Self::If(Conditional::IfNot),
            "when-not" => Self::If(Conditional::WhenNot),
            "if-let" | "when-let" | "if-some" | "when-some" | "when-first" => {
                Self::BindingConditional
            }
            "or" => Self::ShortCircuit(true),
            "and" => Self::ShortCircuit(false),
            "get" | "req!" => Self::Lookup,
            "contains?" => Self::Contains,
            "get-in" => Self::GetIn,
            "assoc" => Self::Assoc,
            "assoc-in" => Self::AssocIn,
            "fn" => Self::Fn,
            "fnil" => Self::Fnil,
            "update" | "update-in" => Self::Update,
            "update-keys" | "update-vals" => Self::MapTransform,
            "group-by" => Self::GroupBy,
            "merge-with" => Self::MergeWith,
            "apply" => Self::Apply,
            "reduce" => Self::Reduce,
            "reduce-kv" => Self::ReduceKv,
            "some" => Self::PredicateSearch(Search::Some),
            "every?" => Self::PredicateSearch(Search::Every),
            "mapv" | "filterv" => Self::VectorTransform,
            "re-find" | "re-matches" => Self::RegexMatch,
            "subs" => Self::Substring,
            _ => return Threading::from_core_symbol(name).map(Self::Thread),
        })
    }
}

impl Checker<'_> {
    pub(super) fn core_form(
        &mut self,
        form: CoreForm,
        name: &str,
        xs: &[Form],
        span: Span,
        env: &Env,
        tail: bool,
    ) -> Value {
        match form {
            CoreForm::Thread(threading) => self.thread_form(xs, span, env, threading, name, tail),
            CoreForm::NamedThread => self.named_thread(xs, span, env, tail),
            CoreForm::Let => self.let_form(xs, span, env, tail, false),
            CoreForm::LetFn => {
                if self
                    .model
                    .can_use_standard_core(xs[0].symbol().unwrap(), self.globals)
                {
                    self.letfn_form(&xs[1..], span, env, tail)
                } else {
                    self.unchecked_form(xs[0].span, "Standard clojure.core/letfn is unconfirmed")
                }
            }
            CoreForm::Loop => self.loop_form(xs, span, env, false),
            CoreForm::Comprehension { lazy } => self.comprehension(&xs[1..], span, env, lazy, tail),
            CoreForm::Cond => self.cond_form(&xs[1..], span, env, tail),
            CoreForm::Condp => self.condp_form(xs, span, env, tail),
            CoreForm::Case => self.case_form(&xs[1..], span, env, tail),
            CoreForm::If(kind) => self.if_form(xs, span, env, kind, tail),
            CoreForm::BindingConditional => self.binding_conditional(xs, span, env, name, tail),
            CoreForm::ShortCircuit(or) => self.short_circuit(xs, env, or, tail),
            CoreForm::Lookup => self.lookup_form(xs, span, env, name),
            CoreForm::Contains => self.contains(xs, span, env),
            CoreForm::GetIn => self.get_in(xs, span, env),
            CoreForm::Assoc => self.assoc(xs, span, env),
            CoreForm::AssocIn => self.assoc_in(xs, span, env),
            CoreForm::Fn | CoreForm::Fnil => {
                self.withhold_captured_facts(env.inline_functions());
                if matches!(form, CoreForm::Fn)
                    && self
                        .model
                        .can_use_standard_core(xs[0].symbol().unwrap(), self.globals)
                    && let Some(diagnostic) = fn_literal_binding_error(&xs[1..], span)
                {
                    self.type_query = None;
                    self.diagnostics.push(diagnostic);
                    return Value::new(Type::Unknown);
                }
                self.unsupported(
                    span,
                    format!(
                        "{} is currently supported in direct calls and inline callbacks",
                        &name["clojure.core/".len()..]
                    ),
                )
            }
            CoreForm::Update => self.update(xs, span, env, name),
            CoreForm::MapTransform => self.map_transform(&xs[1..], span, env, name),
            CoreForm::GroupBy => self.group_by(&xs[1..], span, env),
            CoreForm::MergeWith => self.merge_with(&xs[1..], span, env),
            CoreForm::Apply => self.apply_form(xs, span, env),
            CoreForm::Reduce => self.reduce_form(&xs[1..], span, env),
            CoreForm::ReduceKv => self.reduce_kv_form(&xs[1..], span, env),
            CoreForm::PredicateSearch(search) => {
                if self
                    .model
                    .can_use_standard_core(xs[0].symbol().unwrap(), self.globals)
                {
                    self.search_form(search, &xs[1..], span, env)
                } else {
                    self.unchecked_form(xs[0].span, &format!("Standard {name} is unconfirmed"))
                }
            }
            CoreForm::VectorTransform => self.vector_transform(xs, span, env, name),
            CoreForm::RegexMatch => self.regex_match(&xs[1..], span, env, name),
            CoreForm::Substring => {
                let stock = self
                    .model
                    .can_use_standard_core(xs[0].symbol().unwrap(), self.globals);
                self.substring(&xs[1..], span, env, stock)
            }
        }
    }
}
