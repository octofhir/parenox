//! Query-local parsing and type checking after a reader failure.
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use crate::checker::{Declarations, Globals, Model, SpecFingerprint};
use crate::reader::{self, Conditionals, Form, NamespaceToken};
use crate::syntax::SyntaxKind;

use super::{FileCache, Session};
use crate::project::NavigationFallback;

pub(super) struct TypeScope<'a> {
    pub source: &'a str,
    pub declarations: &'a Declarations,
    pub forms: &'a [Form],
    pub model: &'a Model,
    pub globals: &'a Globals,
}

pub(in crate::project) struct CheckedPrefix {
    declarations: Declarations,
    forms: Arc<[Form]>,
    model: Model,
    globals: Globals,
}

pub(in crate::project) struct RecoveredDelimiters {
    source: String,
    declarations: Declarations,
    forms: Vec<Form>,
    checked: OnceLock<Option<CheckedPrefix>>,
}

impl RecoveredDelimiters {
    pub(super) fn navigation(&self) -> (&Declarations, &[Form]) {
        (&self.declarations, &self.forms)
    }
}

impl Session {
    pub(super) fn recovered_delimited_source<'a>(
        &self,
        file: &'a FileCache,
        path: &str,
    ) -> Option<&'a RecoveredDelimiters> {
        file.recovered_delimiters
            .get_or_init(|| {
                let source = close_eof_delimiters(&file.source)?;
                let (declarations, forms) = self.read_navigation_for(path, &source)?;
                Some(RecoveredDelimiters {
                    source,
                    declarations,
                    forms,
                    checked: OnceLock::new(),
                })
            })
            .as_ref()
    }

    pub(super) fn read_navigation_for(
        &self,
        path: &str,
        source: &str,
    ) -> Option<(Declarations, Vec<Form>)> {
        let (mut read, error) = reader::read_for_check(source, conditionals(path), None);
        if error.is_some() {
            return None;
        }
        let declarations = Declarations::read_source(source, &mut read).ok()?;
        let declarations = self
            .query_globals
            .as_ref()
            .and_then(|globals| {
                declarations.with_runtime_loads(&read.forms, &globals.runtime_loads)
            })
            .unwrap_or(declarations);
        Some((declarations, read.forms))
    }

    pub(super) fn typing_during_edit<'a>(
        &'a self,
        file: &'a FileCache,
        path: &str,
        offset: usize,
    ) -> Option<TypeScope<'a>> {
        if !file.needs_delimiter_recovery(offset) {
            return self.typing_at(file, path, offset);
        }
        let recovered = self.recovered_delimited_source(file, path)?;
        let checked = recovered
            .checked
            .get_or_init(|| self.repaired_type_scope(file, path, &recovered.source, None))
            .as_ref()?;
        Some(checked.scope(&recovered.source))
    }

    pub(super) fn typing_at<'a>(
        &'a self,
        file: &'a FileCache,
        path: &str,
        offset: usize,
    ) -> Option<TypeScope<'a>> {
        let globals = self.query_globals.as_ref()?;
        if let Some(declarations) = &file.declarations {
            return Some(TypeScope {
                source: &file.source,
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
        Some(prefix.scope(&file.source))
    }

    pub(super) fn repaired_type_scope(
        &self,
        file: &FileCache,
        path: &str,
        source: &str,
        insertion: Option<Range<usize>>,
    ) -> Option<CheckedPrefix> {
        let (original, _) = file.navigation_source()?;
        let namespace = original.namespace();
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
        CheckedPrefix::from_read(
            read,
            source,
            path,
            namespace,
            self.query_globals.as_ref()?,
            insertion.zip(file.spec_fingerprint.as_deref()),
        )
    }
}

impl CheckedPrefix {
    pub(super) fn scope<'a>(&'a self, source: &'a str) -> TypeScope<'a> {
        TypeScope {
            source,
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
        read.inactive_namespace_forms
            .retain(|form| form.span.end.offset <= end);
        Self::from_read(
            read,
            source,
            path,
            fallback.declarations.namespace(),
            globals,
            None,
        )
    }

    fn from_read(
        mut read: reader::CheckRead,
        source: &str,
        path: &str,
        namespace: &str,
        globals: &Globals,
        original_specs: Option<(Range<usize>, &SpecFingerprint)>,
    ) -> Option<Self> {
        let declarations = Declarations::read_source(source, &mut read).ok()?;
        let declarations = declarations
            .with_runtime_loads(&read.forms, &globals.runtime_loads)
            .unwrap_or(declarations);
        // A fully read document can reuse its spec registrations when the
        // query's insertion preserves their source and order.
        if declarations.namespace() != namespace
            || (declarations.has_prefix_spec_conflict(globals)
                && !original_specs.is_some_and(|(insertion, original)| {
                    declarations
                        .spec_fingerprint(source, &read.forms)
                        .is_some_and(|current| {
                            original.matches_after_insertion(&current, &insertion)
                        })
                }))
        {
            return None;
        }
        // Publish only in the environment owned by this recovered document.
        // The project's diagnostics and exports still use fully read files.
        let mut globals = globals.clone();
        globals.register_namespace(declarations.namespace());
        let forms = read.forms.into();
        let model = Model::build(&declarations, &forms, source, path, &globals);
        model.publish(&mut globals, true);
        Some(Self {
            declarations,
            forms,
            model,
            globals,
        })
    }
}

/// Append closing delimiters only. The ranges of written tokens stay valid.
pub(super) fn close_eof_delimiters(source: &str) -> Option<String> {
    use SyntaxKind::*;
    let tree = reader::parse(source).syntax();
    let mut closers = Vec::new();
    for node in tree
        .descendants()
        .filter(|node| node.kind() == Error && usize::from(node.text_range().end()) == source.len())
    {
        let closer = match node.first_child_or_token()?.kind() {
            OpenParen | OpenAnonymousFunction => ')',
            OpenBracket => ']',
            OpenBrace | OpenSet => '}',
            _ => return None,
        };
        closers.push(closer);
    }
    if closers.is_empty() {
        return None;
    }
    let mut repaired = source.to_owned();
    repaired.push('\n');
    repaired.extend(closers.into_iter().rev());
    Some(repaired)
}

fn conditionals(path: &str) -> Conditionals<'static> {
    if path.ends_with(".cljc") {
        Conditionals::Select(":clj")
    } else {
        Conditionals::Reject
    }
}
