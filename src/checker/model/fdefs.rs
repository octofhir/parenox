//! Function contracts read from a bounded subset of clojure.spec forms.
use std::collections::{BTreeMap, BTreeSet};

use super::{Globals, Model, Signature};
use crate::checker::function_forms::MAX_FIXED_PARAMETERS;
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
                self.error(
                    "PNX9001",
                    parts[1].span,
                    "s/fdef must target a function in this namespace",
                );
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
        let target = parts[1].span;
        let (pairs, remainder) = parts[2..].as_chunks::<2>();
        if let [key] = remainder {
            self.error("PNX9001", key.span, "s/fdef option requires a value");
            return None;
        }
        let (mut args, mut ret) = (None, None);
        for [key, value] in pairs {
            let slot = match key.keyword() {
                Some(":args") => &mut args,
                Some(":ret") => &mut ret,
                _ => {
                    self.error(
                        "PNX9001",
                        key.span,
                        "Only :args and :ret are supported in s/fdef",
                    );
                    return None;
                }
            };
            if slot.replace(value).is_some() {
                self.error("PNX2790", key.span, "Duplicate s/fdef option");
                return None;
            }
        }
        let (Some(args), Some(ret)) = (args, ret) else {
            self.error("PNX9001", target, "s/fdef requires both :args and :ret");
            return None;
        };
        let Kind::List(items) = &args.without_metadata().kind else {
            self.error(
                "PNX9001",
                args.span,
                "s/fdef :args requires (s/cat :tag Type ...)",
            );
            return None;
        };
        if items
            .first()
            .and_then(Form::symbol)
            .is_none_or(|op| self.resolve(op) != "clojure.spec.alpha/cat")
            || items.len().is_multiple_of(2)
        {
            self.error(
                "PNX9001",
                args.span,
                "s/fdef :args requires (s/cat :tag Type ...)",
            );
            return None;
        }
        let pairs = items[1..].as_chunks::<2>().0;
        let mut tags = BTreeSet::new();
        let mut fixed = Vec::new();
        let mut rest = None;
        for (index, [tag, ty]) in pairs.iter().enumerate() {
            if !tag.keyword().is_some_and(|tag| tags.insert(self.key(tag))) {
                self.error("PNX9001", tag.span, "s/cat requires distinct keyword tags");
                return None;
            }
            if let Kind::List(repeat) = &ty.without_metadata().kind
                && repeat
                    .first()
                    .and_then(Form::symbol)
                    .is_some_and(|op| self.resolve(op) == "clojure.spec.alpha/*")
            {
                if index + 1 != pairs.len() || repeat.len() != 2 {
                    self.error(
                        "PNX9001",
                        ty.span,
                        "s/* requires one type and must be the last s/cat entry",
                    );
                    return None;
                }
                rest = Some(self.contract_type(&repeat[1], globals));
            } else {
                fixed.push(self.contract_type(ty, globals));
            }
        }
        if fixed.len() > MAX_FIXED_PARAMETERS {
            self.error(
                "PNX2554",
                args.span,
                "Clojure/JVM supports at most 20 fixed parameters",
            );
            return None;
        }
        Some(Signature {
            args: fixed,
            rest,
            ret: self.contract_type(ret, globals),
        })
    }
}
