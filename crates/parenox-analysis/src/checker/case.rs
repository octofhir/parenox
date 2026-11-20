//! Literal dispatch and narrowing for the Clojure case macro.
use std::collections::BTreeSet;
use std::slice;

use super::flow::{narrow_literal, refine_owner};
use super::{Checker, Env, Value};
use crate::reader::{Form, Kind, Span};
use crate::types::{IntegerClass, Type};

impl Checker<'_> {
    pub(super) fn case_form(&mut self, forms: &[Form], span: Span, env: &Env, tail: bool) -> Value {
        let Some((input, clauses)) = forms.split_first() else {
            self.error("PNX2554", span, "case requires an expression");
            return Value::new(Type::Unknown);
        };
        let subject = self.infer(input, env);
        let (pairs, fallback) = clauses.as_chunks::<2>();
        let mut tests = BTreeSet::new();
        let mut branches = Vec::with_capacity(pairs.len());
        for [constants, result] in pairs {
            let constants = match &constants.without_metadata().kind {
                Kind::List(forms) => forms.as_slice(),
                _ => slice::from_ref(constants),
            };
            let mut keys = Vec::with_capacity(constants.len());
            for constant in constants {
                let Some(key) = self.case_constant(constant, env) else {
                    return self.unsupported(constant.span,
                        "case analysis supports nil, booleans, characters, strings, keywords and integers in the signed long range");
                };
                if !tests.insert(key.clone()) {
                    self.error(
                        "PNX2790",
                        constant.span,
                        format!("Duplicate case test constant {key}"),
                    );
                    return Value::new(Type::Unknown);
                }
                keys.push(key);
            }
            // An empty group contributes no branch to the macro expansion.
            if !keys.is_empty() {
                branches.push((keys, result));
            }
        }
        if !pairs.is_empty() && branches.is_empty() {
            self.error(
                "PNX1001",
                span,
                "case requires a test constant when clauses are supplied",
            );
            return Value::new(Type::Unknown);
        }

        let local = input.symbol();
        let selector = self.selector(input, env);
        let mut remaining = subject.known;
        let mut types = Vec::new();
        let mut notes = Vec::new();
        let mut matched = false;
        for (keys, result) in branches {
            let select =
                |ty: &Type| Type::union(keys.iter().map(|key| narrow_literal(ty, key, true)));
            let selected = select(&remaining);
            for key in &keys {
                remaining = narrow_literal(&remaining, key, false);
            }
            matched |= selected != Type::Never;
            let mut branch_env = env.clone();
            if selected != Type::Never
                && let Some(selector) = &selector
            {
                refine_owner(selector, &mut branch_env, select);
            }
            let branch = self.case_branch(result, local, selected, branch_env, tail);
            types.push(branch.known);
            notes.extend(branch.notes);
        }
        if let Some(fallback) = fallback.first() {
            let mut branch_env = env.clone();
            if remaining != Type::Never
                && let Some(selector) = &selector
            {
                refine_owner(selector, &mut branch_env, |ty| {
                    let mut rest = ty.clone();
                    for key in &tests {
                        rest = narrow_literal(&rest, key, false);
                    }
                    rest
                });
            }
            let branch = self.case_branch(fallback, local, remaining, branch_env, tail);
            types.push(branch.known);
            notes.extend(branch.notes);
        } else if remaining != Type::Never {
            self.illegal_argument();
            if (!matched && remaining != Type::Unknown)
                || (selector.is_some() && finite_tag(&remaining))
            {
                self.error(
                    "PNX2802",
                    input.span,
                    format!("case has no matching clause for {remaining}"),
                );
            }
        }
        Value::with_notes(Type::union(types), notes)
    }

    fn case_constant(&mut self, form: &Form, env: &Env) -> Option<Type> {
        match form.without_metadata().kind {
            Kind::Nil | Kind::Bool(_) | Kind::Character(_) | Kind::String(_) | Kind::Keyword(_) => {
                Some(self.infer(form, env).known)
            }
            // Dispatch compares the integer value. Refinement intersects the
            // label with the subject so its JVM numeric class is retained.
            Kind::Number => self
                .infer(form, env)
                .known
                .exact_long_value()
                .map(|value| Type::exact_long(value, IntegerClass::Any)),
            _ => None,
        }
    }

    fn case_branch(
        &mut self,
        form: &Form,
        local: Option<&str>,
        ty: Type,
        mut env: Env,
        tail: bool,
    ) -> Value {
        let reachable = ty != Type::Never;
        if let Some(value) = local.and_then(|name| env.get_mut(name)) {
            value.narrow(ty);
        }
        self.infer_path(form, &env, reachable, tail)
    }
}

fn finite_tag(ty: &Type) -> bool {
    match ty {
        Type::Union(parts) => parts.iter().all(finite_tag),
        Type::Nil | Type::Boolean | Type::Bool(_) | Type::Key(_) | Type::Str(_) | Type::Char(_) => {
            true
        }
        _ => ty.exact_long_value().is_some(),
    }
}
