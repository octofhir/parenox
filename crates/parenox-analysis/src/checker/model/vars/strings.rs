//! Inference for bounded calls to standard str in def initializers.
use std::borrow::Cow;
use std::collections::BTreeMap;

use super::super::super::calls::concatenate_literal_strings;
use super::super::{Declarations, Globals, contracts::function_parts, declarations::symbols_match};
use super::{def_initializer, initializers};
use crate::reader::{Form, Kind};
use crate::types::Type;

#[derive(Default)]
pub(in crate::checker::model) struct LiteralStrScope<'a> {
    stable_end: Option<usize>,
    shadows: BTreeMap<&'a str, Option<usize>>,
    pub(in crate::checker::model) macro_inputs: BTreeMap<String, bool>,
}

impl<'a> LiteralStrScope<'a> {
    pub(in crate::checker::model) fn fold(
        &mut self,
        declarations: &Declarations,
        forms: &'a [Form],
        source: &str,
        globals: &Globals,
        value: &'a Form,
    ) -> Option<Form> {
        let (head, operator, arguments) = str_call(declarations, value)?;
        let stable_end = if let Some(end) = self.stable_end {
            end
        } else {
            let end = self.stable_scope_end(declarations, forms, source, globals);
            self.stable_end = Some(end);
            end
        };
        if head.span.start.offset
            >= stable_end.min(self.shadow_start(declarations, forms, operator, "str")?)
        {
            return None;
        }
        let parts = arguments
            .iter()
            .map(|argument| str_argument(declarations, argument))
            .collect::<Option<Vec<_>>>()?;
        let Type::Str(text) = concatenate_literal_strings(parts) else {
            return None;
        };
        Some(Form {
            kind: Kind::String(text),
            span: value.span,
        })
    }

    fn shadow_start(
        &mut self,
        declarations: &Declarations,
        forms: &[Form],
        operator: &'a str,
        name: &str,
    ) -> Option<usize> {
        *self
            .shadows
            .entry(operator)
            .or_insert_with(|| declarations.core_operator_shadow_start(forms, operator, name))
    }

    fn stable_scope_end(
        &mut self,
        declarations: &Declarations,
        forms: &'a [Form],
        source: &str,
        globals: &Globals,
    ) -> usize {
        // Later forms can affect calls compiled after them; earlier roots keep
        // the value they received at load time.
        let limit = declarations.initializer_scope_end(forms);
        let mut preceding_end = 0;
        for form in forms
            .iter()
            .take_while(|form| form.span.start.offset < limit)
        {
            let prefix = &source[preceding_end..form.span.end.offset];
            if prefix.contains("#_")
                || prefix.contains("#=")
                || self.may_change_namespace(declarations, globals, form)
            {
                return form.span.start.offset;
            }
            preceding_end = form.span.end.offset;
            let Kind::List(items) = &form.without_metadata().kind else {
                continue;
            };
            let [head, name, tail @ ..] = items.as_slice() else {
                continue;
            };
            let Some(operator) = head.symbol() else {
                continue;
            };
            if let ("clojure.core", core @ ("defn" | "defn-" | "declare")) =
                declarations.names.resolve_parts(operator)
            {
                let stock_declaration = self
                    .shadow_start(declarations, forms, operator, core)
                    .is_some_and(|end| head.span.start.offset < end);
                if !stock_declaration
                    || !inert_definition_name(name)
                    || (core != "declare"
                        && !function_parts(tail)
                            .1
                            .all(|[key, value]| inert_attribute(key, value)))
                    || (core == "declare" && !tail.iter().all(inert_definition_name))
                {
                    return head.span.start.offset;
                }
                continue;
            }
            if operator != "def" {
                continue;
            }
            if !inert_definition_name(name) {
                return head.span.start.offset;
            }
            let Some(initializer) = def_initializer(tail) else {
                continue;
            };
            let stock_str =
                str_call(declarations, initializer).is_some_and(|(head, operator, _)| {
                    self.shadow_start(declarations, forms, operator, "str")
                        .is_some_and(|end| head.span.start.offset < end)
                });
            if !stock_str
                && !initializers::inert(initializer, false)
                && !platform_constructor(declarations, forms, initializer)
            {
                return initializer.span.start.offset;
            }
        }
        limit
    }

    fn may_change_namespace(
        &mut self,
        declarations: &Declarations,
        globals: &Globals,
        form: &Form,
    ) -> bool {
        !symbols_match(form, &mut |symbol| {
            let (namespace, name) = declarations.names.resolve_parts(symbol);
            match namespace {
                "clojure.core" => !namespace_mutator(name),
                local if local == declarations.namespace() => true,
                _ => {
                    let qualified = format!("{namespace}/{name}");
                    !*self
                        .macro_inputs
                        .entry(qualified.clone())
                        .or_insert_with(|| globals.may_expand_initializer_macro(&qualified))
                }
            }
        })
    }
}

fn inert_definition_name(mut name: &Form) -> bool {
    while let Kind::Metadata(metadata, target) = &name.kind {
        let Kind::Map(entries) = &metadata.kind else {
            return false;
        };
        if !entries
            .as_chunks::<2>()
            .0
            .iter()
            .all(|[key, value]| inert_attribute(key, value))
        {
            return false;
        }
        name = target;
    }
    matches!(name.kind, Kind::Symbol(_))
}

fn inert_attribute(key: &Form, value: &Form) -> bool {
    initializers::inert(key, false)
        && (initializers::inert(value, false)
            || (key.keyword() == Some(":tag") && matches!(value.kind, Kind::Symbol(_))))
}

fn str_call<'a>(
    declarations: &Declarations,
    value: &'a Form,
) -> Option<(&'a Form, &'a str, &'a [Form])> {
    let Kind::List(items) = &value.kind else {
        return None;
    };
    let [head, arguments @ ..] = items.as_slice() else {
        return None;
    };
    let Kind::Symbol(operator) = &head.kind else {
        return None;
    };
    if declarations.names.resolve_parts(operator) != ("clojure.core", "str") {
        return None;
    }
    arguments
        .iter()
        .all(|argument| {
            matches!(
                argument.kind,
                Kind::Nil | Kind::String(_) | Kind::Keyword(_)
            )
        })
        .then_some((head, operator, arguments))
}

fn str_argument<'a>(declarations: &Declarations, argument: &'a Form) -> Option<Cow<'a, str>> {
    match &argument.kind {
        Kind::Nil => Some(Cow::Borrowed("")),
        Kind::String(text) => Some(Cow::Borrowed(text.as_str())),
        Kind::Keyword(keyword) if !keyword.starts_with("::") => Some(Cow::Borrowed(keyword)),
        Kind::Keyword(keyword) => declarations.names.validation_key(keyword).map(Cow::Owned),
        _ => None,
    }
}

fn namespace_mutator(name: &str) -> bool {
    matches!(
        name,
        "alias"
            | "ns-unalias"
            | "refer"
            | "ns-unmap"
            | "alter-var-root"
            | "alter-meta!"
            | "reset-meta!"
            | "intern"
            | "eval"
            | "load"
            | "load-file"
            | "load-string"
            | "in-ns"
            | "require"
            | "use"
    )
}

fn platform_constructor(declarations: &Declarations, forms: &[Form], form: &Form) -> bool {
    let Kind::List(items) = &form.kind else {
        return false;
    };
    // A direct JDK constructor can run in the prefix with literal arguments
    // and static field reads. An arbitrary initializer can rebind a Var.
    let [head, arguments @ ..] = items.as_slice() else {
        return false;
    };
    let Some(operator) = head.symbol() else {
        return false;
    };
    let platform_class = |name: &str| {
        !declarations.names.aliases.contains_key(name)
            && declarations
                .java_class_candidate(name)
                .is_some_and(|class| class.starts_with("java."))
    };
    operator.strip_suffix('.').is_some_and(platform_class)
        && !declarations.names.refers.contains_key(operator)
        && declarations.source_var_before(forms, operator, head.span.start.offset).is_none()
        && arguments.iter().all(|argument| {
            initializers::inert(argument, false)
                || matches!(&argument.kind, Kind::Symbol(name) if name.split_once('/').is_some_and(|(class, _)| platform_class(class)))
        })
}
