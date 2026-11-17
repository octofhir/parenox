//! Pure form transformations for supported core threading macros.
//! Names must be resolved by the consumer before selecting a transformation.
use std::collections::BTreeSet;

use crate::reader::{Form, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Threading {
    First,
    Last,
    SomeFirst,
    SomeLast,
    ConditionalFirst,
    ConditionalLast,
}

impl Threading {
    pub fn from_core_symbol(symbol: &str) -> Option<Self> {
        match symbol {
            "clojure.core/->" => Some(Self::First),
            "clojure.core/->>" => Some(Self::Last),
            "clojure.core/some->" => Some(Self::SomeFirst),
            "clojure.core/some->>" => Some(Self::SomeLast),
            "clojure.core/cond->" => Some(Self::ConditionalFirst),
            "clojure.core/cond->>" => Some(Self::ConditionalLast),
            _ => None,
        }
    }

    pub fn is_conditional(self) -> bool {
        matches!(self, Self::ConditionalFirst | Self::ConditionalLast)
    }

    pub fn guards_nil(self) -> bool {
        matches!(self, Self::SomeFirst | Self::SomeLast)
    }

    /// Inserts the input into one step, preserving source spans and step metadata.
    /// The consumer controls evaluation order, nil guards and temporary binding lifetime.
    pub fn apply(self, step: &Form, input: Form) -> Form {
        let kind = match &step.kind {
            Kind::Metadata(metadata, target) => {
                Kind::Metadata(metadata.clone(), Box::new(self.apply(target, input)))
            }
            Kind::List(forms) => {
                let mut call = Vec::with_capacity(forms.len().max(1) + 1);
                let (head, args) = forms
                    .split_first()
                    .map_or((None, &[][..]), |(head, args)| (Some(head), args));
                call.push(head.cloned().unwrap_or(Form {
                    kind: Kind::Nil,
                    span: step.span,
                }));
                if matches!(self, Self::First | Self::SomeFirst | Self::ConditionalFirst) {
                    call.push(input);
                    call.extend_from_slice(args);
                } else {
                    call.extend_from_slice(args);
                    call.push(input);
                }
                Kind::List(call)
            }
            _ => Kind::List(vec![step.clone(), input]),
        };
        Form {
            kind,
            span: step.span,
        }
    }
}

/// A deterministic temporary that cannot capture symbols in these forms or an outer scope.
pub fn temporary(forms: &[Form], is_bound: impl Fn(&str) -> bool) -> String {
    fn symbols<'a>(form: &'a Form, names: &mut BTreeSet<&'a str>) {
        match &form.kind {
            Kind::Symbol(name) | Kind::AnonymousArgument(name) | Kind::Keyword(name) => {
                let name = name.trim_start_matches(':');
                names.insert(name);
                if let Some((_, local)) = name.rsplit_once('/') {
                    names.insert(local);
                }
            }
            Kind::List(forms) | Kind::Vector(forms) | Kind::Map(forms) | Kind::Set(forms) => {
                for form in forms {
                    symbols(form, names);
                }
            }
            Kind::AnonymousFunction(function) => {
                for form in &function.body {
                    symbols(form, names);
                }
            }
            Kind::Quote(form) | Kind::Conditional { form, .. } => symbols(form, names),
            Kind::Metadata(a, b) | Kind::Tagged(a, b) => {
                symbols(a, names);
                symbols(b, names);
            }
            _ => {}
        }
    }
    let mut names = BTreeSet::new();
    for form in forms {
        symbols(form, &mut names);
    }
    for index in 0usize.. {
        let name = format!("parenox__thread_{index}");
        if !names.contains(name.as_str()) && !is_bound(&name) {
            return name;
        }
    }
    unreachable!("Temporary name space exhausted")
}
