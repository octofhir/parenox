//! Input contracts for sequential spec checks. A conformed value is not the input.

use super::{
    TypeReader,
    conversion::{resolves, source_conversion},
    predicate_type,
};
use crate::Diagnostic;
use crate::reader::{Form, Kind};
use crate::types::{IntegerClass, Type};

impl<F: FnMut(&str, crate::reader::Span) -> Option<Type>> TypeReader<'_, F> {
    pub(super) fn conjunction(&mut self, form: &Form, stages: &[Form]) -> Type {
        if let Some(conversion) = source_conversion(self.names, form) {
            // Type positions refer to the original value, and accept only
            // inputs that can conform without throwing. The runtime
            // predicate domain and converted output are tracked separately.
            return conversion.success_input();
        }
        if let [predicate, set] = stages
            && let Some(domain) = self.guarded_integer_set(predicate, set)
        {
            return domain;
        }
        let [first, rest @ ..] = stages else {
            return self.conjunction_gap(form, "An empty s/and has no modeled input type");
        };
        let mut input = self.conjunction_stage(first);
        if let [last] = rest
            && self.is_tagged_choice(first)
            && self.is_tag_projection(last)
        {
            // A successful s/or returns [tag value]. Projecting value cannot
            // reject an input accepted by the choice, even if a branch conformed it.
            return input;
        }
        let mut same_value = self.preserves_input(first);
        for stage in rest {
            let next = self.conjunction_stage(stage);
            if !input.contains_unknown() && !next.contains_unknown() {
                input = if same_value {
                    input.known_intersection(&next).unwrap_or_else(|| {
                        self.conjunction_gap(stage, "This s/and type intersection is not modeled")
                    })
                } else {
                    self.conjunction_gap(stage, "This s/and stage checks a conformed value")
                };
            } else {
                input = Type::Unknown;
            }
            same_value &= self.preserves_input(stage);
        }
        input
    }

    fn guarded_integer_set(&self, predicate: &Form, set: &Form) -> Option<Type> {
        let Kind::Set(items) = &set.without_metadata().kind else {
            return None;
        };
        let class = if resolves(self.names, predicate, "clojure.core", "int?") {
            IntegerClass::Fixed
        } else if resolves(self.names, predicate, "clojure.core", "integer?") {
            IntegerClass::Any
        } else {
            return None;
        };
        // A literal set also accepts numerically equal BigInt values. The
        // preceding predicate determines which integer classes can reach it.
        items
            .iter()
            .map(|item| {
                let value = Type::integer_literal(item, self.source)
                    .and_then(|ty| ty.exact_long_value())
                    .or_else(|| Type::decimal_long_literal(item, self.source))?;
                Some(Type::exact_long(value, class))
            })
            .collect::<Option<Vec<_>>>()
            .map(Type::union)
    }

    fn conjunction_stage(&mut self, form: &Form) -> Type {
        match &form.without_metadata().kind {
            Kind::Symbol(s) if predicate_type(self.names, s).is_none() => {
                return self.conjunction_gap(form, "Custom predicate in s/and is unchecked");
            }
            Kind::AnonymousFunction(_) | Kind::Map(_) | Kind::Vector(_) => {
                return self.conjunction_gap(form, "Custom predicate in s/and is unchecked");
            }
            Kind::List(items)
                if items
                    .first()
                    .and_then(Form::symbol)
                    .is_none_or(|op| self.names.resolve_parts(op).0 != "clojure.spec.alpha") =>
            {
                return self.conjunction_gap(form, "Custom predicate in s/and is unchecked");
            }
            Kind::Set(items)
                if self.literal_shapes
                    && items.iter().any(|item| {
                        matches!(item.without_metadata().kind, Kind::Nil | Kind::Bool(false))
                            || item.keyword().is_some_and(|key| {
                                self.names.key(key) == ":clojure.spec.alpha/invalid"
                            })
                    }) =>
            {
                return self.conjunction_gap(
                    form,
                    "Set predicate with falsey or invalid members is unchecked",
                );
            }
            _ => {}
        }
        self.parse(form)
    }

    fn preserves_input(&self, form: &Form) -> bool {
        match &form.without_metadata().kind {
            Kind::Symbol(symbol) => predicate_type(self.names, symbol).is_some(),
            Kind::Set(_) => true,
            Kind::List(items) => {
                let Some((head, args)) = items.split_first() else {
                    return false;
                };
                match head.symbol().map(|op| self.names.resolve_parts(op)) {
                    Some(("clojure.spec.alpha", "and")) => {
                        !args.is_empty() && args.iter().all(|arg| self.preserves_input(arg))
                    }
                    Some(("clojure.spec.alpha", "nilable")) => {
                        matches!(args, [inner] if self.preserves_input(inner))
                    }
                    Some(("clojure.spec.alpha", "with-gen")) => {
                        matches!(args, [inner, _] if self.preserves_input(inner))
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn is_tagged_choice(&self, form: &Form) -> bool {
        matches!(&form.without_metadata().kind, Kind::List(items)
            if items.len() >= 3 && items.len() % 2 == 1
                && resolves(self.names, &items[0], "clojure.spec.alpha", "or")
                && items[1..].as_chunks::<2>().0.iter().all(|[tag, _]| tag.keyword().is_some()))
    }

    fn is_tag_projection(&self, form: &Form) -> bool {
        matches!(&form.without_metadata().kind, Kind::List(items)
            if matches!(items.as_slice(), [head, function] | [head, function, _]
                if resolves(self.names, head, "clojure.spec.alpha", "conformer")
                    && resolves(self.names, function, "clojure.core", "second")))
    }

    pub(super) fn unchecked_conformer(&mut self, form: &Form) -> Type {
        self.conjunction_gap(form, "Input and result of this s/conformer are unchecked")
    }

    fn conjunction_gap(&mut self, form: &Form, reason: &str) -> Type {
        self.diagnostics
            .push(Diagnostic::analysis_gap("PNX9001", form.span, reason));
        Type::Unknown
    }
}
