//! Locations in a JVM namespace supplied by the classpath consumer.
use crate::checker::Declarations;
use crate::reader::{self, Conditionals, Form, Kind, Span};

use super::Session;

/// Parsed declarations from a local archive resource, without project checking.
pub struct ArchiveDeclarations {
    declarations: Declarations,
    forms: Vec<Form>,
}

pub(super) fn namespace_declarations(
    source: &str,
    namespace: &str,
    portable: bool,
) -> Option<(Declarations, Vec<Form>)> {
    let conditionals = if portable {
        Conditionals::Select(":clj")
    } else {
        Conditionals::Reject
    };
    let (mut read, error) = reader::read_for_check(source, conditionals, None);
    if error.is_some() {
        return None;
    }
    let original = std::mem::take(&mut read.forms);
    read.forms.reserve(original.len());
    for form in original {
        append_top_level(form, &mut read.forms);
    }
    let declarations = Declarations::read_source(source, &mut read).ok()?;
    (declarations.namespace() == namespace).then_some((declarations, read.forms))
}

fn append_top_level(mut form: Form, forms: &mut Vec<Form>) {
    if let Kind::List(items) = &mut form.kind {
        if items.first().and_then(Form::symbol) == Some("do") {
            for item in std::mem::take(items).into_iter().skip(1) {
                append_top_level(item, forms);
            }
            return;
        }
        // A bare (def name) introduces the var without supplying its root.
        if let [head, name] = items.as_mut_slice()
            && head.symbol() == Some("def")
            && name.symbol().is_some()
        {
            head.kind = Kind::Symbol("clojure.core/declare".to_owned());
        }
    }
    forms.push(form);
}

impl ArchiveDeclarations {
    pub fn read(source: &str, namespace: &str, portable: bool) -> Option<Self> {
        let (declarations, forms) = namespace_declarations(source, namespace, portable)?;
        Some(Self {
            declarations,
            forms,
        })
    }

    /// Select a public var, or the namespace declaration when no var is requested.
    pub fn definition(&self, name: Option<&str>) -> Option<Span> {
        if let Some(name) = name {
            return self.declarations.source_var(&self.forms, name, true);
        }
        self.declarations.namespace_selection(&self.forms)
    }
}

impl Session {
    /// An archive namespace candidate from a libspec's name or alias.
    pub fn archive_namespace_at(&self, path: &str, offset: usize) -> Option<&str> {
        let namespace = self.namespace_reference_at(path, offset)?;
        (!self.has_source_namespace(namespace)).then_some(namespace)
    }
}
