//! Namespace imports from Clojure ns declarations.
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use super::{Declarations, Globals, Names};
use crate::Diagnostic;
use crate::checker::calls::is_builtin;
use crate::reader::{Form, Kind, Span};

/// Ordinary namespace load edges from the current, fully read project sources.
#[derive(Clone, Default)]
pub(crate) struct RuntimeLoads(Option<Arc<RuntimeLoadGraph>>);

struct RuntimeLoadGraph {
    requires: BTreeMap<String, BTreeSet<String>>,
    unavailable: BTreeSet<String>,
    unverified_resources: Vec<String>,
}

impl RuntimeLoadGraph {
    fn resource_unavailable(&self, namespace: &str) -> bool {
        self.unavailable.contains(namespace)
            || self
                .unverified_resources
                .iter()
                .any(|path| RuntimeLoads::matches_source_resource(path, namespace))
    }
}

impl RuntimeLoads {
    pub fn from_sources<'a>(
        sources: impl Iterator<Item = (&'a str, Option<(&'a Declarations, &'a [Form])>, bool)>,
    ) -> Self {
        let mut requires = BTreeMap::new();
        let mut unavailable = BTreeSet::new();
        let mut unverified_resources = Vec::new();
        for (path, source, checked) in sources {
            let Some((declarations, forms)) =
                source.filter(|(declarations, _)| declarations.has_namespace_declaration())
            else {
                unverified_resources.push(path.to_owned());
                continue;
            };
            let namespace = declarations.namespace();
            if !checked || !Self::matches_source_resource(path, namespace) {
                unavailable.insert(namespace.to_owned());
                unverified_resources.push(path.to_owned());
                continue;
            }
            if !declarations.names.required_libraries.is_empty()
                && declarations.has_static_requirements(forms)
            {
                requires.insert(
                    namespace.to_owned(),
                    declarations.names.required_libraries.clone(),
                );
            }
        }
        Self(Some(Arc::new(RuntimeLoadGraph {
            requires,
            unavailable,
            unverified_resources,
        })))
    }

    fn matches_source_resource(path: &str, namespace: &str) -> bool {
        let path = Path::new(path);
        let Some(extension @ ("clj" | "cljc")) = path.extension().and_then(|ext| ext.to_str())
        else {
            return false;
        };
        !namespace
            .split('.')
            .any(|part| part.is_empty() || part.contains(['/', '\\']))
            && path.ends_with(format!(
                "{}.{extension}",
                namespace.replace('-', "_").replace('.', "/")
            ))
    }

    pub(super) fn proven_aliases(&self, names: &Names) -> BTreeSet<String> {
        let mut proven = BTreeSet::new();
        let Some(loads) = &self.0 else {
            return proven;
        };
        let mut unresolved: BTreeSet<_> = names
            .aliased_libraries
            .iter()
            .map(String::as_str)
            .filter(|name| !names.directly_loads(name) && !loads.resource_unavailable(name))
            .collect();
        if unresolved.is_empty() {
            return proven;
        }
        let mut pending: Vec<_> = names
            .required_libraries
            .iter()
            .map(String::as_str)
            .collect();
        let mut visited = BTreeSet::from([names.namespace.as_str()]);
        while !unresolved.is_empty() {
            let Some(namespace) = pending.pop() else {
                break;
            };
            if !visited.insert(namespace) || loads.resource_unavailable(namespace) {
                continue;
            }
            if unresolved.remove(namespace) {
                proven.insert(namespace.to_owned());
            }
            if let Some(required) = loads.requires.get(namespace) {
                pending.extend(required.iter().map(String::as_str));
            }
        }
        proven
    }
}

// Public var names from the pinned Clojure/JVM reference. These are name
// declarations for referrals, without signatures or macro expansion.
const CLOJURE_TEST_PUBLIC_VARS: &[&str] = &[
    "*initial-report-counters*",
    "*load-tests*",
    "*report-counters*",
    "*stack-trace-depth*",
    "*test-out*",
    "*testing-contexts*",
    "*testing-vars*",
    "are",
    "assert-any",
    "assert-expr",
    "assert-predicate",
    "compose-fixtures",
    "deftest",
    "deftest-",
    "do-report",
    "file-position",
    "function?",
    "get-possibly-unbound-var",
    "inc-report-counter",
    "is",
    "join-fixtures",
    "report",
    "run-all-tests",
    "run-test",
    "run-test-var",
    "run-tests",
    "set-test",
    "successful?",
    "test-all-vars",
    "test-ns",
    "test-var",
    "test-vars",
    "testing",
    "testing-contexts-str",
    "testing-vars-str",
    "try-expr",
    "use-fixtures",
    "with-test",
    "with-test-out",
];

pub(super) struct ExplicitReferral {
    qualified: String,
    local: String,
    source: Span,
    renaming: Option<[Span; 2]>,
}

impl Declarations {
    pub(crate) fn alias_declaration(&self, alias: &str) -> Option<Span> {
        self.namespace_alias_sites.get(alias).copied()
    }

    pub(crate) fn namespace_usage(&self, namespace: &str) -> Option<Span> {
        self.namespace_usages
            .iter()
            .rfind(|(name, _)| name == namespace)
            .map(|(_, span)| *span)
    }

    pub(crate) fn namespace_reference_at(&self, offset: usize) -> Option<&str> {
        let contains = |span: &Span| (span.start.offset..span.end.offset).contains(&offset);
        self.namespace_usages
            .iter()
            .find(|(_, span)| contains(span))
            .map(|(namespace, _)| namespace.as_str())
            .or_else(|| {
                let (alias, _) = self
                    .namespace_alias_sites
                    .iter()
                    .find(|(_, span)| contains(span))?;
                self.names.aliases.get(alias).map(String::as_str)
            })
    }

    pub(crate) fn referral_declaration(
        &self,
        namespace: &str,
        name: &str,
        local: Option<&str>,
    ) -> Option<Span> {
        let target = format!("{namespace}/{name}");
        self.referrals
            .iter()
            .rfind(|referral| {
                referral.qualified == target && local.is_none_or(|name| name == referral.local)
            })
            .map(|referral| match (local, referral.renaming) {
                (Some(local), Some([_, renamed])) if local != name => renamed,
                _ => referral.source,
            })
    }

    pub(crate) fn refers_all_from(&self, namespace: &str) -> bool {
        self.all_refers.contains(namespace)
    }

    pub(crate) fn referrals_to<'a>(&'a self, target: &'a str) -> impl Iterator<Item = Span> + 'a {
        self.referrals
            .iter()
            .filter(move |referral| referral.qualified == target)
            .flat_map(|referral| {
                std::iter::once(referral.source)
                    .chain(referral.renaming.as_slice().iter().flatten().copied())
            })
    }

    pub(crate) fn referral_at(&self, offset: usize) -> Option<(&str, &str)> {
        let referral = self.referrals.iter().find(|referral| {
            std::iter::once(&referral.source)
                .chain(referral.renaming.as_slice().iter().flatten())
                .any(|span| (span.start.offset..span.end.offset).contains(&offset))
        })?;
        referral.qualified.split_once('/')
    }

    pub(crate) fn check_referrals(
        &self,
        mut missing: impl FnMut(&str, &str) -> bool,
    ) -> Vec<Diagnostic> {
        self.referrals
            .iter()
            .filter_map(|referral| {
                let (namespace, local) = referral.qualified.split_once('/')?;
                missing(namespace, local).then(|| {
                    Diagnostic::new(
                        "PNX2304",
                        referral.source,
                        format!("Unresolved or private project var {}", referral.qualified),
                    )
                })
            })
            .collect()
    }

    pub(super) fn read_libspec(
        &mut self,
        lib: &Form,
        globals: Option<&Globals>,
        refer_by_default: bool,
    ) {
        let group = match &lib.without_metadata().kind {
            Kind::List(items) => Some(items),
            Kind::Vector(items)
                if items.get(1).is_some_and(|second| {
                    second.keyword().is_none()
                        && !matches!(second.without_metadata().kind, Kind::Nil)
                }) =>
            {
                Some(items)
            }
            _ => None,
        };
        let Some(group) = group else {
            self.read_library(lib, None, globals, refer_by_default);
            return;
        };
        let Some((prefix, children)) = group.split_first() else {
            self.error("PNX9001", lib.span, "Expected a library prefix");
            return;
        };
        let Some(prefix_name) = prefix.symbol().filter(|name| !name.contains('/')) else {
            self.error(
                "PNX9001",
                prefix.span,
                "Expected an unqualified library prefix",
            );
            return;
        };
        for child in children {
            self.read_library(child, Some(prefix_name), globals, refer_by_default);
        }
    }

    fn read_library(
        &mut self,
        lib: &Form,
        prefix: Option<&str>,
        globals: Option<&Globals>,
        refer_by_default: bool,
    ) {
        let (name, options) = match &lib.without_metadata().kind {
            Kind::Vector(items) | Kind::List(items) if prefix.is_some() => {
                items.split_first().unwrap_or((lib, &[]))
            }
            Kind::Vector(items) => items.split_first().unwrap_or((lib, &[])),
            _ => (lib, &[][..]),
        };
        let Some(namespace) = unqualified(name) else {
            self.error(
                "PNX9001",
                name.span,
                "Expected a namespace or libspec vector",
            );
            return;
        };
        let full_namespace = if let Some(prefix) = prefix {
            if namespace.contains('/') || namespace.find('.').is_some_and(|dot| dot > 0) {
                self.error(
                    "PNX9001",
                    name.span,
                    "Expected an unqualified library name without an embedded period",
                );
                return;
            }
            Cow::Owned(format!("{prefix}.{namespace}"))
        } else {
            Cow::Borrowed(namespace)
        };
        let namespace = full_namespace.as_ref();
        let (clause, supported): (&str, &[_]) = if refer_by_default {
            (
                ":use",
                &[":as", ":as-alias", ":refer", ":exclude", ":rename", ":only"],
            )
        } else {
            (
                ":require",
                &[":as", ":as-alias", ":refer", ":exclude", ":rename"],
            )
        };
        let Some(options) = self.options(options, lib.span, clause, supported) else {
            return;
        };
        self.namespace_usages
            .push((namespace.to_owned(), name.without_metadata().span));
        let alias_only =
            !refer_by_default && options.contains_key(":as-alias") && !options.contains_key(":as");
        if alias_only {
            self.names.aliased_libraries.insert(namespace.to_owned());
        } else {
            self.names.required_libraries.insert(namespace.to_owned());
        }
        for key in [":as", ":as-alias"] {
            if let Some(form) = options.get(key) {
                if let Some(alias) = unqualified(form) {
                    if let Some(previous) = self.names.aliases.get(alias)
                        && previous != namespace
                    {
                        self.error(
                            "PNX2790",
                            form.span,
                            format!("Namespace alias {alias} already refers to {previous}"),
                        );
                        continue;
                    }
                    self.names
                        .aliases
                        .insert(alias.to_owned(), namespace.to_owned());
                    self.namespace_alias_sites
                        .insert(alias.to_owned(), form.without_metadata().span);
                } else {
                    self.error(
                        "PNX9001",
                        form.span,
                        format!("{key} requires an alias symbol"),
                    );
                }
            }
        }
        let refer = options
            .get(":refer")
            .or_else(|| options.get(":only"))
            .copied();
        if refer.is_none() && !refer_by_default {
            return;
        }
        let refer_span = refer.map_or(lib.span, |form| form.span);
        let excluded = options
            .get(":exclude")
            .map_or_else(Vec::new, |form| self.symbols(form, ":exclude"));
        let renames = if let Some(renames) = options.get(":rename") {
            let Some(pairs) = renames.map() else {
                self.error("PNX9001", renames.span, ":rename requires a map of symbols");
                return;
            };
            pairs.as_chunks::<2>().0
        } else {
            &[]
        };
        let refer_all = refer.is_none_or(|refer| refer.keyword() == Some(":all"));
        let names: Vec<_> = if refer_all {
            if alias_only {
                self.error(
                    "PNX9001",
                    refer_span,
                    ":refer :all depends on whether an :as-alias namespace is loaded",
                );
                return;
            }
            self.all_refers.insert(namespace.to_owned());
            if namespace == "clojure.test"
                && globals.is_none_or(|globals| !globals.project_namespaces.contains(namespace))
            {
                CLOJURE_TEST_PUBLIC_VARS
                    .iter()
                    .map(|&name| (name, refer_span))
                    .collect()
            } else {
                let Some(globals) = globals else {
                    return;
                };
                let Some(exports) = globals.exports.get(namespace) else {
                    self.error(
                        "PNX9001",
                        refer_span,
                        format!("Referring all vars requires the source of namespace {namespace}"),
                    );
                    return;
                };
                exports
                    .public_var_names()
                    .map(|name| (name, refer_span))
                    .collect()
            }
        } else {
            let option = if options.contains_key(":refer") {
                ":refer"
            } else {
                ":only"
            };
            self.symbols(refer.unwrap(), option)
                .into_iter()
                .map(|(name, form)| (name, form.without_metadata().span))
                .collect()
        };
        if alias_only && !names.is_empty() && !self.names.directly_loads(namespace) {
            self.diagnostics.push(Diagnostic::analysis_gap(
                "PNX9003",
                refer_span,
                format!("Referring vars through :as-alias requires {namespace} to be loaded first"),
            ));
        }
        for (name, source) in names {
            if excluded.iter().any(|(excluded, _)| *excluded == name) {
                continue;
            }
            let renaming = renames
                .iter()
                .rfind(|[from, _]| from.symbol() == Some(name));
            let renamed = renaming.map(|[_, to]| to);
            let span = renamed.map_or(source, |form| form.span);
            let alias = renamed.map_or(Some(name), unqualified);
            let Some(alias) = alias.filter(|name| !name.contains('/') || *name == "/") else {
                self.error(
                    "PNX1001",
                    span,
                    "A referred name must be an unqualified symbol",
                );
                continue;
            };
            let qualified = format!("{namespace}/{name}");
            if let Some(class) = self.names.imports.get(alias) {
                self.error(
                    "PNX2790",
                    span,
                    format!("{alias} already refers to class {class}"),
                );
                continue;
            }
            if let Some(previous) = self.names.refers.get(alias)
                && previous != &qualified
                && !previous.starts_with("clojure.core/")
                && namespace != "clojure.core"
            {
                self.error(
                    "PNX2790",
                    span,
                    format!("{alias} already refers to {previous}"),
                );
                continue;
            }
            if !refer_all && !alias_only {
                self.referrals.push(ExplicitReferral {
                    qualified: qualified.clone(),
                    local: alias.to_owned(),
                    source,
                    renaming: renaming
                        .map(|pair| pair.each_ref().map(|form| form.without_metadata().span)),
                });
            }
            self.names.refers.insert(alias.to_owned(), qualified);
        }
    }

    pub(super) fn read_core_refers(&mut self, forms: &[Form], span: Span) {
        let Some(options) = self.options(
            forms,
            span,
            ":refer-clojure",
            &[":only", ":exclude", ":rename"],
        ) else {
            return;
        };
        if let Some(only) = options.get(":only") {
            self.names.core_only = Some(
                self.symbols(only, ":only")
                    .into_iter()
                    .map(|(name, _)| name.to_owned())
                    .collect(),
            );
        }
        if let Some(excluded) = options.get(":exclude") {
            let hidden = self.symbols(excluded, ":exclude");
            self.names
                .core_hidden
                .extend(hidden.into_iter().map(|(name, _)| name.to_owned()));
        }
        let mut renames = Vec::new();
        if let Some(rename) = options.get(":rename") {
            let Some(entries) = rename.map() else {
                self.error("PNX9001", rename.span, ":rename requires a map of symbols");
                return;
            };
            for [from, to] in entries.as_chunks::<2>().0 {
                let (Some(from), Some(alias)) = (unqualified(from), unqualified(to)) else {
                    self.error(
                        "PNX1001",
                        to.span,
                        "Core rename keys and values must be unqualified symbols",
                    );
                    continue;
                };
                if self.names.core_visible(from) {
                    renames.push((from, alias, to.span));
                }
            }
        }
        self.names
            .core_hidden
            .extend(renames.iter().map(|(from, _, _)| (*from).to_owned()));
        let replacements: Vec<_> = self
            .names
            .refers
            .keys()
            .filter(|name| self.core_refers_here(name))
            .cloned()
            .collect();
        for name in replacements {
            self.names.refers.remove(&name);
        }
        let mut aliases = BTreeSet::new();
        for (from, alias, span) in renames {
            let qualified = format!("clojure.core/{from}");
            if !aliases.insert(alias) || self.core_refers_here(alias) {
                self.error(
                    "PNX9001",
                    span,
                    format!("Core rename to {alias} overlaps another referral"),
                );
            } else {
                self.names.refers.insert(alias.into(), qualified);
            }
        }
    }

    fn core_refers_here(&self, name: &str) -> bool {
        self.names.core_visible(name) && (self.names.core_only.is_some() || known_core_var(name))
    }

    fn options<'a>(
        &mut self,
        forms: &'a [Form],
        span: Span,
        context: &str,
        supported: &[&str],
    ) -> Option<BTreeMap<&'a str, &'a Form>> {
        let (pairs, unmatched) = forms.as_chunks::<2>();
        if !unmatched.is_empty() {
            self.error(
                "PNX9001",
                span,
                format!("Expected {context} option/value pairs"),
            );
            return None;
        }
        let mut options = BTreeMap::new();
        for [option, value] in pairs {
            if let Some(key) = option.keyword().filter(|key| supported.contains(key)) {
                options.insert(key, value);
            } else {
                self.error(
                    "PNX9001",
                    option.span,
                    format!("Unsupported {context} option"),
                );
            }
        }
        Some(options)
    }

    fn symbols<'a>(&mut self, form: &'a Form, option: &str) -> Vec<(&'a str, &'a Form)> {
        let (Kind::Vector(forms) | Kind::List(forms)) = &form.without_metadata().kind else {
            self.error(
                "PNX9001",
                form.span,
                format!("{option} requires an explicit list of symbols"),
            );
            return Vec::new();
        };
        forms
            .iter()
            .filter_map(|form| {
                if let Some(name) = unqualified(form) {
                    Some((name, form))
                } else {
                    self.error("PNX9001", form.span, format!("{option} expects a symbol"));
                    None
                }
            })
            .collect()
    }
}

fn unqualified(form: &Form) -> Option<&str> {
    form.symbol()
        .filter(|name| !name.contains('/') || *name == "/")
}

pub(super) fn known_core_var(name: &str) -> bool {
    is_builtin(&format!("clojure.core/{name}"))
        || matches!(
            name,
            "ns" | "defn"
                | "defn-"
                | "loop"
                | "fn"
                | "let"
                | "letfn"
                | "or"
                | "and"
                | "cond"
                | "case"
                | "when"
                | "if-not"
                | "when-not"
                | "if-let"
                | "when-let"
                | "if-some"
                | "when-some"
                | "map?"
                | "vector?"
                | "set?"
                | "->"
                | "->>"
                | "some->"
                | "some->>"
                | "cond->"
                | "cond->>"
                | "as->"
                | "get"
                | "req!"
                | "contains?"
                | "get-in"
                | "assoc"
                | "assoc-in"
                | "update"
                | "update-in"
                | "mapv"
                | "filterv"
                | "fnil"
        )
}
