//! The Parenox evaluator for the portable Clojure forms it implements.
mod core;
mod value;

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parenox_frontend::reader::{
    Conditionals, Form, IntegerLiteralClass, Kind, NumericLiteral, Span,
    classify_number_with_class, read_prefix,
};

use crate::{Phase, RunError};
use core::Builtin;
use value::Value;

type Namespace = BTreeMap<String, Value>;

/// A single evaluator session with its own namespace and var roots.
pub struct Engine {
    namespaces: BTreeMap<Arc<str>, Namespace>,
    namespace: Arc<str>,
}

impl Default for Engine {
    fn default() -> Self {
        let namespace: Arc<str> = "user".into();
        Self {
            namespaces: BTreeMap::from([(namespace.clone(), Namespace::new())]),
            namespace,
        }
    }
}

impl Engine {
    /// Evaluate each input in sequence and return the last readable value.
    pub fn eval(
        &mut self,
        expressions: &[String],
        output: &mut dyn Write,
        timeout: Option<Duration>,
    ) -> Result<String, RunError> {
        let deadline = Deadline(timeout.map(|timeout| (Instant::now(), timeout)));
        let mut result = Value::Nil;
        for (index, source) in expressions.iter().enumerate() {
            deadline.check()?;
            let path = format!("<expr:{}>", index + 1);
            let (forms, error) = read_prefix(source, Conditionals::Reject);
            let mut eval = Evaluation {
                reader_namespace: self.namespace.clone(),
                engine: self,
                source,
                path: &path,
                output,
                locals: Vec::new(),
                deadline,
            };
            for form in &forms {
                // Auto-resolved keywords use the namespace at the start of
                // this reader form, even if a nested do changes it.
                eval.reader_namespace = eval.engine.namespace.clone();
                result = eval.form(form, true)?;
            }
            if let Some(error) = error {
                return Err(
                    RunError::new(Phase::Execution, error.message).at_file(path, Some(error.span))
                );
            }
        }
        deadline.check()?;
        Ok(result.to_string())
    }
}

#[derive(Clone, Copy)]
struct Deadline(Option<(Instant, Duration)>);

impl Deadline {
    fn check(self) -> Result<(), RunError> {
        if let Some((started, timeout)) = self.0
            && started.elapsed() >= timeout
        {
            return Err(RunError::new(
                Phase::Execution,
                format!("Clojure eval timed out after {} ms", timeout.as_millis()),
            )
            .with_exit_code(124));
        }
        Ok(())
    }
}

struct Evaluation<'a, 'session, 'log> {
    engine: &'session mut Engine,
    source: &'a str,
    reader_namespace: Arc<str>,
    path: &'a str,
    output: &'log mut dyn Write,
    locals: Vec<(&'a str, Value)>,
    deadline: Deadline,
}

impl<'a> Evaluation<'a, '_, '_> {
    fn form(&mut self, form: &'a Form, top_level: bool) -> Result<Value, RunError> {
        self.deadline.check()?;
        match &form.kind {
            Kind::Symbol(name) => self
                .resolve(name)
                .cloned()
                .ok_or_else(|| self.error(form.span, format!("Unable to resolve symbol: {name}"))),
            Kind::Quote(quoted) => self.quoted(quoted),
            Kind::List(items) => self.call(form, items, top_level),
            Kind::Vector(items) => {
                let items = self.values(items)?;
                self.collection(form.span, Value::vector(items))
            }
            Kind::Map(entries) => self.map(form.span, entries, false),
            _ => self.scalar(form),
        }
    }

    fn scalar(&self, form: &Form) -> Result<Value, RunError> {
        Ok(match &form.kind {
            Kind::Nil => Value::Nil,
            Kind::Bool(value) => Value::Bool(*value),
            Kind::Number => {
                let token = &self.source[form.span.start.offset..form.span.end.offset];
                match classify_number_with_class(token) {
                    (NumericLiteral::Integer(value), Some(IntegerLiteralClass::Long)) => {
                        Value::Integer(value)
                    }
                    _ => return Err(self.error(form.span, "Parenox supports Long literals here")),
                }
            }
            Kind::String(value) => Value::String(value.as_str().into()),
            Kind::Keyword(value) => {
                if let Some(name) = value.strip_prefix("::") {
                    if name.contains('/') {
                        return Err(
                            self.error(form.span, "Parenox does not support keyword aliases")
                        );
                    }
                    Value::keyword_parts(Some(&self.reader_namespace), name)
                } else {
                    Value::keyword(value.strip_prefix(':').unwrap_or(value))
                }
            }
            Kind::Utf16String(_) => {
                return Err(self.error(
                    form.span,
                    "Parenox does not support unpaired UTF-16 surrogates",
                ));
            }
            _ => return Err(self.error(form.span, "Unsupported form in the Parenox evaluator")),
        })
    }

    fn quoted(&mut self, form: &'a Form) -> Result<Value, RunError> {
        self.deadline.check()?;
        match &form.kind {
            Kind::Symbol(name) => Ok(Value::Symbol(name.as_str().into())),
            Kind::Quote(quoted) => {
                let quoted = self.quoted(quoted)?;
                self.collection(
                    form.span,
                    Value::list(vec![Value::Symbol("quote".into()), quoted]),
                )
            }
            Kind::List(items) | Kind::Vector(items) => {
                let values = items
                    .iter()
                    .map(|item| self.quoted(item))
                    .collect::<Result<Vec<_>, _>>()?;
                let collection = if matches!(form.kind, Kind::List(_)) {
                    Value::list(values)
                } else {
                    Value::vector(values)
                };
                self.collection(form.span, collection)
            }
            Kind::Map(entries) => self.map(form.span, entries, true),
            _ => self.scalar(form),
        }
    }

    fn map(&mut self, span: Span, entries: &'a [Form], quoted: bool) -> Result<Value, RunError> {
        let (pairs, []) = entries.as_chunks::<2>() else {
            return Err(self.error(span, "Map literal requires key/value pairs"));
        };
        if pairs.len() > 8 {
            return Err(self.error(
                span,
                "Parenox supports up to eight entries in a map literal",
            ));
        }
        let mut result = Vec::with_capacity(pairs.len());
        for [key, value] in pairs {
            let (key_value, value) = if quoted {
                (self.quoted(key)?, self.quoted(value)?)
            } else {
                (self.form(key, false)?, self.form(value, false)?)
            };
            if !matches!(
                key_value,
                Value::Nil
                    | Value::Bool(_)
                    | Value::Integer(_)
                    | Value::String(_)
                    | Value::Keyword { .. }
                    | Value::Symbol(_)
            ) {
                return Err(self.error(key.span, "Parenox supports scalar map keys"));
            }
            result.push((key_value, value));
        }
        for (index, (key, _)) in result.iter().enumerate() {
            if result[..index].iter().any(|(prior, _)| prior == key) {
                let printed = if *key == Value::Nil {
                    "null".to_owned()
                } else {
                    key.display(false).to_string()
                };
                return Err(self.error(span, format!("Duplicate key: {printed}")));
            }
        }
        self.collection(span, Value::map(result))
    }

    fn collection(&self, span: Span, value: Option<Value>) -> Result<Value, RunError> {
        value.ok_or_else(|| self.error(span, "Parenox value nesting limit exceeded"))
    }

    fn resolve(&self, name: &str) -> Option<&Value> {
        if let Some((namespace, local)) = name.split_once('/') {
            return self.engine.namespaces.get(namespace)?.get(local);
        }
        self.locals
            .iter()
            .rev()
            .find_map(|(local, value)| (*local == name).then_some(value))
            .or_else(|| {
                self.engine
                    .namespaces
                    .get(&self.engine.namespace)?
                    .get(name)
            })
    }

    fn core_name(&self, symbol: &'a str) -> Option<&'a str> {
        if let Some((namespace, name)) = symbol.split_once('/') {
            return (namespace == "clojure.core").then_some(name);
        }
        self.resolve(symbol).is_none().then_some(symbol)
    }

    fn call(
        &mut self,
        form: &'a Form,
        items: &'a [Form],
        top_level: bool,
    ) -> Result<Value, RunError> {
        let Some((head, args)) = items.split_first() else {
            return self.collection(form.span, Value::list(Vec::new()));
        };
        let Kind::Symbol(symbol) = &head.kind else {
            return Err(self.error(head.span, "Parenox requires a named call target here"));
        };
        match symbol.as_str() {
            "quote" => {
                let [quoted] = args else {
                    return Err(self.error(form.span, "quote requires one argument"));
                };
                return self.quoted(quoted);
            }
            "if" => {
                let [test, consequent, fallback @ ..] = args else {
                    return Err(self.error(form.span, "if requires two or three arguments"));
                };
                if fallback.len() > 1 {
                    return Err(self.error(form.span, "if requires two or three arguments"));
                }
                let selected = if self.form(test, false)?.truthy() {
                    Some(consequent)
                } else {
                    fallback.first()
                };
                return selected.map_or(Ok(Value::Nil), |form| self.form(form, false));
            }
            "do" => return self.body(args, top_level),
            "let*" => return self.let_binding(form.span, args),
            "def" if top_level => return self.define(form.span, args),
            _ => {}
        }
        let Some(core) = self.core_name(symbol) else {
            return Err(self.error(head.span, format!("Parenox cannot call {symbol} here")));
        };
        match core {
            "let" => self.let_binding(form.span, args),
            "ns" if top_level => self.change_namespace(form.span, args),
            _ => {
                let builtin = Builtin::named(core).ok_or_else(|| {
                    self.error(
                        head.span,
                        format!("Unsupported function or macro: {symbol}"),
                    )
                })?;
                builtin
                    .check_inline_arity(args.len())
                    .map_err(|message| self.error(form.span, message))?;
                let values = self.values(args)?;
                builtin
                    .apply(&values, self.output)
                    .map_err(|message| self.error(form.span, message))
            }
        }
    }

    fn values(&mut self, forms: &'a [Form]) -> Result<Vec<Value>, RunError> {
        forms.iter().map(|form| self.form(form, false)).collect()
    }

    fn body(&mut self, forms: &'a [Form], top_level: bool) -> Result<Value, RunError> {
        let mut value = Value::Nil;
        for form in forms {
            value = self.form(form, top_level)?;
        }
        Ok(value)
    }

    fn let_binding(&mut self, span: Span, args: &'a [Form]) -> Result<Value, RunError> {
        let Some((bindings, body)) = args.split_first() else {
            return Err(self.error(span, "let requires a binding vector"));
        };
        let Kind::Vector(bindings) = &bindings.kind else {
            return Err(self.error(bindings.span, "let requires a binding vector"));
        };
        let (pairs, []) = bindings.as_chunks::<2>() else {
            return Err(self.error(span, "let requires a value for each binding"));
        };
        if let Some([name, _]) = pairs.iter().find(|[name, _]| binding_name(name).is_none()) {
            return Err(self.error(name.span, "Parenox requires simple symbol bindings"));
        }
        let scope = self.locals.len();
        let result = (|| {
            for [name, value] in pairs {
                let value = self.form(value, false)?;
                self.locals.push((binding_name(name).unwrap(), value));
            }
            self.body(body, false)
        })();
        self.locals.truncate(scope);
        result
    }

    fn change_namespace(&mut self, span: Span, args: &[Form]) -> Result<Value, RunError> {
        let [form] = args else {
            return Err(self.error(span, "Parenox supports ns with a name and no clauses"));
        };
        let Kind::Symbol(name) = &form.kind else {
            return Err(self.error(form.span, "ns requires a namespace symbol"));
        };
        if name.contains(['/', ':']) || name == "clojure.core" {
            return Err(self.error(form.span, "Parenox requires a user namespace"));
        }
        let namespace: Arc<str> = name.as_str().into();
        self.engine.namespaces.entry(namespace.clone()).or_default();
        self.engine.namespace = namespace;
        Ok(Value::Nil)
    }

    fn define(&mut self, span: Span, args: &'a [Form]) -> Result<Value, RunError> {
        let [name, value] = args else {
            return Err(self.error(span, "Parenox requires a name and initializer for def"));
        };
        let name = binding_name(name)
            .ok_or_else(|| self.error(name.span, "def requires a simple symbol"))?;
        let value = self.form(value, false)?;
        let namespace = self.engine.namespace.clone();
        self.engine
            .namespaces
            .get_mut(&namespace)
            .unwrap()
            .insert(name.into(), value);
        Ok(Value::Var {
            namespace,
            name: name.into(),
        })
    }

    fn error(&self, span: Span, message: impl Into<String>) -> RunError {
        RunError::new(Phase::Execution, message).at_file(self.path.to_owned(), Some(span))
    }
}

fn binding_name(form: &Form) -> Option<&str> {
    let Kind::Symbol(name) = &form.kind else {
        return None;
    };
    (!name.contains('/') && name != "&").then_some(name)
}
