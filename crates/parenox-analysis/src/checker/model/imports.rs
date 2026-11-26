//! Static JVM class names introduced by an ns import clause.
use std::{collections::HashMap, sync::LazyLock};

use super::{Declarations, Globals, Model};
use crate::reader::{Form, Kind, array_class_component, read_edn};

/// The source token of the effective symbolic type hint.
pub(crate) fn java_type_tag_at(form: &Form, offset: usize) -> Option<&Form> {
    let tag = form.metadata(":tag")?;
    (matches!(tag.kind, Kind::Symbol(_))
        && (tag.span.start.offset..tag.span.end.offset).contains(&offset))
    .then_some(tag)
}

pub(crate) fn bare_dotted_name(name: &str) -> bool {
    name.contains('.') && !name.contains('/') && !name.starts_with('.') && !name.ends_with('.')
}

impl Declarations {
    pub(crate) fn java_import_at(&self, offset: usize) -> Option<&str> {
        self.java_import_sites
            .iter()
            .find(|(span, _)| (span.start.offset..span.end.offset).contains(&offset))
            .and_then(|(_, short)| self.names.imports.get(short).map(String::as_str))
    }

    pub(crate) fn java_completion_candidate<'a>(&'a self, qualifier: &'a str) -> Option<&'a str> {
        if self.names.aliases.contains_key(qualifier) || self.namespace_usage(qualifier).is_some() {
            return None;
        }
        self.java_class_candidate(qualifier)
    }

    pub(crate) fn java_class_candidate<'a>(&'a self, name: &'a str) -> Option<&'a str> {
        self.names
            .imports
            .get(name)
            .map(String::as_str)
            .or_else(|| implicit_java_class(name))
            .or_else(|| qualified_java_name(name).then_some(name))
    }

    pub(super) fn read_import(&mut self, form: &Form) {
        match &form.without_metadata().kind {
            Kind::Symbol(class) => self.import_class(class, form),
            Kind::List(parts) | Kind::Vector(parts) => {
                let [package, class, more @ ..] = parts.as_slice() else {
                    self.error(
                        "PNX1001",
                        form.span,
                        "An import group requires a package and at least one class",
                    );
                    return;
                };
                let Some(package) = package.symbol().filter(|name| valid_package(name)) else {
                    self.error("PNX1001", package.span, "Expected a JVM package name");
                    return;
                };
                for class in std::iter::once(class).chain(more) {
                    let Some(name) = class.symbol().filter(|name| !name.contains(['.', '/']))
                    else {
                        self.error(
                            "PNX1001",
                            class.span,
                            "Expected a short JVM class name in the import group",
                        );
                        continue;
                    };
                    self.import_class(&format!("{package}.{name}"), class);
                }
            }
            _ => self.error(
                "PNX1001",
                form.span,
                "Expected a qualified JVM class or an import group",
            ),
        }
    }

    fn import_class(&mut self, qualified: &str, form: &Form) {
        let span = form.span;
        let (package, short) = qualified
            .rsplit_once('.')
            .map_or((None, qualified), |(package, class)| (Some(package), class));
        if !package.is_none_or(valid_package) || short.is_empty() || short.contains('/') {
            self.error("PNX1001", span, "An import requires a JVM class name");
            return;
        }
        if let Some(previous) = self.names.imports.get(short)
            && previous != qualified
        {
            self.error(
                "PNX2790",
                span,
                format!("{short} already refers to class {previous}"),
            );
        } else if let Some(previous) = self.names.refers.get(short) {
            self.error(
                "PNX2790",
                span,
                format!("{short} already refers to var {previous}"),
            );
        } else {
            self.names.imports.insert(short.into(), qualified.into());
            self.java_import_sites
                .push((form.without_metadata().span, short.into()));
        }
    }
}

fn valid_package(name: &str) -> bool {
    !name.contains('/') && name.split('.').all(|segment| !segment.is_empty())
}

fn qualified_java_name(name: &str) -> bool {
    name.contains('.')
        && name.split('.').all(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|ch| ch.is_alphabetic() || matches!(ch, '_' | '$'))
                && chars.all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '$'))
        })
}

fn implicit_java_class(name: &str) -> Option<&'static str> {
    static IMPORTS: LazyLock<HashMap<Box<str>, Box<str>>> = LazyLock::new(|| {
        let forms = read_edn(include_str!("imports/jvm.edn")).expect("valid JVM import catalog");
        forms[0]
            .vector()
            .expect("JVM import catalog is a vector")
            .iter()
            .map(|form| {
                let Kind::String(qualified) = &form.kind else {
                    unreachable!("JVM class names are strings");
                };
                let short = qualified.rsplit('.').next().unwrap();
                (Box::from(short), Box::from(qualified.as_str()))
            })
            .collect()
    });
    IMPORTS.get(name).map(Box::as_ref)
}

impl Model {
    pub(in crate::checker) fn known_java_member(&self, symbol: &str, globals: &Globals) -> bool {
        let Some((qualifier, _)) = symbol.split_once('/') else {
            return false;
        };
        self.known_java_class(qualifier)
            && array_class_component(symbol).is_none()
            && self.unshadowed_java_qualifier(qualifier, globals)
    }

    pub(in crate::checker) fn known_java_array_class(&self, name: &str, globals: &Globals) -> bool {
        array_class_component(name).is_some_and(|component| {
            (self.known_java_class(component)
                || matches!(
                    component,
                    "boolean" | "byte" | "char" | "short" | "int" | "long" | "float" | "double"
                ))
                && self.unshadowed_java_qualifier(component, globals)
        })
    }

    fn unshadowed_java_qualifier(&self, qualifier: &str, globals: &Globals) -> bool {
        !self.names.aliases.contains_key(qualifier)
            && !globals.has_competing_class_namespace(qualifier)
    }

    pub(in crate::checker) fn known_java_class(&self, name: &str) -> bool {
        let short = name.rsplit('.').next().unwrap();
        matches!(
            name,
            "java.util.Date" | "java.io.InputStream" | "java.util.Iterator"
        ) || implicit_java_class(short).is_some_and(|qualified| name == short || name == qualified)
            || self.names.imports.contains_key(name)
            || self
                .names
                .imports
                .values()
                .any(|qualified| qualified == name)
    }

    pub(in crate::checker) fn possible_java_class(name: &str) -> bool {
        qualified_java_name(name)
            || (!name.contains('/') && name.chars().next().is_some_and(char::is_uppercase))
    }
}
