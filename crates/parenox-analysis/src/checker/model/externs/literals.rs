//! Canonical inert EDN for literal vars, independent of their inferred type.
use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;

use super::{ExternalDeclaration, Globals, Model, Names};
use crate::checker::model::{type_forms::inverted_literal, vars::def_initializer};
use crate::checker::{Checker, Declarations};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;
use crate::{ContractKind, ContractOrigin};

pub(super) type Encoded = Result<String, &'static str>;

pub(crate) type SourceLiterals<'a> = BTreeMap<String, (&'a str, Encoded)>;

#[derive(PartialEq, Eq)]
pub(crate) struct LiteralExtern {
    pub ty: Type,
    pub edn: String,
    pub origin: ContractOrigin,
}

impl Model {
    pub(super) fn read_extern_literal(
        &mut self,
        value: &Form,
        name: &str,
        path: &str,
        source: &str,
        globals: &Globals,
    ) -> Option<ExternalDeclaration> {
        let edn = match encode(value, source, &self.names, Context::Edn) {
            Ok(edn) => edn,
            Err(reason) => {
                self.error("PNX9001", value.span, reason);
                return None;
            }
        };
        let mut checker = Checker::new(self, globals, source);
        let ty = checker.quoted_data(value).known;
        if !checker.diagnostics.is_empty() {
            self.diagnostics.extend(checker.diagnostics);
            return None;
        }
        Some(ExternalDeclaration::Literal(Arc::new(LiteralExtern {
            ty,
            edn,
            origin: ContractOrigin {
                name: name.into(),
                kind: ContractKind::ExternLiteral,
                path: Some(path.into()),
            },
        })))
    }

    pub(crate) fn public_literals<'a>(
        &'a self,
        declarations: &'a Declarations,
        forms: &'a [Form],
        source: &'a str,
    ) -> impl Iterator<Item = (&'a str, Span, Encoded)> {
        declarations.literal_vars.iter().filter_map(move |&index| {
            let Kind::List(parts) = &forms[index].without_metadata().kind else {
                unreachable!()
            };
            let name_form = parts.get(1)?;
            let symbol = name_form.symbol()?;
            let qualified = format!("{}/{symbol}", self.names.namespace);
            let (name, _) = self.exports.literals.get_key_value(&qualified)?;
            let initializer = def_initializer(&parts[2..])?;
            let encoded = if super::is_modeled_call(name) {
                Err("externs cannot replace a built-in checking rule")
            } else {
                let expanded = if self.map_invert_in_project == Some(false) {
                    inverted_literal(declarations, forms, source, initializer)
                } else {
                    None
                };
                encode(
                    expanded
                        .as_ref()
                        .or_else(|| self.captured_initializers.get(&index))
                        .unwrap_or(initializer),
                    source,
                    &self.names,
                    Context::Initializer,
                )
            };
            Some((name.as_str(), name_form.span, encoded))
        })
    }
}

#[derive(Clone, Copy)]
enum Context {
    Initializer,
    Quoted,
    Edn,
}

fn encode(form: &Form, source: &str, names: &Names, context: Context) -> Encoded {
    let print = |form: &Form| encode(form, source, names, context);
    let quoted = |form: &Form| encode(form, source, names, Context::Quoted);
    let sequence = |forms: &[Form]| forms.iter().map(print).collect::<Result<Vec<_>, _>>();
    if matches!(context, Context::Edn) && matches!(form.kind, Kind::Metadata(..) | Kind::Quote(_)) {
        return Err("Extern literal values must use EDN data, without metadata or quote shorthand");
    }
    let form = form.without_metadata();
    Ok(match &form.kind {
        Kind::Nil => "nil".into(),
        Kind::Bool(value) => value.to_string(),
        Kind::Number => source[form.span.start.offset..form.span.end.offset].to_owned(),
        Kind::SymbolicNumber(number) => number.spelling().into(),
        Kind::String(value) => {
            serde_json::to_string(value).expect("string serialization is infallible")
        }
        Kind::Utf16String(units) => {
            let mut edn = String::from("\"");
            for unit in units {
                write!(edn, "\\u{unit:04x}").expect("writing to String is infallible");
            }
            edn.push('"');
            edn
        }
        Kind::Character(value) => format!("\\u{:04x}", u32::from(*value)),
        Kind::Keyword(value) => names.key(value),
        Kind::Symbol(value) if !matches!(context, Context::Initializer) => value.clone(),
        Kind::Quote(value) if matches!(context, Context::Initializer) => quoted(value)?,
        Kind::Quote(value) => format!("(quote {})", quoted(value)?),
        Kind::List(items) if matches!(context, Context::Initializer) => match items.as_slice() {
            [] => "()".into(),
            [head, value] if head.symbol() == Some("quote") => quoted(value)?,
            _ => return Err("Extern values must be inert literal data"),
        },
        Kind::List(items) => format!("({})", sequence(items)?.join(" ")),
        Kind::Vector(items) => format!("[{}]", sequence(items)?.join(" ")),
        Kind::Set(items) => {
            let mut items = sequence(items)?;
            items.sort_unstable();
            if items.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err("Duplicate set member in a literal value");
            }
            format!("#{{{}}}", items.join(" "))
        }
        Kind::Map(items) => {
            let mut entries = items
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[key, val]| Ok((print(key)?, print(val)?)))
                .collect::<Result<Vec<_>, _>>()?;
            entries.sort_unstable();
            let fields: Vec<_> = entries
                .into_iter()
                .map(|(key, val)| format!("{key} {val}"))
                .collect();
            format!("{{{}}}", fields.join(" "))
        }
        _ => return Err("Extern values must be inert literal data"),
    })
}
