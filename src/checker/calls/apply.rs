//! Calls whose final input supplies zero or more positional arguments.
use super::{
    Callable, Checker, Env, Value, call_returns, inline::InlineCallable, vectors::CollectionError,
    vectors::element_type,
};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

const MAX_SPREAD_ALTERNATIVES: usize = 16;

impl Checker<'_> {
    pub(super) fn apply_form(&mut self, forms: &[Form], span: Span, env: &Env) -> Value {
        let [callee, prefix @ .., spread] = forms else {
            self.error(
                "PNX2554",
                span,
                "apply expects a function and a final collection",
            );
            return Value::new(Type::Unknown);
        };
        let mut callable = self.prepare_callable(callee, env);
        if matches!(callable, Callable::Inline(InlineCallable::Fnil(_))) {
            callable = Callable::Value(
                self.unsupported(callee.span, "apply cannot check an inline fnil wrapper yet"),
            );
        }
        let mut args: Vec<_> = self
            .eval_args(&forms[1..], env, callable.returns())
            .collect();
        let (collection, _) = args.pop().unwrap();
        if collection.known == Type::Never || !call_returns(&args) {
            return self.call_prepared(&callable, args, callee.span, span, env, false);
        }
        let types = match &collection.known {
            Type::Union(types) => types.as_slice(),
            ty => std::slice::from_ref(ty),
        };
        if types.len() > MAX_SPREAD_ALTERNATIVES {
            return self.unsupported(spread.span, "apply has too many possible collection shapes");
        }
        let mut result = Value::new(Type::Never);
        for ty in types {
            let elements = match ty {
                Type::Nil => &[][..],
                Type::Tuple(elements) | Type::SeqTuple(elements) => elements,
                Type::MapEntry(elements) => elements.as_slice(),
                Type::Unknown => return Value::new(Type::Unknown),
                ty => {
                    if matches!(element_type(ty.clone()), Err(CollectionError::NotSeqable)) {
                        self.error(
                            "PNX2322",
                            spread.span,
                            format!(
                                "apply expects a seqable final argument, got {}",
                                ty.inferred()
                            ),
                        );
                        self.may_throw();
                        return Value::new(Type::Unknown);
                    }
                    return self.unsupported(
                        spread.span,
                        "apply needs a final collection with a known argument count",
                    );
                }
            };
            let literal = match &spread.without_metadata().kind {
                Kind::Vector(forms) if forms.len() == elements.len() => Some(forms),
                _ => None,
            };
            let mut actual = Vec::with_capacity(prefix.len() + elements.len());
            actual.extend(args.iter().cloned());
            actual.extend(elements.iter().enumerate().map(|(index, ty)| {
                let element_span = literal.map_or(spread.span, |forms| forms[index].span);
                (
                    Value::with_notes(ty.clone(), collection.notes.clone()),
                    element_span,
                )
            }));
            let start = self.diagnostics.len();
            let value = self.call_prepared(&callable, actual, callee.span, span, env, true);
            if literal.is_none() {
                for diagnostic in &mut self.diagnostics[start..] {
                    if diagnostic.code == "PNX2322" && diagnostic.span == spread.span {
                        diagnostic.suggestions.clear();
                        diagnostic.help = Some(
                            "Check the elements of the final collection against the function's argument types.".into(),
                        );
                    }
                }
            }
            result = result.join(value);
        }
        result
    }
}
