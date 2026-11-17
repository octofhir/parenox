//! Positional bindings for vectors and map entries.
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

        if let Some(rest) = positions.iter().find(|form| form.symbol() == Some("&")) {
            self.error("PNX9001", rest.span, "Rest bindings are not supported yet");
            return;
        }
        let mut reported_unsupported = false;
        for (index, pattern) in positions.iter().enumerate() {
            match &pattern.without_metadata().kind {
                Kind::Symbol(name) if !name.contains('/') => {}
                Kind::Map(_) | Kind::Vector(_) => {}
                _ => {
                    self.error("PNX1001", pattern.span, "Invalid positional binding");
                    continue;
                }
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
