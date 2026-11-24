//! Source outline from declarations the analyzer has recognized.
use crate::project::{SourceSymbol, SourceSymbolKind};
use crate::reader::{Form, Kind, Span};

use super::{Declarations, protocol_methods};

impl Declarations {
    pub(crate) fn namespace_selection(&self, forms: &[Form]) -> Option<Span> {
        let end = self.namespace_end?;
        let index = forms.partition_point(|form| form.span.end.offset < end);
        let form = forms
            .get(index)
            .filter(|form| form.span.end.offset == end)?;
        let Kind::List(items) = &form.without_metadata().kind else {
            return None;
        };
        let name = items.get(1)?;
        (name.symbol()? == self.namespace()).then(|| name.without_metadata().span)
    }

    pub(crate) fn outline<'a>(&self, forms: &'a [Form]) -> Vec<SourceSymbol<'a>> {
        use SourceSymbolKind::*;
        let mut symbols = Vec::new();
        for index in 0..forms.len() {
            let form = self.function_form(forms, index);
            let Kind::List(items) = &form.without_metadata().kind else {
                continue;
            };
            let [op, name, tail @ ..] = items.as_slice() else {
                continue;
            };
            let Some(op) = op.symbol() else { continue };
            let kind = if self.namespace_end == Some(form.span.end.offset)
                && matches!(op, "ns" | "clojure.core/ns")
            {
                Namespace
            } else if self.functions.binary_search(&index).is_ok() {
                Function
            } else if self.macros.binary_search(&index).is_ok() {
                Macro
            } else if self.test_forms.binary_search(&index).is_ok() {
                Test
            } else if self.method_forms.binary_search(&index).is_ok() {
                Method
            } else if self.protocols.binary_search(&index).is_ok() {
                Protocol
            } else if self
                .types
                .binary_search_by_key(&index, |kind| kind.form)
                .is_ok()
            {
                Type
            } else if self.literal_vars.binary_search(&index).is_ok()
                || self.unchecked_vars.binary_search(&index).is_ok()
            {
                Variable
            } else if tail.len() == 1
                && self.names.resolve_parts(op) == ("clojure.spec.alpha", "def")
            {
                Spec
            } else {
                continue;
            };
            let Some(label) = (if kind == Spec {
                name.keyword()
            } else {
                name.symbol()
            }) else {
                continue;
            };
            symbols.push(SourceSymbol {
                name: label,
                kind,
                range: form.span,
                selection: name.without_metadata().span,
            });
            if kind == Protocol {
                symbols.extend(protocol_methods(tail).iter().filter_map(|method| {
                    let Kind::List(items) = &method.without_metadata().kind else {
                        return None;
                    };
                    let [name, parameters, ..] = items.as_slice() else {
                        return None;
                    };
                    parameters.vector()?;
                    Some(SourceSymbol {
                        name: name.symbol()?,
                        kind: Method,
                        range: method.span,
                        selection: name.without_metadata().span,
                    })
                }));
            }
        }
        for definition in &self.scoped_functions {
            let Kind::List(items) = &forms[definition.enclosing].without_metadata().kind else {
                continue;
            };
            let form = &items[definition.definition];
            let Kind::List(parts) = &form.without_metadata().kind else {
                continue;
            };
            if let Some(name) = parts[1].symbol() {
                symbols.push(SourceSymbol {
                    name,
                    kind: Function,
                    range: form.span,
                    selection: parts[1].without_metadata().span,
                });
            }
        }
        symbols.sort_unstable_by_key(|symbol| symbol.selection.start.offset);
        symbols
    }
}
