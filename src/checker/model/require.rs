//! Namespace imports from Clojure ns declarations.
use std::collections::BTreeMap;

use super::Declarations;
use crate::reader::{Form, Kind};

impl Declarations {
    pub(super) fn read_libspec(&mut self, lib: &Form) {
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
        let (pairs, unmatched) = options.as_chunks::<2>();
        if !unmatched.is_empty() {
            self.error("PNX9001", lib.span, "Expected require option/value pairs");
            return;
        }
        let mut options = BTreeMap::new();
        for [option, value] in pairs {
            if let Some(key @ (":as" | ":as-alias" | ":refer" | ":exclude" | ":rename")) =
                option.keyword()
            {
                options.insert(key, value);
            } else {
                self.error("PNX9001", option.span, "Unsupported require option");
            }
        }
        if !options.contains_key(":as-alias") || options.contains_key(":as") {
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
        let names = self.symbols(refer, ":refer");
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
        for (name, source) in names {
            if excluded.iter().any(|(excluded, _)| *excluded == name) {
                continue;
            }
            let local = renames
                .iter()
                .rev()
                .find(|[from, _]| from.symbol() == Some(name))
                .map_or(source, |[_, to]| to);
            let Some(alias) = local
                .symbol()
                .filter(|name| !name.contains('/') || *name == "/")
            else {
                self.error(
                    "PNX1001",
                    local.span,
                    "A referred name must be an unqualified symbol",
                );
                continue;
            };
            let qualified = format!("{namespace}/{name}");
            if let Some(previous) = self.names.refers.get(alias)
                && previous != &qualified
                && !previous.starts_with("clojure.core/")
            {
                self.error(
                    "PNX2790",
                    local.span,
                    format!("{alias} already refers to {previous}"),
                );
                continue;
            }
            self.names.refers.insert(alias.to_owned(), qualified);
        }
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
