//! Namespaced map keys. An explicit prefix is part of the reader syntax.
use super::{Form, Kind, NamespaceToken, Position, ReadError, Reader, Span, uniqueness};
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
            .is_some_and(|ch| ch.is_whitespace() || matches!(ch, ',' | '{'));
        if !self.edn && !auto && implicit {
            return Err(self.error(start, "Namespaced map requires a namespace"));
        }
        let prefix = if auto && implicit {
            None
        } else {
            let form = self.without_comments(Self::form)?;
            if form
                .symbol()
                .is_none_or(|name| name != "/" && name.contains('/'))
            {
                return Err(self.error(
                    start,
                    "Namespaced map requires an unqualified namespace symbol",
                ));
            }
            Some(form)
        };
        while self.skip_space() {}
        if self.peek() != Some('{') {
            return Err(self.error(start, "Expected a map after the namespace"));
        }
        if auto && let Some(tokens) = &mut self.namespace_tokens {
            tokens.push(NamespaceToken::Map {
                marker,
                alias: prefix.as_ref().map(|form| form.without_metadata().span),
            });
        }
        let Kind::Map(mut entries) = self.form()?.kind else {
            unreachable!("the next form begins with a map delimiter");
        };
        if auto {
            let alias = prefix.as_ref().and_then(Form::symbol).map(str::to_owned);
            return Ok(Kind::AutoNamespacedMap { alias, entries });
        }
        if !self.preserves_map_entries(&entries) {
            let namespace = prefix.as_ref().and_then(Form::symbol).unwrap();
            qualify_namespaced_entries(&mut entries, namespace, self.source)?;
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
    uniqueness::check_map(entries, source)
}

fn qualify_key(key: &mut Form, namespace: &str) {
    match &mut key.kind {
        Kind::Metadata(_, target) => qualify_key(target, namespace),
        Kind::Keyword(name) if !name.starts_with("::") => {
            let name_without_colon = name.trim_start_matches(':');
            if let Some(local) = name_without_colon.strip_prefix("_/") {
                *name = format!(":{local}");
            } else if name_without_colon == "/" || !name_without_colon.contains('/') {
                *name = format!(":{namespace}/{name_without_colon}");
            }
        }
        Kind::Symbol(name) => {
            if let Some(local) = name.strip_prefix("_/") {
                *name = local.into();
            } else if name == "/" || !name.contains('/') {
                *name = format!("{namespace}/{name}");
            }
        }
        Kind::AnonymousArgument(name) => {
            key.kind = Kind::Symbol(format!("{namespace}/{name}"));
        }
        _ => {}
    }
}
