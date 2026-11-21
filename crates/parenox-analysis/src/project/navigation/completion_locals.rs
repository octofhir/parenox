//! Local names and their checked types at expression positions.
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use crate::checker::{Declarations, ProjectCall};
use crate::core_names::known_core_function;
use crate::reader::{Form, Kind};

use super::{
    Session, cached_type_query,
    recovered_types::{TypeScope, close_eof_delimiters},
    symbol_form_at,
};

pub struct LocalCompletion<'a> {
    pub name: Cow<'a, str>,
    pub inferred_type: Option<String>,
}

impl Session {
    /// Find one nearby lexical spelling for an exact written symbol.
    pub fn local_spelling_fix(&self, path: &str, range: Range<usize>) -> Option<&str> {
        let file = self.current_file(path)?;
        let (declarations, forms) = file.navigation_source()?;
        let site = symbol_form_at(forms, range.start)?;
        let Kind::Symbol(written) = &site.kind else {
            return None;
        };
        if site.span.start.offset != range.start
            || site.span.end.offset != range.end
            || written.contains('/')
            || written.starts_with('.')
            || written.ends_with('.')
        {
            return None;
        }
        let written: Vec<_> = written.chars().collect();
        let mut candidates = self
            .visible_local_names(declarations, forms, range.start, "", usize::MAX)
            .into_iter()
            .filter(|candidate| one_edit_apart(&written, candidate));
        let candidate = candidates.next()?;
        candidates.next().is_none().then_some(candidate)
    }

    /// Complete lexical names in a written expression, including forms with open delimiters at EOF.
    pub fn complete_local(
        &self,
        path: &str,
        token_start: usize,
        prefix: &str,
        limit: usize,
    ) -> Vec<LocalCompletion<'_>> {
        let Some(file) = self.current_file(path) else {
            return Vec::new();
        };
        let Some(end) = token_start.checked_add(prefix.len()) else {
            return Vec::new();
        };
        if file.source.get(token_start..end) != Some(prefix) {
            return Vec::new();
        }
        let source = if file.needs_delimiter_recovery(token_start) {
            self.recovered_delimited_source(file, path)
                .map(|recovered| recovered.navigation())
        } else {
            file.navigation_source()
        };
        let Some((declarations, forms)) = source else {
            return Vec::new();
        };
        let names = self.visible_local_names(declarations, forms, token_start, prefix, limit);
        let types = if names.is_empty() {
            None
        } else {
            cached_type_query(&file.completion_types, (token_start, end, limit), || {
                let scope = self.typing_during_edit(file, path, token_start)?;
                completion_types(&scope, token_start, &names).map(Arc::new)
            })
        };
        names
            .into_iter()
            .map(|name| LocalCompletion {
                name: Cow::Borrowed(name),
                inferred_type: types.as_ref().and_then(|types| types.get(name)).cloned(),
            })
            .collect()
    }

    /// Prove the lexical position with a temporary symbol and keep its types query-local.
    pub fn complete_local_gap(
        &self,
        path: &str,
        offset: usize,
        limit: usize,
    ) -> Option<Arc<[LocalCompletion<'static>]>> {
        let file = self.current_file(path)?;
        if !file
            .source
            .get(..offset)?
            .chars()
            .next_back()
            .is_some_and(gap_delimiter)
            || !file
                .source
                .get(offset..)?
                .chars()
                .next()
                .is_none_or(gap_delimiter)
        {
            return None;
        }
        cached_type_query(&file.gap_completions, (offset, limit), || {
            let original = if file.parse_error.is_some() && !file.needs_delimiter_recovery(offset) {
                let (_, forms) = file.navigation_source()?;
                file.source.get(..forms.last()?.span.end.offset)?
            } else {
                &file.source
            };
            const PLACEHOLDER: &str = " __parenox_completion_slot__ ";
            let mut source = original.to_owned();
            source.insert_str(offset, PLACEHOLDER);
            let insertion = offset..offset + PLACEHOLDER.len();
            let site = offset + 1;
            let source = close_eof_delimiters(&source).unwrap_or(source);
            let (declarations, forms) = self.read_navigation_for(path, &source)?;
            let names = self.visible_local_names(&declarations, &forms, site, "", limit);
            if names.is_empty() {
                return None;
            }
            let types = self
                .repaired_type_scope(file, path, &source, Some(insertion))
                .and_then(|repaired| completion_types(&repaired.scope(&source), site, &names));
            Some(
                names
                    .into_iter()
                    .map(|name| LocalCompletion {
                        name: Cow::Owned(name.to_owned()),
                        inferred_type: types.as_ref().and_then(|types| types.get(name)).cloned(),
                    })
                    .collect(),
            )
        })
    }

    fn visible_local_names<'a>(
        &self,
        declarations: &'a Declarations,
        forms: &'a [Form],
        offset: usize,
        prefix: &str,
        limit: usize,
    ) -> Vec<&'a str> {
        let mut standard_core = None;
        declarations.lexical_completions_at(
            forms,
            offset,
            prefix,
            limit,
            self.has_standard_test(),
            |namespace, local, actual| {
                let core_function = namespace == "clojure.core"
                    && known_core_function(local)
                    && *standard_core.get_or_insert_with(|| !self.has_source_namespace(namespace));
                if core_function
                    || self.query_globals.as_ref().is_some_and(|globals| {
                        declarations.has_external_function(globals, namespace, local)
                    })
                {
                    ProjectCall::function(None)
                } else {
                    self.navigation_call(namespace, local, actual)
                }
            },
        )
    }
}

fn one_edit_apart(written: &[char], candidate: &str) -> bool {
    let candidate: Vec<_> = candidate.chars().collect();
    let common = written
        .iter()
        .zip(&candidate)
        .take_while(|(a, b)| a == b)
        .count();
    let written = &written[common..];
    let candidate = &candidate[common..];
    if written.len() == candidate.len() && !written.is_empty() {
        return written[1..] == candidate[1..]
            || (written.len() >= 2
                && written[0] == candidate[1]
                && written[1] == candidate[0]
                && written[2..] == candidate[2..]);
    }
    if written.len() == candidate.len() + 1 {
        return written[1..] == *candidate;
    }
    candidate.len() == written.len() + 1 && *written == candidate[1..]
}

fn gap_delimiter(c: char) -> bool {
    c.is_whitespace() || "()[]{},".contains(c)
}

fn completion_types(
    scope: &TypeScope<'_>,
    offset: usize,
    names: &[&str],
) -> Option<BTreeMap<String, String>> {
    let site = symbol_form_at(scope.forms, offset)?;
    scope.declarations.completion_types(
        scope.forms,
        scope.model,
        scope.globals,
        scope.source,
        site.span,
        names,
    )
}
