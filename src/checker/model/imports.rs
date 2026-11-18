//! Static JVM class names introduced by an ns import clause.
use super::Declarations;
use crate::reader::{Form, Kind, Span};

impl Declarations {
    pub(super) fn read_import(&mut self, form: &Form) {
        match &form.without_metadata().kind {
            Kind::Symbol(class) => self.import_class(class, form.span),
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
                    self.import_class(&format!("{package}.{name}"), class.span);
                }
            }
            _ => self.error(
                "PNX1001",
                form.span,
                "Expected a qualified JVM class or an import group",
            ),
        }
    }

    fn import_class(&mut self, qualified: &str, span: Span) {
        let Some((_, short)) = qualified.rsplit_once('.').filter(|(package, class)| {
            valid_package(package) && !class.is_empty() && !class.contains('/')
        }) else {
            self.error(
                "PNX1001",
                span,
                "An import requires a fully qualified JVM class name",
            );
            return;
        };
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
        }
    }
}

fn valid_package(name: &str) -> bool {
    !name.contains('/') && name.split('.').all(|segment| !segment.is_empty())
}
