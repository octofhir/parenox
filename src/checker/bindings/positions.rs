//! Positional bindings and seq remainders in vector destructuring.
use super::super::{Checker, Env, Value};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

impl Checker<'_> {
    pub(super) fn bind_positions(
        &mut self,
        forms: &[Form],
        span: Span,
        value: Value,
        env: &mut Env,
    ) {
        let (positions, alias) = match forms {
            [positions @ .., directive, alias] if directive.keyword() == Some(":as") => {
                (positions, Some(alias))
            }
            _ => (forms, None),
        };

        let (positions, mut rest) =
            if let Some(index) = positions.iter().position(|form| form.symbol() == Some("&")) {
                let [pattern] = &positions[index + 1..] else {
                    self.error(
                        "PNX1001",
                        positions[index].span,
                        "& requires one binding pattern before optional :as",
                    );
                    return;
                };
                if !positional_pattern(pattern) {
                    self.error("PNX1001", pattern.span, "Invalid rest binding");
                    return;
                }
                // Clojure uses seq/first/next for all positions when & is
                // present, rather than the nth reads used without &.
                let sequence = self.sequence_call("clojure.core/seq", &value, span);
                (&positions[..index], Some((pattern, sequence)))
            } else {
                (positions, None)
            };
        let mut reported_unsupported = false;
        for (index, pattern) in positions.iter().enumerate() {
            if !positional_pattern(pattern) {
                self.error("PNX1001", pattern.span, "Invalid positional binding");
                continue;
            }
            if let Some((_, remaining)) = &mut rest {
                let item = self.sequence_call("clojure.core/first", remaining, pattern.span);
                *remaining = self.sequence_call("clojure.core/next", remaining, pattern.span);
                self.bind(pattern, item, env);
                continue;
            }
            let ty = value.known.sequential_item(index).unwrap_or_else(|| {
                if !reported_unsupported {
                    self.error(
                        "PNX9001",
                        span,
                        format!(
                            "Positional destructuring of {} is not supported yet",
                            value.ty
                        ),
                    );
                    reported_unsupported = true;
                }
                Type::Unknown
            });
            self.bind(pattern, Value::with_notes(ty, value.notes.clone()), env);
        }
        if let Some((pattern, remaining)) = rest {
            self.bind(pattern, remaining, env);
        }
        if let Some(alias) = alias {
            if alias
                .symbol()
                .is_some_and(|name| !name.contains('/') && name != "&")
            {
                self.bind(alias, value, env);
            } else {
                self.error("PNX1001", alias.span, ":as requires a local symbol");
            }
        }
    }
}

fn positional_pattern(form: &Form) -> bool {
    match &form.without_metadata().kind {
        Kind::Symbol(name) | Kind::AnonymousArgument(name) => name != "&" && !name.contains('/'),
        Kind::Map(_) | Kind::Vector(_) => true,
        _ => false,
    }
}
