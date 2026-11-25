//! Explicit Var root writes in executed top-level forms.
use super::Declarations;
use crate::reader::{Form, Kind};

impl Declarations {
    pub(crate) fn collect_root_writes(&self, forms: &[Form], mut record: impl FnMut(&str, &str)) {
        let mut pending = Vec::new();
        for form in forms {
            pending.push(form);
            while let Some(form) = pending.pop() {
                let Kind::List(items) = &form.kind else {
                    continue;
                };
                let [head, tail @ ..] = items.as_slice() else {
                    continue;
                };
                if head.symbol() == Some("do") {
                    pending.extend(tail.iter().rev());
                    continue;
                }
                if let Some((namespace, name)) = self.explicit_root_write(forms, items) {
                    record(namespace, name);
                }
            }
        }
    }

    pub(super) fn explicit_root_write<'a>(
        &'a self,
        forms: &[Form],
        items: &'a [Form],
    ) -> Option<(&'a str, &'a str)> {
        let [head, var, _, ..] = items else {
            return None;
        };
        if !self.is_unshadowed_core_operator(forms, head, "alter-var-root") {
            return None;
        }
        let Kind::List(var) = &var.kind else {
            return None;
        };
        let [operator, target] = var.as_slice() else {
            return None;
        };
        if operator.symbol() != Some("var") {
            return None;
        }
        let symbol = target.symbol()?;
        if symbol != "/" && symbol.contains('/') {
            Some(self.names.resolve_parts(symbol))
        } else if self.all_refers.is_empty()
            || self
                .source_var_before(forms, symbol, target.span.start.offset)
                .is_some()
        {
            Some(self.top_level_call(symbol))
        } else {
            None
        }
    }
}
