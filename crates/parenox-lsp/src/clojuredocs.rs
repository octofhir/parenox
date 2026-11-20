//! Index an inert ClojureDocs snapshot and serve its raw protocol entries.
use std::collections::BTreeMap;

use parenox_analysis::reader::{self, Form, Kind};
use serde_json::{Map, Value};

#[derive(Default)]
pub struct ClojureDocs {
    entries: BTreeMap<String, Value>,
}

impl ClojureDocs {
    pub fn parse(source: &str) -> Result<Self, String> {
        let forms = reader::read_edn(source).map_err(|error| {
            format!(
                "{}:{}: {}",
                error.span.start.line, error.span.start.column, error.message
            )
        })?;
        let [root] = forms.as_slice() else {
            return Err("expected one ClojureDocs EDN map".into());
        };
        let pairs = entries(root).ok_or("expected one ClojureDocs EDN map")?;
        let mut result = Self::default();
        for [key, entry] in pairs {
            let name = keyword(key)
                .filter(|key| {
                    key.split_once('/')
                        .is_some_and(|(ns, name)| !ns.is_empty() && !name.is_empty())
                })
                .ok_or("expected a namespace-qualified ClojureDocs keyword")?;
            let page = json_map(source, entry)
                .ok_or_else(|| format!("invalid ClojureDocs entry for {name}"))?;
            if result
                .entries
                .insert(name.to_owned(), Value::Object(page))
                .is_some()
            {
                return Err(format!("duplicate ClojureDocs entry for {name}"));
            }
        }
        Ok(result)
    }

    pub fn raw(&self, params: &Value) -> Option<&Value> {
        let namespace = params
            .get("symNs")
            .or_else(|| params.get("sym-ns"))?
            .as_str()?;
        let name = params
            .get("symName")
            .or_else(|| params.get("sym-name"))?
            .as_str()?;
        self.find(&format!("{namespace}/{name}"))
    }

    pub fn find(&self, qualified_name: &str) -> Option<&Value> {
        self.entries.get(qualified_name)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn keyword(form: &Form) -> Option<&str> {
    form.keyword()?.strip_prefix(':')
}

fn entries(form: &Form) -> Option<&[[Form; 2]]> {
    Some(form.map()?.as_chunks::<2>().0)
}

fn json_map(source: &str, map: &Form) -> Option<Map<String, Value>> {
    entries(map)?
        .iter()
        .map(|[key, value]| Some((camel_case(keyword(key)?), json_value(source, value)?)))
        .collect()
}

fn json_value(source: &str, form: &Form) -> Option<Value> {
    Some(match &form.kind {
        Kind::Nil => Value::Null,
        Kind::Bool(value) => Value::Bool(*value),
        Kind::Number => {
            let token = &source[form.span.start.offset..form.span.end.offset];
            Value::Number(token.parse().ok()?)
        }
        Kind::String(value) | Kind::Symbol(value) => Value::String(value.clone()),
        Kind::Utf16String(value) => Value::String(String::from_utf16_lossy(value)),
        Kind::Keyword(_) => Value::String(keyword(form)?.to_owned()),
        Kind::Vector(items) | Kind::List(items) => Value::Array(
            items
                .iter()
                .map(|item| json_value(source, item))
                .collect::<Option<_>>()?,
        ),
        Kind::Map(_) => Value::Object(json_map(source, form)?),
        _ => return None,
    })
}

fn camel_case(key: &str) -> String {
    let mut words = key.split('-');
    let mut result = words.next().unwrap_or_default().to_owned();
    for word in words {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.extend(first.to_uppercase());
            result.push_str(chars.as_str());
        }
    }
    result
}
