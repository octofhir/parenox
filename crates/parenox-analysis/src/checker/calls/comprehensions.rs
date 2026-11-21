//! Sequential bindings and evaluation rules for for and doseq.
use super::super::{Checker, Env, Value};
use super::sequences::iteration_may_throw;
use super::vectors::element_type;
use crate::reader::{Form, Span};
use crate::types::Type;

enum Binding<'a> {
    Source(&'a Form, &'a Form),
    Let(&'a [[Form; 2]]),
    Test(&'a Form),
}

impl Checker<'_> {
    pub(super) fn comprehension(
        &mut self,
        args: &[Form],
        span: Span,
        env: &Env,
        lazy: bool,
        tail: bool,
    ) -> Value {
        let name = if lazy { "for" } else { "doseq" };
        let Some((vector, body)) = args.split_first() else {
            return self.invalid_comprehension(span, name, "requires a binding vector");
        };
        if lazy && body.len() != 1 {
            return self.invalid_comprehension(span, name, "requires exactly one body expression");
        }
        let Some(bindings) = self.comprehension_bindings(vector, name) else {
            return Value::new(Type::Unknown);
        };
        if lazy {
            let Some(Binding::Source(_, outer)) = bindings.first() else {
                return self.invalid_comprehension(
                    vector.span,
                    name,
                    "must start with a source binding",
                );
            };
            // JVM for evaluates the first source expression when constructing
            // the lazy seq. Even seq'ing that source happens only on realization.
            let first = self.infer(outer, env);
            let constructed = first.known != Type::Never;
            let enclosing = self.throws.take();
            let (item, may_skip) =
                self.comprehension_body(&bindings, body, env, name, Some(first), false);
            self.throws = enclosing;
            if !constructed {
                return Value::new(Type::Never);
            }
            let no_items = item.known == Type::Never;
            let ty = Type::Seq(Box::new(item.known));
            // Keep the lazy Seq variant even if there are no normal item values:
            // forcing it can still throw before yielding anything.
            let ty = if may_skip || no_items {
                Type::union([Type::empty_seq(), ty])
            } else {
                ty
            };
            Value::with_notes(ty, item.notes)
        } else {
            // Without iteration or a guard, the JVM expansion is just
            // sequential let bindings around (do body...).
            let only_let = bindings
                .iter()
                .all(|binding| matches!(binding, Binding::Let(_)));
            let (value, may_skip) =
                self.comprehension_body(&bindings, body, env, name, None, only_let && tail);
            if only_let {
                return value;
            }
            Value::new(if may_skip || value.known != Type::Never {
                Type::Nil
            } else {
                Type::Never
            })
        }
    }

    fn comprehension_bindings<'a>(
        &mut self,
        vector: &'a Form,
        name: &str,
    ) -> Option<Vec<Binding<'a>>> {
        let Some(bindings) = vector.vector() else {
            self.invalid_comprehension(vector.span, name, "requires a binding vector");
            return None;
        };
        let (pairs, []) = bindings.as_chunks::<2>() else {
            self.invalid_comprehension(vector.span, name, "requires binding/value pairs");
            return None;
        };
        pairs
            .iter()
            .map(|[binding, init]| {
                Some(match binding.keyword() {
                    Some(":let") => {
                        if let Some(pairs) =
                            init.vector()
                                .and_then(|forms| match forms.as_chunks::<2>() {
                                    (pairs, [])
                                        if pairs
                                            .iter()
                                            .all(|[pattern, _]| binding_pattern(pattern)) =>
                                    {
                                        Some(pairs)
                                    }
                                    _ => None,
                                })
                        {
                            Binding::Let(pairs)
                        } else {
                            self.invalid_comprehension(
                                init.span,
                                name,
                                ":let requires a vector of binding/value pairs",
                            );
                            return None;
                        }
                    }
                    Some(":when" | ":while") => Binding::Test(init),
                    Some(key) => {
                        self.invalid_comprehension(
                            binding.span,
                            name,
                            &format!("does not support modifier {key}"),
                        );
                        return None;
                    }
                    None if binding_pattern(binding) => Binding::Source(binding, init),
                    None => {
                        self.invalid_comprehension(
                            binding.span,
                            name,
                            "requires a local binding pattern",
                        );
                        return None;
                    }
                })
            })
            .collect()
    }

    fn comprehension_body(
        &mut self,
        bindings: &[Binding<'_>],
        body: &[Form],
        env: &Env,
        name: &str,
        mut first: Option<Value>,
        tail: bool,
    ) -> (Value, bool) {
        let mut local = env.clone();
        let mut reachable = true;
        let mut may_skip = false;
        for binding in bindings {
            match binding {
                Binding::Source(pattern, init) => {
                    let input = first
                        .take()
                        .unwrap_or_else(|| self.infer_path(init, &local, reachable, false));
                    let invoked = reachable && input.known != Type::Never;
                    let start = self.diagnostics.len();
                    let sequence = self.inspect_path(invoked, |checker| {
                        if input.unproven_tuple_traversal() {
                            checker
                                .unchecked_tuple_traversal(name, init.span)
                                .known
                                .nilable()
                        } else {
                            checker.comprehension_seq(&input.known, init.span, name)
                        }
                    });
                    self.dedup_diagnostics(start);
                    may_skip |= invoked && sequence.without_nil() != sequence;
                    let item = element_type(sequence).unwrap_or_else(|_| {
                        unreachable!("seq_type returns a sequence, nil, never or unknown")
                    });
                    reachable = invoked && item != Type::Never;
                    self.inspect_path(reachable, |checker| {
                        checker.bind(pattern, input.traversal_element(item), &mut local);
                    });
                }
                Binding::Let(pairs) => {
                    for [pattern, init] in *pairs {
                        reachable = self.let_binding(pattern, init, &mut local, reachable);
                    }
                }
                Binding::Test(form) => {
                    let guard = self.infer_path(form, &local, reachable, false);
                    may_skip |= guard.known.falsey() != Type::Never;
                    self.narrow(form, true, &mut local);
                    reachable = guard.known.truthy() != Type::Never && local.reachable();
                }
            }
        }
        (
            self.check_path(reachable, |checker| checker.body(body, &local, tail)),
            may_skip,
        )
    }

    fn comprehension_seq(&mut self, ty: &Type, span: Span, name: &str) -> Type {
        match ty {
            Type::Parameter(parameter) if let Some(upper) = &parameter.upper => {
                self.comprehension_seq(upper, span, name)
            }
            Type::Any | Type::Unknown | Type::SpecValue | Type::SpecTruthy | Type::Parameter(_) => {
                self.error(
                    "PNX9004",
                    span,
                    format!("{name}: cannot verify that the source is seqable"),
                );
                self.may_throw();
                Type::Unknown.nilable()
            }
            Type::Union(parts) => Type::union(
                parts
                    .iter()
                    .map(|part| self.comprehension_seq(part, span, name)),
            ),
            _ => {
                if iteration_may_throw(ty) {
                    self.may_throw();
                }
                let sequence = self.seq_type(ty, name, span);
                if sequence == Type::Unknown {
                    sequence.nilable()
                } else {
                    sequence
                }
            }
        }
    }

    fn invalid_comprehension(&mut self, span: Span, name: &str, problem: &str) -> Value {
        self.error("PNX1001", span, format!("{name} {problem}"));
        Value::new(Type::Unknown)
    }
}

fn binding_pattern(form: &Form) -> bool {
    form.symbol()
        .is_some_and(|name| name != "&" && (name == "/" || !name.contains('/')))
        || form.vector().is_some()
        || form.map().is_some()
}
