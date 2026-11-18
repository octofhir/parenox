//! Namespace imports from Clojure ns declarations.
use std::collections::{BTreeMap, BTreeSet};

use super::{Declarations, Globals};
use crate::checker::calls::is_builtin;
use crate::reader::{Form, Kind, Span};

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

impl Declarations {
    pub(super) fn read_libspec(&mut self, lib: &Form, globals: Option<&Globals>) {
        if let Some(namespace) = lib.symbol() {
            self.dependencies.push(namespace.to_owned());
            return;
        }
        let Some([name, options @ ..]) = lib.vector() else {
            self.error(
                "PNX9001",
                lib.span,
                "Expected a namespace or require vector",
            );
            return;
        };
        let Some(namespace) = name.symbol() else {
            self.error("PNX9001", name.span, "Expected a required namespace");
            return;
        };
        let Some(options) = self.options(
            options,
            lib.span,
            ":require",
            &[":as", ":as-alias", ":refer", ":exclude", ":rename"],
        ) else {
            return;
        };
        let alias_only = options.contains_key(":as-alias") && !options.contains_key(":as");
        if !alias_only {
            self.dependencies.push(namespace.to_owned());
        }
        for key in [":as", ":as-alias"] {
            if let Some(form) = options.get(key) {
                if let Some(alias) = form.symbol() {
                    self.names
                        .aliases
                        .insert(alias.to_owned(), namespace.to_owned());
                } else {
                    self.error(
                        "PNX9001",
                        form.span,
                        format!("{key} requires an alias symbol"),
                    );
                }
            }
        }
        let Some(refer) = options.get(":refer") else {
            return;
        };
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
        let names: Vec<_> = if refer.keyword() == Some(":all") {
            if alias_only {
                self.error(
                    "PNX9001",
                    refer.span,
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
                    .map(|&name| (name, refer.span))
                    .collect()
            } else {
                let Some(globals) = globals else {
                    return;
                };
                let Some(exports) = globals.exports.get(namespace) else {
                    self.error(
                        "PNX9001",
                        refer.span,
                        format!(":refer :all requires the source of namespace {namespace}"),
                    );
                    return;
                };
                exports
                    .public_var_names()
                    .map(|name| (name, refer.span))
                    .collect()
            }
        } else {
            self.symbols(refer, ":refer")
                .into_iter()
                .map(|(name, form)| (name, form.span))
                .collect()
        };
        for (name, source) in names {
            if excluded.iter().any(|(excluded, _)| *excluded == name) {
                continue;
            }
            let renamed = renames
                .iter()
                .rev()
                .find(|[from, _]| from.symbol() == Some(name))
                .map(|[_, to]| to);
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
                if let Some(name) = form.symbol() {
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

fn known_core_var(name: &str) -> bool {
    is_builtin(&format!("clojure.core/{name}"))
        || matches!(
            name,
            "ns" | "defn"
                | "defn-"
                | "loop"
                | "fn"
                | "let"
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
