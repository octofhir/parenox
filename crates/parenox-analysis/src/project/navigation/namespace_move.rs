//! Source edits for a namespace move. Library aliases keep their spelling.
use std::collections::{BTreeMap, BTreeSet};

use crate::checker::ScopedReference;
use crate::reader::{self, Form, Kind, Span};
use crate::syntax::{SyntaxElement, SyntaxKind};

use super::{Definition, Session};

impl Session {
    /// Plan source text edits before an editor moves a namespace's file.
    pub fn namespace_move(
        &self,
        path: &str,
        old_namespace: &str,
        new_namespace: &str,
    ) -> Result<Vec<Definition>, String> {
        if new_namespace.contains('/')
            || !reader::read(new_namespace).is_ok_and(
                |forms| matches!(forms.as_slice(), [name] if name.symbol() == Some(new_namespace)),
            )
        {
            return Err("The destination namespace is invalid".into());
        }
        let file = self
            .current_file(path)
            .ok_or("Source file is not indexed")?;
        let (declarations, _) = file
            .navigation_source()
            .filter(|_| file.parse_error.is_none())
            .ok_or("The source namespace cannot be read")?;
        if declarations.namespace() == new_namespace {
            return Ok(Vec::new());
        }
        if declarations.namespace() != old_namespace
            || self
                .navigation_provider(old_namespace)
                .map(|provider| provider.0)
                != Some(path)
        {
            return Err(
                "The original file path does not identify a unique source namespace".into(),
            );
        }
        if self.files.values().any(|file| {
            file.snapshot == self.snapshot
                && file.navigation_source().is_some_and(|(declarations, _)| {
                    declarations.namespace() == new_namespace
                        || declarations
                            .dependencies
                            .iter()
                            .any(|name| name == new_namespace)
                })
        }) {
            return Err(format!(
                "Namespace {new_namespace} is already used in the project"
            ));
        }

        let mut edits = Vec::new();
        let mut providers = BTreeMap::new();
        let standard_test = self.has_standard_test();
        let old_prefix = old_namespace
            .split_once('.')
            .map_or(old_namespace, |(prefix, _)| prefix);
        for (source_path, file) in &self.files {
            if file.snapshot != self.snapshot
                || !(file.source.contains(old_prefix) || file.source.contains(new_namespace))
            {
                continue;
            }
            let parsed = reader::parse(&file.source);
            let mut uses = NamespaceUses::default();
            for form in &parsed.forms {
                if let Kind::List(items) = &form.without_metadata().kind
                    && matches!(
                        items.first().and_then(Form::symbol),
                        Some("ns" | "clojure.core/ns")
                    )
                    && let [_, name, clauses @ ..] = items.as_slice()
                {
                    if source_path == path && name.symbol() == Some(old_namespace) {
                        uses.edits.push(name.without_metadata().span);
                    }
                    uses.clauses(clauses, old_namespace, false)?;
                }
            }
            if uses.required.contains(new_namespace) {
                return Err(format!(
                    "Namespace {new_namespace} is already required in the project"
                ));
            }
            if source_path == path && uses.edits.is_empty() {
                return Err("An explicit namespace declaration is required".into());
            }
            let mut qualified: Vec<_> = self.collect_file_references(
                source_path, file, standard_test, false, &mut providers,
            ).uses.into_iter().filter_map(|(span, reference)| {
                matches!(reference, ScopedReference::Referred(namespace, _) if namespace == old_namespace)
                    .then_some(span)
            }).collect();
            // A direct top-level call can be an ordinary function usage even
            // when the checker has no enclosing typed declaration to visit.
            for form in &parsed.forms {
                if let Kind::List(items) = &form.without_metadata().kind
                    && let Some(head) = items.first()
                    && let Some((namespace, local)) =
                        head.symbol().and_then(|name| name.split_once('/'))
                    && namespace == old_namespace
                    && self
                        .resolve_source_var(source_path, namespace, local, None)
                        .is_some()
                {
                    qualified.push(head.without_metadata().span);
                }
            }
            let resolved: BTreeMap<_, _> = qualified
                .into_iter()
                .filter(|span| {
                    file.source[span.start.offset..span.end.offset]
                        .split_once('/')
                        .is_some_and(|(namespace, _)| namespace == old_namespace)
                })
                .map(|span| (span.start.offset, span))
                .collect();
            uses.audit(&parsed, old_namespace, new_namespace, &resolved)?;
            edits.extend(uses.edits.into_iter().map(|span| Definition {
                path: source_path.clone(),
                span,
            }));
        }
        edits.sort_unstable_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then(a.span.start.offset.cmp(&b.span.start.offset))
        });
        edits.dedup();
        Ok(edits)
    }
}

#[derive(Default)]
struct NamespaceUses<'a> {
    edits: Vec<Span>,
    aliases: BTreeMap<&'a str, BTreeSet<&'a str>>,
    conditional_aliases: BTreeSet<&'a str>,
    required: BTreeSet<&'a str>,
    bindings: BTreeSet<usize>,
    opaque_libspec: bool,
}

impl<'a> NamespaceUses<'a> {
    fn clauses(&mut self, clauses: &'a [Form], old: &str, conditional: bool) -> Result<(), String> {
        for clause in clauses {
            match &clause.without_metadata().kind {
                Kind::List(items) => {
                    if let [head, libraries @ ..] = items.as_slice()
                        && matches!(
                            head.keyword(),
                            Some(":require" | ":use" | ":require-macros" | ":use-macros")
                        )
                    {
                        self.libraries(libraries, old, conditional)?;
                    }
                }
                Kind::Conditional { form, .. } => {
                    for branch in branches(form) {
                        match &branch.without_metadata().kind {
                            Kind::Vector(clauses) => self.clauses(clauses, old, true)?,
                            _ => self.clauses(std::slice::from_ref(branch), old, true)?,
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn libraries(
        &mut self,
        libraries: &'a [Form],
        old: &str,
        conditional: bool,
    ) -> Result<(), String> {
        for form in libraries {
            match &form.without_metadata().kind {
                Kind::Symbol(name) => {
                    self.required.insert(name);
                    if name == old {
                        self.edits.push(form.without_metadata().span);
                    }
                }
                Kind::Vector(items) | Kind::List(items) => {
                    let Some((library, options)) = items.split_first() else {
                        continue;
                    };
                    let Some(name) = library.symbol() else {
                        continue;
                    };
                    if options
                        .first()
                        .is_some_and(|option| option.keyword().is_none())
                    {
                        self.opaque_libspec = true;
                        if old
                            .strip_prefix(name)
                            .is_some_and(|suffix| suffix.starts_with('.'))
                        {
                            return Err(format!("A prefixed libspec may refer to namespace {old}"));
                        }
                        continue;
                    }
                    self.required.insert(name);
                    if name == old {
                        self.edits.push(library.without_metadata().span);
                    }
                    for [option, value] in options.as_chunks::<2>().0 {
                        if matches!(option.keyword(), Some(":as" | ":as-alias")) {
                            self.alias(value, name, conditional);
                        }
                    }
                }
                Kind::Conditional { splicing, form } => {
                    for branch in branches(form) {
                        if *splicing {
                            if let Kind::List(forms) | Kind::Vector(forms) =
                                &branch.without_metadata().kind
                            {
                                self.libraries(forms, old, true)?;
                            }
                        } else {
                            self.libraries(std::slice::from_ref(branch), old, true)?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn alias(&mut self, form: &'a Form, name: &'a str, conditional: bool) {
        if let Some(alias) = form.symbol() {
            self.aliases.entry(alias).or_default().insert(name);
            self.bindings
                .insert(form.without_metadata().span.start.offset);
            if conditional {
                self.conditional_aliases.insert(alias);
            }
        } else if let Kind::Conditional {
            form,
            splicing: false,
        } = &form.without_metadata().kind
        {
            for branch in branches(form) {
                self.alias(branch, name, true);
            }
        }
    }

    fn audit(
        &mut self,
        parsed: &reader::Parse,
        old: &str,
        new: &str,
        qualified: &BTreeMap<usize, Span>,
    ) -> Result<(), String> {
        let known: BTreeSet<_> = self.edits.iter().map(|span| span.start.offset).collect();
        for token in parsed
            .syntax()
            .descendants_with_tokens()
            .filter_map(SyntaxElement::into_token)
        {
            let start = usize::from(token.text_range().start());
            let symbol = token.text();
            let is_qualified = symbol
                .split_once('/')
                .is_some_and(|(prefix, name)| prefix == old && !name.is_empty());
            if !matches!(token.kind(), SyntaxKind::Symbol | SyntaxKind::ErrorToken)
                || (symbol != old && !is_qualified)
                || known.contains(&start)
                || self.bindings.contains(&start)
                || token
                    .parent_ancestors()
                    .any(|node| node.kind() == SyntaxKind::Discard)
            {
                continue;
            }
            if is_qualified && self.aliases.contains_key(old) {
                // With a same-spelling alias its source spelling stays put.
                if self.aliases[old].len() > 1 || self.conditional_aliases.contains(old) {
                    return Err(format!("Ambiguous alias {old} in a reader branch"));
                }
                continue;
            }
            let Some(span) = qualified.get(&start).filter(|_| is_qualified) else {
                return Err(format!(
                    "Cannot verify a source reference to namespace {old}"
                ));
            };
            if self.opaque_libspec {
                return Err("A prefixed libspec makes the destination qualifier ambiguous".into());
            }
            if self
                .aliases
                .get(new)
                .is_some_and(|names| names.iter().any(|&name| name != old))
            {
                return Err(format!("The alias {new} would capture a renamed reference"));
            }
            let mut span = *span;
            span.end.offset = span.start.offset + old.len();
            span.end.column = span.start.column + old.chars().count();
            self.edits.push(span);
        }
        if !self.edits.is_empty() && !parsed.errors.is_empty() {
            return Err(format!(
                "A source file using namespace {old} has reader errors"
            ));
        }
        Ok(())
    }
}

fn branches(form: &Form) -> impl Iterator<Item = &Form> {
    let forms = match &form.without_metadata().kind {
        Kind::List(forms) => forms.as_slice(),
        _ => &[],
    };
    forms.as_chunks::<2>().0.iter().map(|[_, branch]| branch)
}
