//! Protocol method owners and explicit top-level implementation sites.
use super::{Declarations, extension_specs};
use crate::reader::{Form, Kind, Span};

impl Declarations {
    pub(crate) fn protocol_method_owner<'a>(
        &'a self,
        forms: &'a [Form],
        name: &str,
        span: Span,
    ) -> Option<&'a str> {
        let method = self.protocol_vars(forms).find(|var| {
            var.method
                && var.name.symbol() == Some(name)
                && var.name.without_metadata().span == span
        })?;
        let offset = method.owner.span.start.offset;
        let index = forms.partition_point(|form| form.span.end.offset <= offset);
        let Kind::List(items) = &forms.get(index)?.without_metadata().kind else {
            return None;
        };
        if self.core_registration(forms, items.first()?)? != "defprotocol" {
            return None;
        }
        method.owner.symbol()
    }

    pub(crate) fn protocol_method_targets<'a>(
        &'a self,
        forms: &'a [Form],
        method: &str,
    ) -> Vec<(&'a str, Span)> {
        let mut targets = Vec::new();
        let indices = self
            .types
            .iter()
            .map(|kind| kind.form)
            .chain(self.protocol_extensions.iter().copied());
        for index in indices {
            let Kind::List(items) = &forms[index].without_metadata().kind else {
                continue;
            };
            let [head, target, tail @ ..] = items.as_slice() else {
                continue;
            };
            let (grouped, owner) = match self.core_registration(forms, head) {
                Some("defrecord" | "deftype") => {
                    let [_, groups @ ..] = tail else { continue };
                    (groups, None)
                }
                Some("extend-type") => (tail, None),
                Some("extend-protocol") => (tail, target.symbol()),
                _ => continue,
            };
            collect_protocol_group_methods(grouped, owner, method, &mut targets);
        }
        targets
    }

    fn core_registration<'a>(&'a self, forms: &[Form], head: &'a Form) -> Option<&'a str> {
        let symbol = head.symbol()?;
        let (namespace, name) = self.names.resolve_parts(symbol);
        (namespace == "clojure.core"
            && (symbol.contains('/')
                || (self.all_refers.is_empty()
                    && self
                        .source_var_before(forms, symbol, head.span.start.offset)
                        .is_none()
                    && self.source_declarations(forms, symbol).nth(1).is_none())))
        .then_some(name)
    }
}

pub(in crate::checker::model) fn collect_protocol_group_methods<'a>(
    groups: &'a [Form],
    owner: Option<&'a str>,
    method: &str,
    targets: &mut Vec<(&'a str, Span)>,
) {
    if !extension_specs(groups, owner.is_some()) {
        return;
    }
    let mut protocol = owner;
    for part in groups {
        if let Kind::List(method_form) = &part.without_metadata().kind {
            if let Some(name) = method_form.first()
                && name.symbol() == Some(method)
                && let Some(protocol) = protocol
            {
                targets.push((protocol, name.without_metadata().span));
            }
        } else if owner.is_none() {
            // A new group can name an interface or an unresolved symbol.
            protocol = part.symbol();
        }
    }
}
