//! Local names and their checked types at a written expression.
use std::sync::Arc;

use crate::checker::ProjectCall;

use super::{Session, cached_type_query, symbol_form_at};

pub struct LocalCompletion<'a> {
    pub name: &'a str,
    pub inferred_type: Option<String>,
}

impl Session {
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
        let names = declarations.lexical_completions_at(
            forms,
            token_start,
            prefix,
            limit,
            self.has_standard_test(),
            |namespace, local, actual| {
                if self.query_globals.as_ref().is_some_and(|globals| {
                    declarations.has_external_function(globals, namespace, local)
                }) {
                    ProjectCall::Function {
                        expected_arity: None,
                    }
                } else {
                    self.navigation_call(namespace, local, actual)
                }
            },
        );
        let types = if names.is_empty() {
            None
        } else {
            cached_type_query(&file.completion_types, (token_start, end, limit), || {
                let scope = self.typing_during_edit(file, path, token_start)?;
                let site = symbol_form_at(scope.forms, token_start)?;
                scope
                    .declarations
                    .completion_types(
                        scope.forms,
                        scope.model,
                        scope.globals,
                        scope.source,
                        site.span,
                        &names,
                    )
                    .map(Arc::new)
            })
        };
        names
            .into_iter()
            .map(|name| LocalCompletion {
                name,
                inferred_type: types.as_ref().and_then(|types| types.get(name)).cloned(),
            })
            .collect()
    }
}
