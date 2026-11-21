//! Type queries for complete forms before a reader failure.
use crate::checker::{Declarations, Globals, Model};
use crate::reader::{self, Conditionals, Form, NamespaceToken};

use super::{FileCache, Session};
use crate::project::NavigationFallback;

pub(super) struct TypeScope<'a> {
    pub declarations: &'a Declarations,
    pub forms: &'a [Form],
    pub model: &'a Model,
    pub globals: &'a Globals,
}

pub(in crate::project) struct CheckedPrefix {
    declarations: Declarations,
    forms: Vec<Form>,
    model: Model,
    globals: Globals,
}

impl Session {
    pub(super) fn typing_at<'a>(
        &'a self,
        file: &'a FileCache,
        path: &str,
        offset: usize,
    ) -> Option<TypeScope<'a>> {
        let globals = self.query_globals.as_ref()?;
        if let Some(declarations) = &file.declarations {
            return Some(TypeScope {
                declarations,
                forms: &file.forms,
                model: file.model.as_ref()?,
                globals,
            });
        }
        let cutoff = file.parse_error.as_ref()?.span.start.offset;
        if offset >= cutoff {
            return None;
        }
        let fallback = file.navigation_fallback.as_deref()?;
        let namespace = fallback.declarations.namespace();
        // These namespaces determine standard operations for other files.
        if matches!(namespace, "clojure.core" | "clojure.set") {
            return None;
        }
        let prefix = fallback
            .checked_prefix
            .get_or_init(|| {
                let (_, provider, _, _) = self.navigation_provider(namespace)?;
                if !std::ptr::eq(file, provider) {
                    return None;
                }
                CheckedPrefix::read(file, fallback, path, cutoff, globals)
            })
            .as_ref()?;
        Some(prefix.scope())
    }

    pub(super) fn repaired_type_scope(
        &self,
        file: &FileCache,
        path: &str,
        source: &str,
    ) -> Option<CheckedPrefix> {
        let fallback = file.navigation_fallback.as_deref()?;
        let namespace = fallback.declarations.namespace();
        if matches!(namespace, "clojure.core" | "clojure.set") {
            return None;
        }
        let (_, provider, _, _) = self.navigation_provider(namespace)?;
        if !std::ptr::eq(file, provider) {
            return None;
        }
        let (read, error) = reader::read_for_check(source, conditionals(path), None);
        if error.is_some() {
            return None;
        }
        CheckedPrefix::from_read(read, source, path, namespace, self.query_globals.as_ref()?)
    }
}

impl CheckedPrefix {
    pub(super) fn scope(&self) -> TypeScope<'_> {
        TypeScope {
            declarations: &self.declarations,
            forms: &self.forms,
            model: &self.model,
            globals: &self.globals,
        }
    }

    fn read(
        file: &FileCache,
        fallback: &NavigationFallback,
        path: &str,
        cutoff: usize,
        globals: &Globals,
    ) -> Option<Self> {
        let count = fallback
            .forms
            .partition_point(|form| form.span.end.offset <= cutoff);
        let complete = &fallback.forms[..count];
        let end = complete.last()?.span.end.offset;
        let source = file.source.get(..cutoff)?;
        let (mut read, _) = reader::read_for_check(source, conditionals(path), None);
        read.forms.truncate(count);
        if !read
            .forms
            .iter()
            .map(|form| form.span)
            .eq(complete.iter().map(|form| form.span))
        {
            return None;
        }
        read.namespace_tokens.retain(|token| {
            let span = match token {
                NamespaceToken::Keyword(span) | NamespaceToken::Map { marker: span, .. } => span,
            };
            span.end.offset <= end
        });
        Self::from_read(
            read,
            source,
            path,
            fallback.declarations.namespace(),
            globals,
        )
    }

    fn from_read(
        mut read: reader::CheckRead,
        source: &str,
        path: &str,
        namespace: &str,
        globals: &Globals,
    ) -> Option<Self> {
        let declarations = Declarations::read_source(source, &mut read).ok()?;
        let declarations = declarations
            .with_runtime_loads(&read.forms, &globals.runtime_loads)
            .unwrap_or(declarations);
        // The strict snapshot omits this file. A copied registry can contain
        // entries that its source declarations would hide or conflict with.
        if declarations.namespace() != namespace || declarations.has_prefix_spec_conflict(globals) {
            return None;
        }
        // Publish only in the environment owned by this recovered document.
        // The project's diagnostics and exports still use fully read files.
        let mut globals = globals.clone();
        globals.register_namespace(declarations.namespace());
        let model = Model::build(&declarations, &read.forms, source, path, &globals);
        model.publish(&mut globals);
        Some(Self {
            declarations,
            forms: read.forms,
            model,
            globals,
        })
    }
}

fn conditionals(path: &str) -> Conditionals<'static> {
    if path.ends_with(".cljc") {
        Conditionals::Select(":clj")
    } else {
        Conditionals::Reject
    }
}
