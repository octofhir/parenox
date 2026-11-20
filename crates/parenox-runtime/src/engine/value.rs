//! Immutable values and printing for the Parenox evaluator.
use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use parenox_frontend::reader::MAX_NESTING_DEPTH;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Value {
    Nil,
    Bool(bool),
    Integer(i64),
    String(Arc<str>),
    Keyword {
        namespace: Option<Arc<str>>,
        name: Arc<str>,
    },
    Symbol(Arc<str>),
    Vector(Collection<Value>),
    List(Collection<Value>),
    Map(Collection<(Value, Value)>),
    Var {
        namespace: Arc<str>,
        name: Arc<str>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Collection<T> {
    items: Arc<[T]>,
    depth: usize,
}

impl<T> Collection<T> {
    fn checked(items: Vec<T>, child_depth: usize) -> Option<Self> {
        let depth = child_depth + 1;
        (depth <= MAX_NESTING_DEPTH).then(|| Self {
            items: items.into(),
            depth,
        })
    }

    pub(super) fn items(&self) -> &[T] {
        &self.items
    }
}

impl Collection<Value> {
    fn sequence(items: Vec<Value>) -> Option<Self> {
        let child_depth = items.iter().map(Value::depth).max().unwrap_or(0);
        Self::checked(items, child_depth)
    }
}

impl Value {
    pub(super) fn keyword(spelling: &str) -> Self {
        match spelling.split_once('/') {
            Some((namespace, name)) if spelling != "/" => {
                Self::keyword_parts(Some(namespace), name)
            }
            _ => Self::keyword_parts(None, spelling),
        }
    }

    pub(super) fn keyword_parts(namespace: Option<&str>, name: &str) -> Self {
        Self::Keyword {
            namespace: namespace.map(Arc::from),
            name: name.into(),
        }
    }

    pub(super) fn vector(items: Vec<Value>) -> Option<Self> {
        Collection::sequence(items).map(Self::Vector)
    }

    pub(super) fn list(items: Vec<Value>) -> Option<Self> {
        Collection::sequence(items).map(Self::List)
    }

    pub(super) fn map(entries: Vec<(Value, Value)>) -> Option<Self> {
        let child_depth = entries
            .iter()
            .map(|(key, value)| key.depth().max(value.depth()))
            .max()
            .unwrap_or(0);
        Collection::checked(entries, child_depth).map(Self::Map)
    }

    fn depth(&self) -> usize {
        match self {
            Self::Vector(collection) | Self::List(collection) => collection.depth,
            Self::Map(collection) => collection.depth,
            _ => 0,
        }
    }

    pub(super) fn truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Bool(false))
    }

    /// Text returned by Clojure's `str` for this value.
    pub(super) fn text(&self) -> Cow<'_, str> {
        match self {
            Self::Nil => Cow::Borrowed(""),
            Self::String(text) | Self::Symbol(text) => Cow::Borrowed(text),
            _ => Cow::Owned(self.to_string()),
        }
    }

    pub(super) fn display(&self, readable: bool) -> impl fmt::Display + '_ {
        PrintedValue {
            value: self,
            readable,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.display(true), f)
    }
}

struct PrintedValue<'a> {
    value: &'a Value,
    readable: bool,
}

impl fmt::Display for PrintedValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.value {
            Value::Nil => f.write_str("nil"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Integer(value) => write!(f, "{value}"),
            Value::String(text) if self.readable => write_quoted(f, text),
            Value::String(text) | Value::Symbol(text) => f.write_str(text),
            Value::Keyword {
                namespace: Some(namespace),
                name,
            } => write!(f, ":{namespace}/{name}"),
            Value::Keyword {
                namespace: None,
                name,
            } => write!(f, ":{name}"),
            Value::Vector(values) => write_sequence(f, "[", "]", values.items(), self.readable),
            Value::List(values) => write_sequence(f, "(", ")", values.items(), self.readable),
            Value::Map(entries) => {
                f.write_str("{")?;
                for (index, (key, value)) in entries.items().iter().enumerate() {
                    if index != 0 {
                        f.write_str(", ")?;
                    }
                    write!(
                        f,
                        "{} {}",
                        key.display(self.readable),
                        value.display(self.readable)
                    )?;
                }
                f.write_str("}")
            }
            Value::Var { namespace, name } => write!(f, "#'{namespace}/{name}"),
        }
    }
}

fn write_sequence(
    f: &mut fmt::Formatter<'_>,
    open: &str,
    close: &str,
    values: &[Value],
    readable: bool,
) -> fmt::Result {
    f.write_str(open)?;
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            f.write_str(" ")?;
        }
        write!(f, "{}", value.display(readable))?;
    }
    f.write_str(close)
}

fn write_quoted(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    f.write_str("\"")?;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        let escaped = match character {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            '\x08' => "\\b",
            '\x0c' => "\\f",
            _ => continue,
        };
        f.write_str(&text[start..index])?;
        f.write_str(escaped)?;
        start = index + character.len_utf8();
    }
    f.write_str(&text[start..])?;
    f.write_str("\"")
}

#[cfg(test)]
mod tests {
    use super::{MAX_NESTING_DEPTH, Value};

    #[test]
    fn readable_strings_escape_the_jvm_printer_characters() {
        let text = "\"\\\n\r\t\x08\x0c\0\x01\x1b\x7f\u{85}\u{2028}λ🙂";
        let value = Value::String(text.into());
        assert_eq!(
            value.to_string(),
            concat!(
                "\"",
                "\\\"",
                "\\\\",
                "\\n\\r\\t\\b\\f",
                "\0\x01\x1b\x7f\u{85}\u{2028}λ🙂",
                "\""
            )
        );
        assert_eq!(value.text(), text);
        assert_eq!(value.display(false).to_string(), text);
    }

    #[test]
    fn str_and_plain_print_differ_for_nil_and_nested_strings() {
        let value = Value::map(vec![
            (
                Value::keyword("items"),
                Value::vector(vec![
                    Value::String("λ".into()),
                    Value::Nil,
                    Value::Integer(42),
                ])
                .unwrap(),
            ),
            (
                Value::Symbol("symbols".into()),
                Value::list(vec![Value::String("🙂".into()), Value::Bool(false)]).unwrap(),
            ),
        ])
        .unwrap();
        let readable = "{:items [\"λ\" nil 42], symbols (\"🙂\" false)}";
        assert_eq!(value.to_string(), readable);
        assert_eq!(value.text(), readable);
        assert_eq!(
            value.display(false).to_string(),
            "{:items [λ nil 42], symbols (🙂 false)}"
        );
        assert_eq!(Value::Nil.text(), "");
        assert_eq!(Value::Nil.display(false).to_string(), "nil");
    }

    #[test]
    fn keyword_identity_retains_the_namespace_separator() {
        let unqualified = Value::keyword_parts(None, "a/b");
        let literal = Value::keyword("a/b");
        assert_ne!(unqualified, literal);
        assert_eq!(unqualified.text(), ":a/b");
        assert_eq!(literal.text(), ":a/b");

        let namespace_slash = Value::keyword_parts(Some("a/b"), "c");
        let name_slash = Value::keyword_parts(Some("a"), "b/c");
        assert_ne!(namespace_slash, name_slash);
        assert_eq!(namespace_slash.to_string(), ":a/b/c");
        assert_eq!(name_slash.to_string(), ":a/b/c");
        assert_eq!(Value::keyword("a/b/c"), name_slash);
        assert_eq!(Value::keyword("/"), Value::keyword_parts(None, "/"));
    }

    #[test]
    fn collection_depth_is_bounded_even_for_shared_preexisting_values() {
        let mut nested = Value::Nil;
        for level in 0..MAX_NESTING_DEPTH {
            nested = match level % 3 {
                0 => Value::vector(vec![nested]),
                1 => Value::list(vec![nested]),
                _ => Value::map(vec![(Value::keyword("child"), nested)]),
            }
            .unwrap();
        }
        assert_eq!(nested.depth(), MAX_NESTING_DEPTH);
        assert!(Value::vector(vec![nested.clone()]).is_none());
        assert!(Value::list(vec![nested.clone()]).is_none());
        assert!(Value::map(vec![(nested.clone(), Value::Nil)]).is_none());
        assert!(Value::map(vec![(Value::Nil, nested)]).is_none());
    }

    #[test]
    fn vars_print_as_var_references_and_only_nil_and_false_are_falsey() {
        let var = Value::Var {
            namespace: "portable.example".into(),
            name: "answer".into(),
        };
        assert_eq!(var.to_string(), "#'portable.example/answer");
        assert_eq!(var.text(), "#'portable.example/answer");
        assert!(var.truthy());
        assert!(!Value::Nil.truthy());
        assert!(!Value::Bool(false).truthy());
        assert!(Value::Bool(true).truthy());
        assert!(Value::Integer(0).truthy());
        assert!(Value::String("".into()).truthy());
        assert!(Value::list(Vec::new()).unwrap().truthy());
    }
}
