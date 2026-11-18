//! Literal dispatch and narrowing for the Clojure case macro.
use std::collections::BTreeSet;
use std::slice;

use super::flow::narrow_by_type;
use super::{Checker, Env, Value};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

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
                        "case analysis supports nil, booleans, characters, strings, keywords and decimal integers in the i32 range");
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
        let mut remaining = subject.known;
        let mut types = Vec::new();
        let mut notes = Vec::new();
        let mut matched = false;
        for (keys, result) in branches {
            let selected =
                Type::union(keys.iter().map(|key| narrow_by_type(&remaining, key, true)));
            for key in &keys {
                remaining = unmatched(&remaining, key);
            }
            matched |= selected != Type::Never;
            let branch = self.case_branch(result, local, selected, env, tail);
            types.push(branch.known);
            notes.extend(branch.notes);
        }
        if let Some(fallback) = fallback.first() {
            let branch = self.case_branch(fallback, local, remaining, env, tail);
            types.push(branch.known);
            notes.extend(branch.notes);
        } else if remaining != Type::Never {
            self.illegal_argument();
            if !matched && remaining != Type::Unknown {
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
            Kind::Number => {
                let ty = self.infer(form, env).known;
                matches!(ty, Type::Integer(_)).then_some(ty)
            }
            _ => None,
        }
    }

    fn case_branch(
        &mut self,
        form: &Form,
        local: Option<&str>,
        ty: Type,
        env: &Env,
        tail: bool,
    ) -> Value {
        let reachable = ty != Type::Never;
        let mut env = env.clone();
        if let Some(value) = local.and_then(|name| env.get_mut(name)) {
            value.narrow(ty);
        }
        self.infer_path(form, &env, reachable, tail)
    }
}

fn unmatched(actual: &Type, key: &Type) -> Type {
    match (actual, key) {
        (Type::Boolean, Type::Bool(test)) => Type::Bool(!test),
        (Type::Union(parts), _) => Type::union(parts.iter().map(|part| unmatched(part, key))),
        _ => narrow_by_type(actual, key, false),
    }
}
