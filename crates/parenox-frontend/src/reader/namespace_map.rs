//! Namespaced map keys. An explicit prefix is part of the reader syntax.
use std::borrow::Cow;
use std::collections::BTreeMap;

use super::{
    Form, ImmediateSyntaxQuote, Kind, MAX_NESTING_DEPTH, NamespaceToken, Position, ReadError,
    Reader, Span, uniqueness,
};
use crate::syntax::SyntaxKind;

impl Reader<'_> {
    pub(super) fn namespaced_map(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.bump(); // :
        let auto = self.peek() == Some(':');
        if auto {
            self.bump();
            if self.edn {
                return Err(self.error(
                    start,
                    "Auto-resolved namespaced maps are unavailable in EDN",
                ));
            }
        }
        let marker = Span {
            start,
            end: self.pos,
        };
        self.token(SyntaxKind::NamespacedMapMark, start);
        let implicit = self
            .peek()
            .is_some_and(|ch| Self::reader_whitespace(ch) || matches!(ch, ',' | '{'));
        if !self.edn && !auto && implicit {
            return Err(self.error(start, "Namespaced map requires a namespace"));
        }
        let prefix = if auto && implicit {
            None
        } else {
            Some(self.without_comments(Self::form)?)
        };
        let tag_span = prefix
            .as_ref()
            .and_then(|form| self.data_reader_result_span(form));
        if let Some(form) = &prefix
            && tag_span.is_none()
            && form
                .symbol()
                .is_none_or(|name| name != "/" && name.contains('/'))
        {
            return Err(self.error(
                start,
                "Namespaced map requires an unqualified namespace symbol",
            ));
        }
        while self.skip_space() {}
        if self.peek() != Some('{') {
            return Err(self.error(start, "Expected a map after the namespace"));
        }
        if auto
            && tag_span.is_none()
            && let Some(tokens) = &mut self.namespace_tokens
        {
            tokens.push(NamespaceToken::Map {
                marker,
                alias: prefix.as_ref().map(|form| form.without_metadata().span),
            });
        }
        // The physical opening brace belongs to this map. Pending forms from
        // a splicing prefix become its first entries, ahead of the brace's text.
        let open = self.pos;
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(self.nesting_error(open));
        }
        self.depth += 1;
        self.start_node(open);
        self.bump(); // {
        let entries = self.map(open);
        self.finish_node(if entries.is_ok() {
            SyntaxKind::Map
        } else {
            SyntaxKind::Error
        });
        self.depth -= 1;
        let mut entries = entries?;
        if let Some(span) = tag_span {
            if let Some(prefix) = prefix {
                self.retain_inactive_namespace_form(prefix);
            }
            self.retain_rejected_literal(start, Kind::Map(entries));
            return Err(ReadError::unchecked(
                span,
                "Namespaced map prefix depends on the value returned by a data reader",
            ));
        }
        if auto {
            let alias = prefix.as_ref().and_then(Form::symbol).map(str::to_owned);
            return Ok(Kind::AutoNamespacedMap { alias, entries });
        }
        if !self.preserves_map_entries(&entries) {
            let namespace = prefix.as_ref().and_then(Form::symbol).unwrap();
            if let Err(error) = qualify_namespaced_entries_at(
                &mut entries,
                namespace,
                self.source,
                start.offset,
                &mut self.projected_key_namespaces,
            ) {
                self.retain_rejected_literal(start, Kind::Map(entries));
                return Err(error);
            }
        }
        Ok(Kind::Map(entries))
    }
}

pub fn qualify_namespaced_entries(
    entries: &mut [Form],
    namespace: &str,
    source: &str,
) -> Result<(), ReadError> {
    for [key, _] in entries.as_chunks_mut::<2>().0 {
        qualify_key(key, namespace);
    }
    uniqueness::check_qualified_map(entries, source, namespace)
}

pub fn qualify_namespaced_entries_at(
    entries: &mut [Form],
    namespace: &str,
    source: &str,
    offset: usize,
    projected_key_namespaces: &mut BTreeMap<usize, String>,
) -> Result<(), ReadError> {
    if entries.as_chunks::<2>().0.iter().any(|[key, _]| {
        matches!(Reader::immediate_syntax_quote_value(key),
            Some(ImmediateSyntaxQuote::Form(value))
                if matches!(value.without_metadata().kind,
                    Kind::Symbol(_) | Kind::AnonymousArgument(_) | Kind::Keyword(_)))
    }) {
        projected_key_namespaces.insert(offset, namespace.to_owned());
    }
    qualify_namespaced_entries(entries, namespace, source)
}

fn qualify_key(key: &mut Form, namespace: &str) {
    match &mut key.kind {
        Kind::Metadata(_, target) => qualify_key(target, namespace),
        Kind::Keyword(name) => {
            if let Some(Cow::Owned(qualified)) = qualify_keyword(name, Some(namespace)) {
                *name = qualified;
            }
        }
        Kind::Symbol(name) => {
            if let Some(Cow::Owned(qualified)) = qualify_symbol(name, Some(namespace)) {
                *name = qualified;
            }
        }
        Kind::AnonymousArgument(name) => {
            key.kind = Kind::Symbol(format!("{namespace}/{name}"));
        }
        _ => {}
    }
}

pub(super) fn qualify_keyword<'a>(name: &'a str, namespace: Option<&str>) -> Option<Cow<'a, str>> {
    if name.starts_with("::") {
        return None;
    }
    qualify_name(name, name.trim_start_matches(':'), namespace, ":")
}

pub(super) fn qualify_symbol<'a>(name: &'a str, namespace: Option<&str>) -> Option<Cow<'a, str>> {
    qualify_name(name, name, namespace, "")
}

fn qualify_name<'a>(
    spelling: &'a str,
    name: &str,
    namespace: Option<&str>,
    prefix: &str,
) -> Option<Cow<'a, str>> {
    Some(if let Some(local) = name.strip_prefix("_/") {
        Cow::Owned(format!("{prefix}{local}"))
    } else if name == "/" || !name.contains('/') {
        let namespace = namespace?;
        Cow::Owned(format!("{prefix}{namespace}/{name}"))
    } else {
        Cow::Borrowed(spelling)
    })
}
