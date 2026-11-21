//! Refactoring requires complete coverage of the tokens that could refer to the target.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::checker::{Declarations, ScopedReference};
use crate::reader::{self, Form, Kind, Span};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxToken};

use super::{Definition, Session, SourceTarget};

struct Rename {
    selected: Span,
    edits: Vec<Definition>,
    local: bool,
}

impl Session {
    /// Return just the name to be replaced, preserving the qualifier at a qualified use.
    pub fn prepare_rename(&self, path: &str, offset: usize) -> Option<Span> {
        self.rename_plan(path, offset, None)
            .ok()
            .map(|plan| plan.selected)
    }

    /// Return edits for a source var or lexical binding in the current snapshot.
    pub fn rename(
        &self,
        path: &str,
        offset: usize,
        new_name: &str,
    ) -> Result<Vec<Definition>, String> {
        if !plain_name(new_name) {
            return Err("Expected an unqualified Clojure name for the replacement".into());
        }
        self.rename_plan(path, offset, Some(new_name))
            .map(|plan| plan.edits)
    }

    pub fn linked_editing_ranges(&self, path: &str, offset: usize) -> Option<Vec<Span>> {
        let plan = self.rename_plan(path, offset, None).ok()?;
        (plan.local && plan.edits.len() > 1)
            .then(|| plan.edits.into_iter().map(|edit| edit.span).collect())
    }

    fn rename_plan(
        &self,
        path: &str,
        offset: usize,
        new_name: Option<&str>,
    ) -> Result<Rename, String> {
        let unsupported = || "This position has no supported rename target".to_owned();
        let unproven = || "Rename cannot verify every occurrence of the name".to_owned();
        let origin_file = self.current_file(path).ok_or_else(unsupported)?;
        if origin_file.parse_error.is_some() || !origin_file.source.is_char_boundary(offset) {
            return Err(unsupported());
        }
        let standard_test = self.has_standard_test();
        let mut providers = BTreeMap::new();
        let origin =
            self.collect_file_references(path, origin_file, standard_test, false, &mut providers);
        let target = self
            .reference_target(path, offset, origin_file, &origin)
            .ok_or_else(unsupported)?;
        let (name, namespace, local_scope) = match &target {
            SourceTarget::Local(binding) => {
                let (_, forms) = origin_file.navigation_source().ok_or_else(unsupported)?;
                let top = containing_form(forms, binding.start.offset).ok_or_else(unsupported)?;
                let name = &origin_file.source[bounds(*binding)];
                let parsed = reader::parse(&origin_file.source).syntax();
                let token = parsed
                    .token_at_offset(
                        u32::try_from(binding.start.offset)
                            .map_err(|_| unsupported())?
                            .into(),
                    )
                    .find(|token| token_bounds(token) == bounds(*binding))
                    .ok_or_else(unsupported)?;
                if !plain_name(name) || implicit_key_binding(&token) {
                    return Err("This binding derives its name from a destructured key".into());
                }
                if let Some(replacement) =
                    new_name.filter(|replacement| *replacement != name && replacement.contains('.'))
                    && !origin.named_fn_bindings.contains(binding)
                {
                    return Err(format!(
                        "Clojure/JVM local binding {replacement} cannot contain a dot"
                    ));
                }
                (name, None, Some(bounds(top.span)))
            }
            SourceTarget::Var(target) => {
                let (decl, forms) = target.source.navigation_source().ok_or_else(unsupported)?;
                let form =
                    containing_form(forms, target.span.start.offset).ok_or_else(unsupported)?;
                if target.source.parse_error.is_some()
                    || !decl.has_known_var_surface(forms)
                    || !ordinary_var(form, target.span)
                {
                    return Err("This var is outside the supported source declaration forms".into());
                }
                (target.name, Some(target.namespace), None)
            }
        };
        let mut edits = self.references(path, offset, true);
        if let SourceTarget::Var(target) = &target {
            let (declarations, forms) = target.source.navigation_source().ok_or_else(unproven)?;
            edits.extend(
                declarations
                    .local_var_registrations(forms, target.name)
                    .into_iter()
                    .map(|span| Definition {
                        path: target.path.into(),
                        span,
                    }),
            );
            edits.sort_unstable_by(|a, b| {
                a.path
                    .cmp(&b.path)
                    .then(a.span.start.offset.cmp(&b.span.start.offset))
            });
            edits.dedup();
        }
        let selected = edits
            .iter()
            .find(|edit| edit.path == path && bounds(edit.span).contains(&offset))
            .and_then(|edit| name_span(&origin_file.source, edit.span, name))
            .filter(|span| bounds(*span).contains(&offset))
            .ok_or_else(unsupported)?;
        for edit in &mut edits {
            let source = &self.current_file(&edit.path).ok_or_else(unproven)?.source;
            edit.span = name_span(source, edit.span, name).ok_or_else(unproven)?;
        }
        if new_name == Some(name) {
            return Ok(Rename {
                selected,
                edits: Vec::new(),
                local: local_scope.is_some(),
            });
        }

        let edited: BTreeMap<_, Vec<_>> = edits
            .chunk_by(|a, b| a.path == b.path)
            .map(|file| {
                (
                    file[0].path.as_str(),
                    file.iter().map(|edit| bounds(edit.span)).collect(),
                )
            })
            .collect();
        let qualified = namespace.map(|namespace| format!("{namespace}/{name}"));
        for (candidate_path, file) in &self.files {
            if file.snapshot != self.snapshot || (local_scope.is_some() && candidate_path != path) {
                continue;
            }
            let declarations = file.navigation_source().map(|(decl, _)| decl);
            if let Some(namespace) = namespace
                && declarations.is_some_and(|decl| decl.refers_all_from(namespace))
            {
                return Err("Rename cannot verify a namespace reached through :refer :all".into());
            }
            if !file.source.contains(name) {
                continue;
            }
            let edited_here = edited
                .get(candidate_path.as_str())
                .map_or(&[][..], Vec::as_slice);
            let sees_unqualified = namespace.is_none_or(|namespace| {
                declarations.is_none_or(|decl| {
                    decl.namespace() == namespace
                        || decl
                            .dependencies
                            .iter()
                            .any(|dependency| dependency == namespace)
                        || qualified
                            .as_ref()
                            .is_some_and(|name| decl.referrals_to(name).next().is_some())
                })
            });
            let references = if candidate_path == path {
                None
            } else {
                Some(self.collect_file_references(
                    candidate_path,
                    file,
                    standard_test,
                    false,
                    &mut providers,
                ))
            };
            let occurrences = references.as_ref().unwrap_or(&origin);
            let shadows: BTreeSet<_> = occurrences
                .bindings
                .iter()
                .chain(occurrences.uses.iter().filter_map(|(span, reference)| {
                    (namespace.is_none() || matches!(reference, ScopedReference::Local(_)))
                        .then_some(span)
                }))
                .map(|span| (span.start.offset, span.end.offset))
                .collect();
            let parsed = reader::parse(&file.source);
            if !edited_here.is_empty() && !parsed.errors.is_empty() {
                return Err(unproven());
            }
            for token in parsed
                .syntax()
                .descendants_with_tokens()
                .filter_map(SyntaxElement::into_token)
            {
                if !matches!(token.kind(), SyntaxKind::Symbol | SyntaxKind::Keyword) {
                    continue;
                }
                let token_range = token_bounds(&token);
                if local_scope
                    .as_ref()
                    .is_some_and(|scope| !scope.contains(&token_range.start))
                    || inert(&token)
                {
                    continue;
                }
                let raw = token.text();
                if let Some(new_name) = new_name
                    && !edited_here.is_empty()
                    && raw.trim_start_matches(':').rsplit('/').next() == Some(new_name)
                    && implicit_key_binding(&token)
                {
                    return Err(format!("The name {new_name} is already used in a binding"));
                }
                if token.kind() != SyntaxKind::Symbol {
                    continue;
                }
                if let Some(new_name) = new_name
                    && !edited_here.is_empty()
                    && (raw == new_name
                        || (namespace.is_some()
                            && qualified_name(raw, new_name, namespace, declarations)))
                {
                    return Err(format!(
                        "The name {new_name} is already used in the affected scope"
                    ));
                }
                let could_be_target = (raw == name && sees_unqualified)
                    || (namespace.is_some() && qualified_name(raw, name, namespace, declarations));
                let covered = edited_here
                    .iter()
                    .any(|edit| token_range.end == edit.end && token_range.start <= edit.start)
                    || shadows.contains(&(token_range.start, token_range.end));
                if could_be_target && !covered {
                    return Err(unproven());
                }
            }
        }
        Ok(Rename {
            selected,
            edits,
            local: local_scope.is_some(),
        })
    }
}

fn plain_name(name: &str) -> bool {
    if name.contains('/')
        || name.starts_with(['.', '%'])
        || name.ends_with('.')
        || matches!(
            name,
            "&" | "if"
                | "do"
                | "recur"
                | "throw"
                | "try"
                | "catch"
                | "finally"
                | "quote"
                | "var"
                | "set!"
                | "def"
                | "let*"
                | "fn*"
                | "loop*"
                | "letfn*"
                | "new"
                | "monitor-enter"
                | "monitor-exit"
                | "deftype*"
                | "reify*"
                | "case*"
                | "import*"
        )
    {
        return false;
    }
    reader::read(name)
        .is_ok_and(|forms| matches!(forms.as_slice(), [form] if form.symbol() == Some(name)))
}

fn ordinary_var(form: &Form, name: Span) -> bool {
    matches!(&form.without_metadata().kind, Kind::List(items)
        if matches!(items.as_slice(), [head, binding, ..]
            if matches!(head.symbol(), Some("def" | "defn" | "defn-" |
                "clojure.core/def" | "clojure.core/defn" | "clojure.core/defn-"))
                && binding.without_metadata().span == name
                && [":dynamic", ":macro", ":inline"].iter().all(|key| binding.metadata(key).is_none())))
}

fn containing_form(forms: &[Form], offset: usize) -> Option<&Form> {
    let form = forms.get(forms.partition_point(|form| form.span.end.offset <= offset))?;
    bounds(form.span).contains(&offset).then_some(form)
}

fn name_span(source: &str, mut span: Span, name: &str) -> Option<Span> {
    let raw = source.get(bounds(span))?;
    if raw == name {
        return Some(span);
    }
    let (qualifier, local) = raw.split_once('/')?;
    if qualifier.is_empty() || local != name {
        return None;
    }
    span.start.offset += qualifier.len() + 1;
    span.start.column += qualifier.chars().count() + 1;
    Some(span)
}

fn qualified_name(
    raw: &str,
    name: &str,
    namespace: Option<&str>,
    decl: Option<&Declarations>,
) -> bool {
    let Some((qualifier, local)) = raw.split_once('/') else {
        return false;
    };
    local == name && decl.is_none_or(|decl| Some(decl.qualifier(qualifier)) == namespace)
}

fn inert(token: &SyntaxToken) -> bool {
    token.parent_ancestors().any(|node| {
        matches!(node.kind(), SyntaxKind::Quote | SyntaxKind::Discard)
            || (node.kind() == SyntaxKind::List
                && node
                    .children_with_tokens()
                    .filter(|element| !element.kind().is_trivia())
                    .nth(1)
                    .and_then(SyntaxElement::into_token)
                    .is_some_and(|head| matches!(head.text(), "quote" | "clojure.core/quote")))
    })
}

fn implicit_key_binding(token: &SyntaxToken) -> bool {
    let Some(vector) = token
        .parent_ancestors()
        .find(|node| node.kind() != SyntaxKind::Metadata)
    else {
        return false;
    };
    if vector.kind() != SyntaxKind::Vector {
        return false;
    }
    let Some(map) = vector
        .parent()
        .filter(|parent| matches!(parent.kind(), SyntaxKind::Map | SyntaxKind::NamespacedMap))
    else {
        return false;
    };
    map.children_with_tokens()
        .take_while(|child| child.text_range() != vector.text_range())
        .filter(|element| !element.kind().is_trivia())
        .last()
        .and_then(SyntaxElement::into_token)
        .is_some_and(|key| {
            key.kind() == SyntaxKind::Keyword
                && matches!(
                    key.text().trim_start_matches(':').rsplit('/').next(),
                    Some("keys" | "strs" | "syms")
                )
        })
}

fn bounds(span: Span) -> Range<usize> {
    span.start.offset..span.end.offset
}

fn token_bounds(token: &SyntaxToken) -> Range<usize> {
    let range = token.text_range();
    usize::from(range.start())..usize::from(range.end())
}
