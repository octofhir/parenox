//! Source locations and hover contracts for named specs.
use super::{Definition, Session};
use crate::project::SpecContractProposal;
use crate::reader::{Form, Kind, Span};

impl Session {
    /// Identify the explicit reader namespace declared before an editable token.
    pub fn reader_namespace_before(&self, path: &str, offset: usize) -> Option<(&str, Span)> {
        let file = self.current_file(path)?;
        if file
            .parse_error
            .as_ref()
            .is_some_and(|error| error.span.start.offset <= offset)
        {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        declarations.reader_namespace_before(forms, offset)
    }

    pub(super) fn spec_definition(&self, path: &str, offset: usize) -> Option<Definition> {
        let (_, _, definition) = self.spec_target(path, offset, false)?;
        Some(definition)
    }

    /// Returns the proposal and the span of the spec keyword under the cursor.
    pub fn hover_spec_at(&self, path: &str, offset: usize) -> Option<(SpecContractProposal, Span)> {
        let (key, span, definition) = self.spec_target(path, offset, true)?;
        Some((self.spec_contract(&definition.path, &key)?, span))
    }

    fn spec_target(
        &self,
        path: &str,
        offset: usize,
        include_declaration: bool,
    ) -> Option<(String, Span, Definition)> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (caller, forms) = file.navigation_source()?;
        let keyword = keyword_at(forms, offset)?;
        let spelling = keyword.keyword()?;
        let key = caller.resolve_spec_keyword(spelling)?;
        let declaration = include_declaration && caller.spec_site(&key) == Some(Ok(keyword.span));
        if !declaration
            && caller
                .spec_type_reference_at(forms, &file.source, offset)
                .as_deref()
                != Some(key.as_str())
            && (self.has_source_namespace("clojure.spec.alpha")
                || caller.validation_spec_at(
                    forms,
                    offset,
                    self.has_standard_test(),
                    |namespace, local, actual| self.navigation_call(namespace, local, actual),
                ) != Some(spelling))
        {
            return None;
        }
        // An explicitly qualified s/def need not be in the namespace named by
        // its key. Include completed declarations from incomplete buffers, but
        // never choose arbitrarily between multiple sites.
        let mut sites = self.files.iter().filter_map(|(path, file)| {
            if file.snapshot != self.snapshot {
                return None;
            }
            let (provider, _) = file.navigation_source()?;
            Some((path, provider.spec_site(&key)?))
        });
        let (path, span) = sites.next()?;
        if sites.next().is_some() {
            return None;
        }
        Some((
            key,
            keyword.span,
            Definition {
                path: path.clone(),
                span: span.ok()?,
            },
        ))
    }
}

pub(super) fn keyword_at(forms: &[Form], offset: usize) -> Option<&Form> {
    let index = forms.partition_point(|form| form.span.end.offset <= offset);
    let form = forms.get(index)?;
    if !(form.span.start.offset..form.span.end.offset).contains(&offset) {
        return None;
    }
    match &form.kind {
        Kind::Keyword(_) => Some(form),
        Kind::List(items)
        | Kind::Vector(items)
        | Kind::Map(items)
        | Kind::Set(items)
        | Kind::AutoNamespacedMap { entries: items, .. } => keyword_at(items, offset),
        Kind::AnonymousFunction(function) => keyword_at(&function.body, offset),
        Kind::Quote(item) => keyword_at(std::slice::from_ref(item), offset),
        Kind::Metadata(meta, target) => keyword_at(std::slice::from_ref(meta), offset)
            .or_else(|| keyword_at(std::slice::from_ref(target), offset)),
        _ => None,
    }
}
