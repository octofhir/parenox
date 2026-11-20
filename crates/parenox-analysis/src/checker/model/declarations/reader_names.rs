//! Attach source namespace context to names the frontend left unresolved.
use super::Declarations;
use crate::reader::{
    CheckRead, Form, Kind, NamespaceToken, ReadError, Span, qualify_namespaced_entries,
};

impl Declarations {
    pub(crate) fn reader_namespace_before(
        &self,
        forms: &[Form],
        offset: usize,
    ) -> Option<(&str, Span)> {
        self.namespace_end.filter(|end| *end <= offset)?;
        Some((
            self.reader_namespace(offset),
            self.namespace_selection(forms)?,
        ))
    }

    pub(crate) fn read_source(source: &str, read: &mut CheckRead) -> Result<Self, ReadError> {
        let mut declarations = Self::read(&read.forms);
        declarations.validate_reader_aliases(source, &read.namespace_tokens)?;
        if read
            .namespace_tokens
            .iter()
            .any(|token| matches!(token, NamespaceToken::Map { .. }))
        {
            declarations.resolve_reader_maps(source, &mut read.forms)?;
            declarations = Self::read(&read.forms);
        }
        // References can occur in quoted data and namespace renames. Check
        // the selected forms only when the spelling occurs in the source.
        declarations.refers_to_assert_expr =
            source.contains("assert-expr") && declarations.references_assert_expr(&read.forms);
        Ok(declarations)
    }

    fn validate_reader_aliases(
        &self,
        source: &str,
        tokens: &[NamespaceToken],
    ) -> Result<(), ReadError> {
        for token in tokens {
            match *token {
                NamespaceToken::Keyword(span) => {
                    let keyword = spelling(source, span);
                    if let Some((alias, _)) = keyword[2..].split_once('/')
                        && self.reader_alias(alias, span.start.offset).is_none()
                    {
                        return Err(ReadError {
                            span,
                            message: format!(
                                "No namespace alias {alias} for auto-resolved keyword {keyword}"
                            ),
                        });
                    }
                }
                NamespaceToken::Map {
                    alias: Some(span), ..
                } => {
                    let alias = spelling(source, span);
                    if self.reader_alias(alias, span.start.offset).is_none() {
                        return Err(ReadError {
                            span,
                            message: format!("No namespace alias {alias} for auto-resolved map"),
                        });
                    }
                }
                NamespaceToken::Map { alias: None, .. } => {}
            }
        }
        Ok(())
    }

    fn reader_alias(&self, name: &str, offset: usize) -> Option<&str> {
        self.namespace_end.filter(|end| offset >= *end)?;
        self.names.aliases.get(name).map(String::as_str)
    }

    fn reader_namespace(&self, offset: usize) -> &str {
        if self.namespace_end.is_some_and(|end| offset >= end) {
            self.namespace()
        } else {
            "user"
        }
    }

    fn resolve_reader_maps(&self, source: &str, forms: &mut [Form]) -> Result<(), ReadError> {
        for form in forms {
            if let Kind::AutoNamespacedMap { alias, entries } = &mut form.kind {
                let at = form.span.start.offset;
                let namespace = if let Some(alias) = alias {
                    self.reader_alias(alias, at).ok_or_else(|| ReadError {
                        span: form.span,
                        message: format!("No namespace alias {alias} for auto-resolved map"),
                    })?
                } else {
                    self.reader_namespace(at)
                };
                for [key, _] in entries.as_chunks_mut::<2>().0 {
                    if let Kind::Keyword(keyword) = &mut key.kind
                        && let Some(local) = keyword.strip_prefix("::")
                    {
                        let at = key.span.start.offset;
                        let (namespace, name) = if let Some((alias, name)) = local.split_once('/') {
                            let namespace = self.reader_alias(alias, at).ok_or_else(|| ReadError {
                                span: key.span,
                                message: format!("No namespace alias {alias} for auto-resolved keyword {keyword}"),
                            })?;
                            (namespace, name)
                        } else {
                            (self.reader_namespace(at), local)
                        };
                        *keyword = format!(":{namespace}/{name}");
                    }
                }
                qualify_namespaced_entries(entries, namespace, source)?;
                form.kind = Kind::Map(std::mem::take(entries));
            }
            match &mut form.kind {
                Kind::List(items) | Kind::Vector(items) | Kind::Map(items) | Kind::Set(items) => {
                    self.resolve_reader_maps(source, items)?;
                }
                Kind::AnonymousFunction(function) => {
                    self.resolve_reader_maps(source, &mut function.body)?
                }
                Kind::Quote(item)
                | Kind::SyntaxQuote(item)
                | Kind::Unquote { form: item, .. }
                | Kind::Conditional { form: item, .. } => {
                    self.resolve_reader_maps(source, std::slice::from_mut(item))?;
                }
                Kind::Metadata(metadata, object) | Kind::Tagged(metadata, object) => {
                    self.resolve_reader_maps(source, std::slice::from_mut(metadata))?;
                    self.resolve_reader_maps(source, std::slice::from_mut(object))?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn spelling(source: &str, span: Span) -> &str {
    &source[span.start.offset..span.end.offset]
}
