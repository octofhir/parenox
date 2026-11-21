//! Declaration sites within the calling namespace.
use std::collections::BTreeMap;

use crate::checker::{Declarations, ScopedReference, bare_dotted_name};
use crate::reader::{Form, Kind};

use super::{Definition, Session, symbol_at};

impl Session {
    /// Find the local namespace clause that introduced a var usage.
    pub fn declaration(&self, path: &str, offset: usize) -> Option<Definition> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        let spelling = symbol_at(forms, offset)?;
        let references = self.collect_file_references(
            path,
            file,
            self.has_standard_test(),
            false,
            &mut BTreeMap::new(),
        );
        let reference = references
            .uses
            .iter()
            .find(|(span, _)| (span.start.offset..span.end.offset).contains(&offset))
            .map(|(_, reference)| *reference)
            .or_else(|| self.top_level_reference(declarations, forms, spelling, offset))?;
        let (namespace, name) = match reference {
            ScopedReference::Referred(namespace, name) => (namespace, name),
            ScopedReference::InNamespace(name) => {
                if declarations.source_var(forms, name, false).is_some() {
                    return None;
                }
                let (namespace, remote_name) = file.model.as_ref()?.resolved_referral(name)?;
                if !declarations.refers_all_from(namespace) {
                    return None;
                }
                (namespace, remote_name)
            }
            ScopedReference::Local(_) => return None,
        };
        let qualifier = spelling
            .split_once('/')
            .filter(|(prefix, _)| !prefix.is_empty());
        let span = qualifier
            .and_then(|(prefix, _)| declarations.alias_declaration(prefix))
            .or_else(|| {
                declarations.referral_declaration(
                    namespace,
                    name,
                    qualifier.is_none().then_some(spelling),
                )
            })
            .or_else(|| declarations.namespace_usage(namespace))?;
        Some(Definition {
            path: path.to_owned(),
            span,
        })
    }
    pub(super) fn top_level_dotted_function(
        &self,
        declarations: &Declarations,
        forms: &[Form],
        spelling: &str,
    ) -> bool {
        if !bare_dotted_name(spelling) {
            return false;
        }
        if declarations.ordinary_dotted_function(forms, spelling) {
            return true;
        }
        let (namespace, name) = declarations.top_level_call(spelling);
        namespace != declarations.namespace()
            && declarations.runtime_alias_gap_in(namespace).is_none()
            && self
                .query_globals
                .as_ref()
                .is_some_and(|globals| globals.proves_ordinary_dotted_function(namespace, name))
    }

    pub(super) fn top_level_reference<'a>(
        &self,
        declarations: &'a Declarations,
        forms: &'a [Form],
        spelling: &'a str,
        offset: usize,
    ) -> Option<ScopedReference<'a>> {
        let index = forms.partition_point(|form| form.span.end.offset <= offset);
        let form = forms.get(index)?.without_metadata();
        match &form.kind {
            Kind::Symbol(_) => {}
            Kind::List(items) => {
                let head = items.first()?.without_metadata();
                if !(head.span.start.offset..head.span.end.offset).contains(&offset) {
                    return None;
                }
            }
            _ => return None,
        }
        if self.top_level_dotted_function(declarations, forms, spelling) {
            return None;
        }
        Some(
            declarations
                .qualified_reference(spelling)
                .map(|(namespace, name)| ScopedReference::Referred(namespace, name))
                .unwrap_or_else(|| {
                    let (namespace, name) = declarations.top_level_call(spelling);
                    if namespace == declarations.namespace() {
                        ScopedReference::InNamespace(name)
                    } else {
                        ScopedReference::Referred(namespace, name)
                    }
                }),
        )
    }
}
