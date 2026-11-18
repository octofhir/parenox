//! Function contracts read from a bounded subset of clojure.spec forms.
use std::collections::BTreeMap;

use super::{Globals, Model, Signature};
use super::{contracts::track_spec, type_forms::read_function_spec};
use crate::Diagnostic;
use crate::reader::{Form, Kind};

impl Model {
    pub(super) fn collect_fdefs<'a>(
        &mut self,
        indices: &[usize],
        forms: &'a [Form],
    ) -> BTreeMap<String, &'a [Form]> {
        let mut fdefs = BTreeMap::new();
        for &index in indices {
            let form = &forms[index];
            let Kind::List(parts) = &form.without_metadata().kind else {
                unreachable!();
            };
            let Some(symbol) = parts.get(1).and_then(Form::symbol) else {
                self.error("PNX9001", form.span, "s/fdef requires a function symbol");
                continue;
            };
            let name = self.function_name(symbol);
            if name.split_once('/').unwrap().0 != self.names.namespace {
                self.diagnostics.push(Diagnostic::analysis_gap(
                    "PNX9001",
                    parts[1].span,
                    "Type checking for an external s/fdef is unavailable",
                ));
                continue;
            }
            match fdefs.entry(name) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(parts.as_slice());
                }
                std::collections::btree_map::Entry::Occupied(entry) => {
                    self.error(
                        "PNX2790",
                        parts[1].span,
                        format!("Duplicate s/fdef for {}", entry.key()),
                    );
                }
            }
        }
        fdefs
    }

    pub(super) fn parse_fdef(&mut self, parts: &[Form], globals: &Globals) -> Option<Signature> {
        read_function_spec(
            &self.names,
            &parts[2..],
            parts[1].span,
            &mut self.diagnostics,
            |key, _| track_spec(&mut self.external_specs, globals, key),
        )
    }
}
