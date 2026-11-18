//! Names of core forms with their own evaluation rules.
use super::Conditional;
use crate::checker::{Checker, Env, Value};
use crate::expand::Threading;
use crate::reader::{Form, Span};

pub(super) enum CoreForm {
    Thread(Threading),
    NamedThread,
    Let,
    Loop,
    Cond,
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
    Apply,
    Reduce,
    ReduceKv,
    VectorTransform,
}

impl CoreForm {
    pub(super) fn resolve(name: &str) -> Option<Self> {
        Some(match name.strip_prefix("clojure.core/")? {
            "as->" => Self::NamedThread,
            "let" => Self::Let,
            "loop" => Self::Loop,
            "cond" => Self::Cond,
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
            "apply" => Self::Apply,
            "reduce" => Self::Reduce,
            "reduce-kv" => Self::ReduceKv,
            "mapv" | "filterv" => Self::VectorTransform,
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
            CoreForm::Let => self.let_form(xs, span, env, tail),
            CoreForm::Loop => self.loop_form(xs, span, env, false),
            CoreForm::Cond => self.cond_form(&xs[1..], span, env, tail),
            CoreForm::Case => self.case_form(&xs[1..], span, env, tail),
            CoreForm::If(kind) => self.if_form(xs, span, env, kind, tail),
            CoreForm::BindingConditional => self.binding_conditional(xs, span, env, name, tail),
            CoreForm::ShortCircuit(or) => self.short_circuit(xs, env, or, tail),
            CoreForm::Lookup => self.lookup_form(xs, span, env, name),
            CoreForm::Contains => self.contains(xs, span, env),
            CoreForm::GetIn => self.get_in(xs, span, env),
            CoreForm::Assoc => self.assoc(xs, span, env),
            CoreForm::AssocIn => self.assoc_in(xs, span, env),
            CoreForm::Fn | CoreForm::Fnil => self.unsupported(
                span,
                format!(
                    "{} is currently supported in direct calls and inline callbacks",
                    &name["clojure.core/".len()..]
                ),
            ),
            CoreForm::Update => self.update(xs, span, env, name),
            CoreForm::MapTransform => self.map_transform(&xs[1..], span, env, name),
            CoreForm::GroupBy => self.group_by(&xs[1..], span, env),
            CoreForm::Apply => self.apply_form(&xs[1..], span, env),
            CoreForm::Reduce => self.reduce_form(&xs[1..], span, env),
            CoreForm::ReduceKv => self.reduce_kv_form(&xs[1..], span, env),
            CoreForm::VectorTransform => self.vector_transform(xs, span, env, name),
        }
    }
}
