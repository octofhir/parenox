//! Namespaced map keys. An explicit prefix is part of the reader syntax.
use super::{Form, Kind, Position, ReadError, Reader, uniqueness};
use crate::syntax::SyntaxKind;

impl Reader<'_> {
    pub(super) fn namespaced_map(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.bump(); // :
        self.token(SyntaxKind::NamespacedMapMark, start);
        if !self.edn
            && self
                .peek()
                .is_some_and(|ch| ch.is_whitespace() || matches!(ch, ',' | '{'))
        {
            return Err(self.error(start, "Namespaced map requires a namespace"));
        }
        let prefix = self.without_comments(Self::form)?;
        let Some(namespace) = prefix
            .symbol()
            .filter(|name| *name == "/" || !name.contains('/'))
        else {
            return Err(self.error(
                start,
                "Namespaced map requires an unqualified namespace symbol",
            ));
        };
        while self.skip_space() {}
        if self.peek() != Some('{') {
            return Err(self.error(start, "Expected a map after the namespace"));
        }
        let Kind::Map(mut entries) = self.form()?.kind else {
            unreachable!("the next form begins with a map delimiter");
        };
        if !self.preserves_map_entries(&entries) {
            for [key, _] in entries.as_chunks_mut::<2>().0 {
                qualify_key(key, namespace);
            }
            uniqueness::check_map(&entries, self.source)?;
        }
        Ok(Kind::Map(entries))
    }
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
