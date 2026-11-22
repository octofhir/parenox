//! Inference for bounded literal folds in def initializers.
use std::borrow::Cow;
use std::collections::BTreeMap;

use super::super::super::calls::{concatenate_literal_strings, identifier_name};
use super::super::{Declarations, Globals, contracts::function_parts, declarations::symbols_match};
use super::{CapturedInitializer, def_initializer, initializers, sets};
use crate::reader::{Form, Kind};
use crate::types::Type;

#[derive(Default)]
pub(in crate::checker::model) struct LiteralInitializerScope<'a> {
    source: &'a str,
    stable_limit: Option<usize>,
    scanned_forms: usize,
    blocked: bool,
    shadows: BTreeMap<&'a str, Option<usize>>,
    pub(in crate::checker::model) macro_inputs: BTreeMap<String, bool>,
    pub(in crate::checker::model) class_inputs: BTreeMap<String, bool>,
    pub(in crate::checker::model) set_namespace_in_project: Option<bool>,
}

enum Foldable<'a> {
    Str(&'a [Form]),
    Names(&'a [Form]),
    SetUnion(Vec<&'a Form>),
    PlatformLong(i64),
}

impl<'a> LiteralInitializerScope<'a> {
    pub(in crate::checker::model) fn new(source: &'a str) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }

    pub(in crate::checker::model) fn fold(
        &mut self,
        declarations: &'a Declarations,
        forms: &'a [Form],
        globals: &Globals,
        index: usize,
        value: &'a Form,
        captured: &BTreeMap<usize, CapturedInitializer>,
    ) -> Option<CapturedInitializer> {
        let initializer = self.foldable(declarations, forms, globals, index, value)?;
        let limit = *self
            .stable_limit
            .get_or_insert_with(|| declarations.initializer_scope_end(forms));
        if self.blocked || value.span.start.offset >= limit {
            return None;
        }
        if !self.stable_through(declarations, forms, globals, index + 1, captured) {
            self.blocked = true;
            return None;
        }
        let kind = match initializer {
            Foldable::Str(arguments) => {
                let parts = arguments
                    .iter()
                    .map(|argument| str_argument(declarations, argument))
                    .collect::<Option<Vec<_>>>()?;
                let Type::Str(text) = concatenate_literal_strings(parts) else {
                    return None;
                };
                Kind::String(text)
            }
            Foldable::Names(items) => Kind::Vector(
                items
                    .iter()
                    .map(|item| name_item(declarations, item))
                    .collect::<Option<_>>()?,
            ),
            Foldable::SetUnion(items) => Kind::Quote(Box::new(Form {
                kind: Kind::Set(items.into_iter().cloned().collect()),
                span: value.span,
            })),
            Foldable::PlatformLong(value) => return Some(CapturedInitializer::PlatformLong(value)),
        };
        Some(CapturedInitializer::from_form(Form {
            kind,
            span: value.span,
        }))
    }

    fn foldable(
        &mut self,
        declarations: &'a Declarations,
        forms: &'a [Form],
        globals: &Globals,
        index: usize,
        value: &'a Form,
    ) -> Option<Foldable<'a>> {
        if let Kind::Symbol(symbol) = &value.kind {
            return self
                .platform_long(declarations, globals, symbol)
                .map(Foldable::PlatformLong);
        }
        if let Some((head, operator, arguments)) = str_call(declarations, value) {
            let end = self.shadow_start(declarations, forms, operator, "str")?;
            return (head.span.start.offset < end).then_some(Foldable::Str(arguments));
        }
        let Kind::List(items) = &value.kind else {
            return None;
        };
        if let [head, left, right] = items.as_slice()
            && declarations.names.loads_library("clojure.set")
            && head.symbol().is_some_and(|name| {
                declarations.names.resolve_parts(name) == ("clojure.set", "union")
            })
        {
            let in_project = globals.project_namespaces.contains("clojure.set");
            self.set_namespace_in_project = Some(in_project);
            if in_project {
                return None;
            }
            return sets::union_members(
                declarations,
                forms,
                self.source,
                [left, right],
                value.span.start.offset,
            )
            .map(Foldable::SetUnion);
        }
        let [thread, input, mapper, vector] = items.as_slice() else {
            return None;
        };
        let Kind::List(map_call) = &mapper.kind else {
            return None;
        };
        let [map, name] = map_call.as_slice() else {
            return None;
        };
        for (head, core) in [
            (thread, "->>"),
            (map, "map"),
            (name, "name"),
            (vector, "vec"),
        ] {
            let Kind::Symbol(operator) = &head.kind else {
                return None;
            };
            if declarations.names.resolve_parts(operator) != ("clojure.core", core)
                || head.span.start.offset
                    >= self.shadow_start(declarations, forms, operator, core)?
            {
                return None;
            }
        }
        let input = if let Kind::Symbol(symbol) = &input.kind {
            let (name, root) = initializers::adjacent_literal_def(declarations, forms, index)?;
            (symbol == name).then_some(root)?
        } else {
            input
        };
        let Kind::Vector(items) = &input.kind else {
            return None;
        };
        (input.span.end.offset - input.span.start.offset <= initializers::MAX_SUBSTITUTED_BYTES
            && items
                .iter()
                .all(|item| matches!(item.kind, Kind::String(_) | Kind::Keyword(_))))
        .then_some(Foldable::Names(items))
    }

    fn platform_long(
        &mut self,
        declarations: &Declarations,
        globals: &Globals,
        symbol: &str,
    ) -> Option<i64> {
        let (class, field) = symbol.split_once('/')?;
        let value = match field {
            "MAX_VALUE" => i64::MAX,
            "MIN_VALUE" => i64::MIN,
            _ => return None,
        };
        if declarations.names.aliases.contains_key(class)
            || declarations.names.directly_loads(class)
            || declarations.names.loads_library(class)
            || declarations.java_class_candidate(class) != Some("java.lang.Long")
        {
            return None;
        }
        let competing = globals.has_competing_class_namespace(class);
        self.class_inputs.insert(class.into(), competing);
        (!competing).then_some(value)
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

    fn stable_through(
        &mut self,
        declarations: &'a Declarations,
        forms: &'a [Form],
        globals: &Globals,
        end: usize,
        captured: &BTreeMap<usize, CapturedInitializer>,
    ) -> bool {
        // Scan through the requested initializer after earlier roots have
        // been captured, so a proven alias is available at its load position.
        let unchecked = declarations.first_unchecked_dispatch();
        for (index, form) in forms.iter().enumerate().take(end).skip(self.scanned_forms) {
            if unchecked.is_some_and(|offset| offset < form.span.end.offset) {
                return false;
            }
            if declarations
                .discarded_comments
                .binary_search(&index)
                .is_ok()
            {
                continue;
            }
            if self.may_change_namespace(declarations, globals, form) {
                return false;
            }
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
                    return false;
                }
                continue;
            }
            if operator != "def" {
                continue;
            }
            if !inert_definition_name(name) {
                return false;
            }
            let Some(initializer) = def_initializer(tail) else {
                continue;
            };
            let captured_read =
                matches!(initializer.kind, Kind::Symbol(_)) && captured.contains_key(&index);
            if !initializers::inert(initializer, false)
                && !captured_read
                && self
                    .foldable(declarations, forms, globals, index, initializer)
                    .is_none()
                && !platform_constructor(declarations, forms, initializer)
            {
                return false;
            }
        }
        self.scanned_forms = end;
        true
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

fn name_item(declarations: &Declarations, item: &Form) -> Option<Form> {
    let text = match &item.kind {
        Kind::String(text) => text.clone(),
        Kind::Keyword(keyword) if keyword.starts_with("::") => {
            identifier_name(&declarations.names.validation_key(keyword)?).to_owned()
        }
        Kind::Keyword(keyword) => identifier_name(keyword).to_owned(),
        _ => return None,
    };
    Some(Form {
        kind: Kind::String(text),
        span: item.span,
    })
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
