//! Attach source namespace context to names the frontend left unresolved.
use std::collections::BTreeMap;

use super::super::vars::{def_initializer, inert};
use super::{Declarations, Names};
use crate::checker::quote::quoted_symbol;
use crate::reader::{
    CheckRead, Form, Kind, NamespaceToken, Position, ReadError, Span,
    check_resolved_collection_with_namespaces, qualify_namespaced_entries_at,
};

struct LiteralError {
    read_end: usize,
    error: ReadError,
}

struct ReaderNamespaces<'a> {
    declarations: &'a Declarations,
    scopes: Vec<ReaderNamespace>,
    namespaces: Vec<NamespaceAliases>,
    prelude: ReaderPrelude,
}

type AliasBindings = BTreeMap<String, (usize, String)>;

#[derive(Default)]
struct ReaderPrelude {
    aliases: AliasBindings,
    expires: Option<usize>,
    entered: Option<EnteredNamespace>,
    unchecked_load: Option<usize>,
}

struct EnteredNamespace {
    since: usize,
    through: Option<usize>,
    name: String,
}

struct ReaderNamespace {
    end: usize,
    namespace: usize,
}

struct NamespaceAliases {
    name: String,
    aliases: AliasBindings,
}

impl Declarations {
    pub(crate) fn reader_namespace_before(
        &self,
        forms: &[Form],
        offset: usize,
    ) -> Option<(&str, Span)> {
        self.namespace_read_boundary()
            .filter(|end| *end <= offset)?;
        Some((self.namespace(), self.namespace_selection(forms)?))
    }

    pub(crate) fn read_source(source: &str, read: &mut CheckRead) -> Result<Self, ReadError> {
        let mut declarations = Self::read_source_declarations(read);
        let count = read.forms.len();
        declarations.resolve_reader_prefix(source, read, count, usize::MAX)?;
        if read
            .namespace_tokens
            .iter()
            .any(|token| matches!(token, NamespaceToken::Map { .. }))
        {
            declarations = Self::read_source_declarations(read);
        }
        // References can occur in quoted data and namespace renames. Check
        // the selected forms only when the spelling occurs in the source.
        declarations.refers_to_assert_expr =
            source.contains("assert-expr") && declarations.references_assert_expr(&read.forms);
        Ok(declarations)
    }

    /// Expose compiler-level do bodies as top-level forms after reader validation.
    pub(crate) fn expand_top_level_do(&mut self, read: &mut CheckRead) {
        let can_expand = |form: &Form| {
            top_level_do(form).is_some() && !changes_reader_namespace(form, &self.names)
        };
        if !read.forms.iter().any(can_expand) {
            return;
        }
        let mut expanded = Vec::with_capacity(read.forms.len());
        let boundary = self.completed_before_last_tag();
        let mut expanded_boundary = 0;
        for (index, form) in std::mem::take(&mut read.forms).into_iter().enumerate() {
            if can_expand(&form) {
                append_top_level_do(form, &mut expanded);
            } else {
                expanded.push(form);
            }
            if index < boundary {
                expanded_boundary = expanded.len();
            }
        }
        if let Some(dispatches) = &mut read.unchecked_dispatches {
            dispatches.completed_before_last_tag = expanded_boundary;
        }
        read.forms = expanded;
        let refers_to_assert_expr = self.refers_to_assert_expr;
        *self = Self::read_source_declarations(read);
        self.refers_to_assert_expr = refers_to_assert_expr;
    }

    pub(crate) fn read_source_declarations(read: &CheckRead) -> Self {
        Self::read_declarations_with_boundary(read, &read.forms)
    }

    pub(crate) fn earlier_reader_error(source: &str, read: &mut CheckRead) -> Option<ReadError> {
        let end = read
            .first_error_read_end
            .filter(|_| !read.namespace_tokens.is_empty())?;
        let prefix = read.completed_before(end);
        let count = prefix.len();
        let declarations = Self::read_declarations_with_boundary(read, prefix);
        declarations
            .resolve_reader_prefix(source, read, count, end)
            .err()
    }

    fn resolve_reader_prefix(
        &self,
        source: &str,
        read: &mut CheckRead,
        count: usize,
        read_end: usize,
    ) -> Result<(), ReadError> {
        if read.namespace_tokens.is_empty() {
            return Ok(());
        }
        let namespaces = ReaderNamespaces::new(self, read, count);
        let alias = namespaces
            .validate_reader_aliases(source, &read.namespace_tokens, read_end)
            .err();
        // An alias failure occurs at its token. A previously read collection
        // can fail first, while an enclosing collection is still unfinished.
        let before = alias
            .as_ref()
            .map_or(read_end, |error| error.span.start.offset);
        let active = namespaces.resolve_reader_literals(
            source,
            &mut read.forms[..count],
            before,
            &mut read.projected_key_namespaces,
        );
        let inactive = namespaces.resolve_reader_literals(
            source,
            &mut read.inactive_namespace_forms,
            before,
            &mut read.projected_key_namespaces,
        );
        active
            .err()
            .into_iter()
            .chain(inactive.err())
            .min_by_key(|error| error.read_end)
            .map(|error| error.error)
            .or(alias)
            .map_or(Ok(()), Err)
    }

    fn read_declarations_with_boundary(read: &CheckRead, forms: &[Form]) -> Self {
        let mut declarations = Self::read_with_imports(
            forms,
            None,
            Default::default(),
            read.unchecked_dispatches.clone().map(std::sync::Arc::new),
        );
        declarations.namespace_read_end =
            declarations.namespace_end.map(|end| read.end_of_read(end));
        declarations
    }

    fn namespace_read_boundary(&self) -> Option<usize> {
        self.namespace_read_end.or(self.namespace_end)
    }
}

fn top_level_do(form: &Form) -> Option<&[Form]> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let (head, body) = items.split_first()?;
    matches!(&head.kind, Kind::Symbol(name) if name == "do").then_some(body)
}

fn changes_reader_namespace(form: &Form, names: &Names) -> bool {
    if let Some(body) = top_level_do(form) {
        return body
            .iter()
            .any(|form| changes_reader_namespace(form, names));
    }
    let Kind::List(items) = &form.without_metadata().kind else {
        return false;
    };
    items.first().and_then(Form::symbol).is_some_and(|op| {
        matches!(op, "ns" | "require" | "in-ns")
            || matches!(
                names.resolve_parts(op),
                ("clojure.core", "ns" | "require" | "in-ns" | "alias")
            )
    })
}

fn append_top_level_do(form: Form, output: &mut Vec<Form>) {
    if top_level_do(&form).is_some() {
        let Kind::List(items) = form.kind else {
            unreachable!()
        };
        for child in items.into_iter().skip(1) {
            append_top_level_do(child, output);
        }
    } else {
        output.push(form);
    }
}

impl<'a> ReaderNamespaces<'a> {
    fn new(declarations: &'a Declarations, read: &CheckRead, count: usize) -> Self {
        let namespaces = read.forms[..count].iter().filter_map(|form| {
            let Kind::List(items) = &form.without_metadata().kind else {
                return None;
            };
            if !matches!(items.first()?.symbol(), Some("ns" | "clojure.core/ns")) {
                return None;
            }
            Some((form, items))
        });
        let mut scopes = Vec::new();
        let mut namespace_aliases = Vec::new();
        let mut prelude = ReaderPrelude::default();
        if namespaces.clone().nth(1).is_some() {
            let mut indices = BTreeMap::new();
            for (form, items) in namespaces {
                let Some(name) = items.get(1).and_then(Form::symbol) else {
                    continue;
                };
                let mut current = Declarations::default();
                current.read_ns(items, form.span, None);
                let end = read.end_of_read(form.span.end.offset);
                let namespace = *indices.entry(name).or_insert_with(|| {
                    let index = namespace_aliases.len();
                    namespace_aliases.push(NamespaceAliases {
                        name: current.names.namespace,
                        aliases: BTreeMap::new(),
                    });
                    index
                });
                for (alias, target) in current.names.aliases {
                    namespace_aliases[namespace]
                        .aliases
                        .entry(alias)
                        .or_insert((end, target));
                }
                scopes.push(ReaderNamespace { end, namespace });
            }
        } else if declarations.has_top_level_require || declarations.has_top_level_in_ns {
            prelude = Self::read_prelude(declarations, read, count);
        }
        Self {
            declarations,
            scopes,
            namespaces: namespace_aliases,
            prelude,
        }
    }

    fn read_prelude(declarations: &Declarations, read: &CheckRead, count: usize) -> ReaderPrelude {
        let mut known = ReaderPrelude::default();
        let forms = &read.forms[..count];
        if !declarations.has_static_requirements(forms) {
            return known;
        }
        let original = &declarations.names.aliases;
        let mut rest = forms
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, form)| {
                declarations
                    .discarded_comments
                    .binary_search(&index)
                    .is_err()
                    .then_some(form)
            })
            .peekable();
        let mut shadows_require = false;
        while let Some(form) = rest.peek() {
            if let Some(name) = inert_definition(form) {
                shadows_require |= name == "require";
                rest.next();
                continue;
            }
            let core = known
                .aliases
                .get("clojure.core")
                .map(|(_, name)| name)
                .or_else(|| original.get("clojure.core"));
            let Some((aliases, requires_loading)) = literal_require_aliases(
                declarations,
                form,
                core.map(String::as_str),
                shadows_require,
            ) else {
                break;
            };
            let end = read.end_of_read(form.span.end.offset);
            if requires_loading {
                known.unchecked_load = Some(end);
                break;
            }
            if aliases.iter().any(|(alias, name)| {
                known
                    .aliases
                    .get(alias)
                    .map(|(_, name)| name)
                    .or_else(|| original.get(alias))
                    .is_some_and(|previous| previous != name)
            }) {
                break;
            }
            for (alias, name) in aliases {
                if !original.contains_key(&alias) {
                    known.aliases.entry(alias).or_insert((end, name));
                }
            }
            rest.next();
        }
        // The opaque form is itself read before any of its effects, so
        // it can still use aliases established by the literal prelude.
        if (!known.aliases.is_empty() || declarations.has_top_level_in_ns)
            && let Some(form) = rest.next()
        {
            let end = read.end_of_read(form.span.end.offset);
            known.expires = Some(end);
            let core = known
                .aliases
                .get("clojure.core")
                .map(|(_, name)| name)
                .or_else(|| original.get("clojure.core"));
            if let Some(namespace) =
                literal_namespace_switch(declarations, forms, form, core.map(String::as_str))
                && namespace != declarations.namespace()
            {
                known.entered = Some(EnteredNamespace {
                    since: end,
                    through: rest
                        .find(|form| inert_definition(form).is_none())
                        .map(|form| read.end_of_read(form.span.end.offset)),
                    name: namespace.into(),
                });
            }
        }
        known
    }

    fn static_names_at(&self, offset: usize) -> Option<&Names> {
        self.declarations
            .namespace_read_boundary()
            .filter(|end| *end <= offset)
            .map(|_| &self.declarations.names)
    }

    fn namespace_at(&self, offset: usize) -> Option<&NamespaceAliases> {
        let index = self.scopes.partition_point(|scope| scope.end <= offset);
        let scope = &self.scopes[index.checked_sub(1)?];
        Some(&self.namespaces[scope.namespace])
    }

    fn validate_reader_aliases(
        &self,
        source: &str,
        tokens: &[NamespaceToken],
        read_end: usize,
    ) -> Result<(), ReadError> {
        let tag_end = self
            .declarations
            .unchecked_dispatches
            .as_ref()
            .and_then(|dispatches| dispatches.first_tag)
            .map(|tag| tag.end.offset);
        for token in tokens {
            let span = match *token {
                NamespaceToken::Keyword(span) => span,
                NamespaceToken::Map { marker, alias } => alias.unwrap_or(marker),
            };
            if span.end.offset > read_end {
                continue;
            }
            if tag_end.is_some_and(|end| end <= span.start.offset) {
                return Err(ReadError::unchecked(
                    span,
                    "Reader namespace is unchecked after a data reader call",
                ));
            }
            if let Some(scope) = &self.prelude.entered
                && scope.through.is_some_and(|end| end <= span.start.offset)
            {
                return Err(ReadError::unchecked(
                    span,
                    "Reader namespace is unchecked after a top-level effect following in-ns",
                ));
            }
            let qualifier = match *token {
                NamespaceToken::Keyword(span) => {
                    let keyword = spelling(source, span);
                    keyword[2..].split_once('/').map(|(alias, _)| alias)
                }
                NamespaceToken::Map { alias, .. } => alias.map(|span| spelling(source, span)),
            };
            if let Some(alias) = qualifier
                && self.reader_alias(alias, span.start.offset).is_none()
            {
                if let Some(namespace) = self.entered_namespace(span.start.offset) {
                    return Err(ReadError::unchecked(
                        span,
                        format!("Alias {alias} in {namespace} is unchecked after in-ns"),
                    ));
                }
                if self
                    .prelude
                    .unchecked_load
                    .is_some_and(|end| end <= span.start.offset)
                {
                    return Err(ReadError::unchecked(
                        span,
                        format!(
                            "Alias {alias} is unchecked after a require that may load a library"
                        ),
                    ));
                }
                if self.prelude.aliases.contains_key(alias)
                    && self
                        .prelude
                        .expires
                        .is_some_and(|end| end <= span.start.offset)
                {
                    return Err(ReadError::unchecked(
                        span,
                        format!("Alias {alias} is unchecked after a top-level effect"),
                    ));
                }
                let message = match token {
                    NamespaceToken::Keyword(_) => format!(
                        "No namespace alias {alias} for auto-resolved keyword {}",
                        spelling(source, span),
                    ),
                    NamespaceToken::Map { .. } => {
                        format!("No namespace alias {alias} for auto-resolved map",)
                    }
                };
                return Err(ReadError::invalid(span, message));
            }
        }
        Ok(())
    }

    fn reader_alias(&self, name: &str, offset: usize) -> Option<&str> {
        if self.entered_namespace(offset).is_some() {
            return None;
        }
        if let Some((since, name)) = self.prelude.aliases.get(name)
            && *since <= offset
            && self.prelude.expires.is_none_or(|end| offset < end)
        {
            return Some(name);
        }
        if self.scopes.is_empty() {
            return self
                .static_names_at(offset)?
                .aliases
                .get(name)
                .map(String::as_str);
        }
        let (since, target) = self.namespace_at(offset)?.aliases.get(name)?;
        (*since <= offset).then_some(target.as_str())
    }

    fn entered_namespace(&self, offset: usize) -> Option<&str> {
        let scope = self.prelude.entered.as_ref()?;
        (scope.since <= offset && scope.through.is_none_or(|end| offset < end))
            .then_some(scope.name.as_str())
    }

    fn reader_namespace(&self, offset: usize) -> &str {
        if let Some(name) = self.entered_namespace(offset) {
            return name;
        }
        if self.scopes.is_empty() {
            self.static_names_at(offset)
                .map_or("user", |names| names.namespace.as_str())
        } else {
            self.namespace_at(offset)
                .map_or("user", |namespace| namespace.name.as_str())
        }
    }

    fn resolve_reader_keyword(&self, keyword: &str, position: Position) -> Option<String> {
        let local = keyword.strip_prefix("::")?;
        let (namespace, name) = if let Some((alias, name)) = local.split_once('/') {
            (self.reader_alias(alias, position.offset)?, name)
        } else {
            (self.reader_namespace(position.offset), local)
        };
        Some(format!(":{namespace}/{name}"))
    }

    fn resolve_reader_literals(
        &self,
        source: &str,
        forms: &mut [Form],
        before: usize,
        projected_key_namespaces: &mut BTreeMap<usize, String>,
    ) -> Result<(), LiteralError> {
        for form in forms {
            if form.span.start.offset >= before {
                continue;
            }
            let read_end = form.span.end.offset;
            let located = |error| LiteralError { read_end, error };
            match &mut form.kind {
                Kind::AutoNamespacedMap { alias, entries } => {
                    self.resolve_reader_literals(
                        source,
                        entries,
                        before,
                        projected_key_namespaces,
                    )?;
                    if read_end > before {
                        continue;
                    }
                    let at = form.span.start.offset;
                    let namespace = match alias {
                        Some(alias) => self.reader_alias(alias, at).ok_or_else(|| {
                            located(ReadError::invalid(
                                form.span,
                                format!("No namespace alias {alias} for auto-resolved map"),
                            ))
                        })?,
                        None => self.reader_namespace(at),
                    };
                    for [key, _] in entries.as_chunks_mut::<2>().0 {
                        if let Kind::Keyword(keyword) = &mut key.kind
                            && let Some(resolved) =
                                self.resolve_reader_keyword(keyword, key.span.start)
                        {
                            *keyword = resolved;
                        }
                    }
                    qualify_namespaced_entries_at(
                        entries,
                        namespace,
                        source,
                        at,
                        projected_key_namespaces,
                    )
                    .map_err(located)?;
                    form.kind = Kind::Map(std::mem::take(entries));
                }
                Kind::List(items) | Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => {
                    self.resolve_reader_literals(source, items, before, projected_key_namespaces)?;
                }
                Kind::AnonymousFunction(function) => self.resolve_reader_literals(
                    source,
                    &mut function.body,
                    before,
                    projected_key_namespaces,
                )?,
                Kind::Quote(item)
                | Kind::SyntaxQuote(item)
                | Kind::Unquote { form: item, .. }
                | Kind::Conditional { form: item, .. } => {
                    self.resolve_reader_literals(
                        source,
                        std::slice::from_mut(item),
                        before,
                        projected_key_namespaces,
                    )?;
                }
                Kind::Metadata(metadata, object) | Kind::Tagged(metadata, object) => {
                    self.resolve_reader_literals(
                        source,
                        std::slice::from_mut(metadata),
                        before,
                        projected_key_namespaces,
                    )?;
                    self.resolve_reader_literals(
                        source,
                        std::slice::from_mut(object),
                        before,
                        projected_key_namespaces,
                    )?;
                }
                Kind::Keyword(keyword)
                    if keyword.starts_with("::")
                        && (!self.scopes.is_empty()
                            || !self.prelude.aliases.is_empty()
                            || self.prelude.entered.is_some()
                            || self.declarations.namespace_read_boundary().is_some_and(
                                |end| {
                                    form.span.start.offset < end
                                        && self.declarations.namespace() != "user"
                                },
                            )) =>
                {
                    // Later phases use the declared namespace. Keep the value
                    // the JVM read before that declaration took effect.
                    if let Some(resolved) = self.resolve_reader_keyword(keyword, form.span.start) {
                        *keyword = resolved;
                    }
                }
                _ => {}
            }
            if read_end <= before {
                check_resolved_collection_with_namespaces(
                    form,
                    source,
                    &|keyword, position| self.resolve_reader_keyword(keyword, position),
                    projected_key_namespaces,
                )
                .map_err(located)?;
            }
        }
        Ok(())
    }
}

fn spelling(source: &str, span: Span) -> &str {
    &source[span.start.offset..span.end.offset]
}

fn literal_require_aliases(
    declarations: &Declarations,
    form: &Form,
    core_alias: Option<&str>,
    shadows_require: bool,
) -> Option<(BTreeMap<String, String>, bool)> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [head, args @ ..] = items.as_slice() else {
        return None;
    };
    match head.symbol()? {
        "require"
            if !shadows_require
                && declarations.names.core_visible("require")
                && !declarations.names.refers.contains_key("require")
                && declarations.all_refers.is_empty() => {}
        "clojure.core/require" if core_alias.is_none_or(|name| name == "clojure.core") => {}
        _ => return None,
    }
    if args.is_empty() {
        return None;
    }
    let mut update = Declarations::default();
    for argument in args {
        let lib = match &argument.kind {
            Kind::Quote(lib) => lib.as_ref(),
            Kind::List(items) => match items.as_slice() {
                [quote, lib] if quote.symbol() == Some("quote") => lib,
                _ => return None,
            },
            _ => return None,
        };
        update.read_libspec(lib, None, false);
    }
    if !update.diagnostics.is_empty()
        || !update.names.refers.is_empty()
        || !update.all_refers.is_empty()
    {
        return None;
    }
    let requires_loading = update
        .names
        .required_libraries
        .iter()
        .any(|library| !declarations.names.directly_loads(library));
    Some((update.names.aliases, requires_loading))
}

fn inert_definition(form: &Form) -> Option<&str> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [head, name, tail @ ..] = items.as_slice() else {
        return None;
    };
    let Kind::Symbol(name) = &name.kind else {
        return None;
    };
    (head.symbol() == Some("def")
        && !name.contains('/')
        && def_initializer(tail).is_some_and(|value| inert(value, false)))
    .then_some(name.as_str())
}

fn literal_namespace_switch<'a>(
    declarations: &Declarations,
    forms: &[Form],
    form: &'a Form,
    core_alias: Option<&str>,
) -> Option<&'a str> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [head, name] = items.as_slice() else {
        return None;
    };
    match head.symbol()? {
        "in-ns"
            if declarations.names.core_visible("in-ns")
                && !declarations.names.refers.contains_key("in-ns")
                && declarations.all_refers.is_empty()
                && !declarations
                    .source_declarations(forms, "in-ns")
                    .any(|binding| binding.name().span.start.offset < head.span.start.offset) => {}
        "clojure.core/in-ns" if core_alias.is_none_or(|name| name == "clojure.core") => {}
        _ => return None,
    }
    quoted_symbol(name).filter(|name| !name.contains('/'))
}
