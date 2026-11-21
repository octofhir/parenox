//! Completion queries for namespace requires.
use std::collections::BTreeSet;

use crate::checker::{Declarations, ScopedReference};
use crate::reader::{Form, Kind};

use super::{Session, symbol_at};

pub(super) enum LiteralRequireSite<'a> {
    Namespace(&'a str),
    Var(&'a str, &'a str),
}

impl Session {
    /// Complete namespace declarations from the current source snapshot.
    pub fn complete_require_namespace(&self, path: &str, prefix: &str, limit: usize) -> Vec<&str> {
        self.files
            .iter()
            .filter(|(candidate, file)| {
                candidate.as_str() != path && file.snapshot == self.snapshot
            })
            .filter_map(|(_, file)| {
                let (declarations, forms) = file.navigation_source()?;
                let namespace = declarations.namespace();
                (namespace.starts_with(prefix) && declares_namespace(forms, namespace))
                    .then_some(namespace)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(limit)
            .collect()
    }

    /// Complete public source vars for a require's explicit `:refer` list.
    /// `None` lets the caller consult dependency archives for an unindexed namespace.
    pub fn complete_require_refer(
        &self,
        namespace: &str,
        prefix: &str,
        limit: usize,
    ) -> Option<Vec<&str>> {
        let Some((_, _, declarations, forms)) = self.navigation_provider(namespace) else {
            return self.has_source_namespace(namespace).then(Vec::new);
        };
        Some(
            declarations
                .top_level_names(forms)
                .into_iter()
                .filter(|name| {
                    name.starts_with(prefix) && declarations.source_var(forms, name, true).is_some()
                })
                .take(limit)
                .collect(),
        )
    }

    /// Find a top-level literal require introducing this alias before the cursor.
    pub fn completion_require_alias(
        &self,
        path: &str,
        qualifier: &str,
        offset: usize,
    ) -> Option<&str> {
        let (declarations, forms) = self.current_file(path)?.navigation_source()?;
        if declarations.alias_declaration(qualifier).is_some() {
            return None;
        }
        find_literal_require(declarations, forms, offset, |found, spec, completed| {
            if completed && spec.aliases().any(|name| name.symbol() == Some(qualifier)) {
                found.get_or_insert(spec.namespace());
            }
        })
    }

    pub(super) fn literal_require_site(
        &self,
        path: &str,
        offset: usize,
    ) -> Option<LiteralRequireSite<'_>> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        let current = forms.partition_point(|form| form.span.end.offset <= offset);
        let Kind::List(items) = &forms.get(current)?.without_metadata().kind else {
            return None;
        };
        if !matches!(
            items.first().and_then(Form::symbol),
            Some("require" | "clojure.core/require")
        ) {
            return None;
        }
        find_literal_require(declarations, forms, offset, |found, spec, completed| {
            if !completed && let Some(site) = spec.site(offset) {
                *found = Some(site);
            }
        })
    }

    pub(super) fn literal_require_qualified_var(
        &self,
        path: &str,
        offset: usize,
    ) -> Option<(&str, &str)> {
        let file = self.current_file(path)?;
        let (declarations, forms) = file.navigation_source()?;
        if !declarations.has_top_level_require || !file.source.is_char_boundary(offset) {
            return None;
        }
        let (qualifier, name) = symbol_at(forms, offset)?.split_once('/')?;
        if qualifier.is_empty() || name.is_empty() || name.contains('/') {
            return None;
        }
        Some((
            self.completion_require_alias(path, qualifier, offset)?,
            name,
        ))
    }

    pub(super) fn literal_require_referred_var(
        &self,
        path: &str,
        offset: usize,
    ) -> Option<(&str, &str)> {
        let file = self.current_file(path)?;
        let (declarations, forms) = file.navigation_source()?;
        if !declarations.has_top_level_require || !file.source.is_char_boundary(offset) {
            return None;
        }
        let name = symbol_at(forms, offset)?;
        if name.contains('/') {
            return None;
        }
        let reference = declarations
            .scoped_reference_at(
                forms,
                offset,
                self.has_standard_test(),
                |namespace, local, arity| self.navigation_call(namespace, local, arity),
            )
            .or_else(|| self.top_level_reference(declarations, forms, name, offset))?;
        if !matches!(reference, ScopedReference::InNamespace(_))
            || declarations
                .source_var_before(forms, name, offset)
                .is_some()
        {
            return None;
        }
        find_literal_require(declarations, forms, offset, |found, spec, completed| {
            if completed && let Some(refer) = spec.option(":refer") {
                if refer.keyword() == Some(":all") {
                    *found = None;
                } else if sequence(refer)
                    .is_some_and(|names| names.iter().any(|item| item.symbol() == Some(name)))
                {
                    *found = Some((spec.namespace(), name));
                }
            }
        })
    }
}

fn find_literal_require<'a, T>(
    declarations: &'a Declarations,
    forms: &'a [Form],
    offset: usize,
    mut inspect: impl FnMut(&mut Option<T>, LiteralLibspec<'a>, bool),
) -> Option<T> {
    if !declarations.has_top_level_require {
        return None;
    }
    let mut found = None;
    let core_namespace = declarations.qualifier("clojure.core") == "clojure.core";
    let mut core_qualified = core_namespace;
    let mut core_unqualified = true;
    for form in forms
        .iter()
        .take_while(|form| form.span.start.offset <= offset)
    {
        let Kind::List(items) = &form.without_metadata().kind else {
            continue;
        };
        let Some((head, args)) = items.split_first() else {
            continue;
        };
        let completed = form.span.end.offset <= offset;
        match head.symbol() {
            Some("ns" | "clojure.core/ns") if completed => {
                found = None;
                core_qualified = core_namespace;
                core_unqualified = true;
                continue;
            }
            Some("in-ns" | "clojure.core/in-ns") if completed => return None,
            Some("clojure.core/require") if core_qualified => {}
            Some("require")
                if core_unqualified
                    && declarations.core_completion_visible(
                        forms,
                        "require",
                        head.span.start.offset,
                    ) => {}
            _ => continue,
        }
        if args
            .iter()
            .any(|arg| !require_flag(arg) && literal_libspec(arg).is_none())
        {
            continue;
        }
        for spec in args.iter().filter_map(literal_libspec) {
            inspect(&mut found, spec, completed);
            core_qualified &= spec.namespace() == "clojure.core"
                || !spec
                    .aliases()
                    .any(|name| name.symbol() == Some("clojure.core"));
            if let Some(refer) = spec.option(":refer") {
                core_unqualified &= refer.keyword() != Some(":all")
                    && !sequence(refer).is_some_and(|names| {
                        names.iter().any(|name| name.symbol() == Some("require"))
                    });
            }
        }
    }
    found
}

fn declares_namespace(forms: &[Form], namespace: &str) -> bool {
    forms.iter().any(|form| {
        matches!(&form.without_metadata().kind, Kind::List(items)
            if matches!(items.as_slice(), [head, name, ..]
                if matches!(head.symbol(), Some("ns" | "clojure.core/ns"))
                && unqualified_symbol(name) == Some(namespace)))
    })
}

#[derive(Clone, Copy)]
struct LiteralLibspec<'a> {
    name: &'a Form,
    options: &'a [[Form; 2]],
}

impl<'a> LiteralLibspec<'a> {
    fn namespace(self) -> &'a str {
        self.name.symbol().unwrap()
    }

    fn option(self, name: &str) -> Option<&'a Form> {
        self.options
            .iter()
            .rfind(|[option, _]| option.keyword() == Some(name))
            .map(|[_, value]| value)
    }

    fn aliases(self) -> impl Iterator<Item = &'a Form> {
        [":as", ":as-alias"]
            .into_iter()
            .filter_map(move |key| self.option(key))
    }

    fn site(self, offset: usize) -> Option<LiteralRequireSite<'a>> {
        let contains = |form: &Form| {
            let span = form.without_metadata().span;
            (span.start.offset..span.end.offset).contains(&offset)
        };
        let namespace = self.namespace();
        if contains(self.name) || self.aliases().any(contains) {
            return Some(LiteralRequireSite::Namespace(namespace));
        }
        let name = sequence(self.option(":refer")?)?
            .iter()
            .find(|name| contains(name))?
            .symbol()?;
        Some(LiteralRequireSite::Var(namespace, name))
    }
}

fn literal_libspec(form: &Form) -> Option<LiteralLibspec<'_>> {
    let quoted = match &form.without_metadata().kind {
        Kind::Quote(quoted) => quoted,
        Kind::List(items) => match items.as_slice() {
            [head, quoted] if head.symbol() == Some("quote") => quoted,
            _ => return None,
        },
        _ => return None,
    };
    if unqualified_symbol(quoted).is_some() {
        return Some(LiteralLibspec {
            name: quoted,
            options: &[],
        });
    }
    let [head, options @ ..] = quoted.vector()? else {
        return None;
    };
    unqualified_symbol(head)?;
    let (options, remaining) = options.as_chunks::<2>();
    let valid = remaining.is_empty()
        && options.iter().all(|[key, value]| match key.keyword() {
            Some(":as" | ":as-alias") => unqualified_symbol(value).is_some(),
            Some(":refer") => {
                value.keyword() == Some(":all")
                    || sequence(value).is_some_and(|forms| {
                        forms.iter().all(|name| unqualified_symbol(name).is_some())
                    })
            }
            _ => false,
        });
    valid.then_some(LiteralLibspec {
        name: head,
        options,
    })
}

fn require_flag(form: &Form) -> bool {
    matches!(form.keyword(), Some(":reload" | ":reload-all" | ":verbose"))
}

fn unqualified_symbol(form: &Form) -> Option<&str> {
    form.symbol().filter(|name| !name.contains('/'))
}

fn sequence(form: &Form) -> Option<&[Form]> {
    match &form.without_metadata().kind {
        Kind::List(items) | Kind::Vector(items) => Some(items),
        _ => None,
    }
}
