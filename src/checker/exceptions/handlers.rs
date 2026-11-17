//! Clojure try, catch and finally scopes.
use super::{ExtraFields, map};
use crate::checker::{Checker, Env, Value};
use crate::reader::{Form, Kind};
use crate::types::Type;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExceptionClass {
    Throwable,
    Error,
    Exception,
    RuntimeException,
    IllegalArgumentException,
    IllegalStateException,
    ClassCastException,
    NullPointerException,
    ArithmeticException,
    IndexOutOfBoundsException,
    ExceptionInfo,
}

impl ExceptionClass {
    fn resolve(name: &str) -> Option<Self> {
        if matches!(name, "ExceptionInfo" | "clojure.lang.ExceptionInfo") {
            return Some(Self::ExceptionInfo);
        }
        match name.strip_prefix("java.lang.").unwrap_or(name) {
            "Throwable" => Some(Self::Throwable),
            "Error" => Some(Self::Error),
            "Exception" => Some(Self::Exception),
            "RuntimeException" => Some(Self::RuntimeException),
            "IllegalArgumentException" => Some(Self::IllegalArgumentException),
            "IllegalStateException" => Some(Self::IllegalStateException),
            "ClassCastException" => Some(Self::ClassCastException),
            "NullPointerException" => Some(Self::NullPointerException),
            "ArithmeticException" => Some(Self::ArithmeticException),
            "IndexOutOfBoundsException" => Some(Self::IndexOutOfBoundsException),
            _ => None,
        }
    }

    fn parent(self) -> Option<Self> {
        match self {
            Self::Throwable => None,
            Self::Error | Self::Exception => Some(Self::Throwable),
            Self::RuntimeException => Some(Self::Exception),
            _ => Some(Self::RuntimeException),
        }
    }

    fn catches(self, class: Option<Self>) -> bool {
        self == Self::Throwable
            || class.is_some_and(|class| self == class || self.catches(class.parent()))
    }
}

struct Handler<'a> {
    class: Option<ExceptionClass>,
    binding: &'a str,
    body: &'a [Form],
}

impl Checker<'_> {
    pub(in crate::checker) fn try_form(&mut self, forms: &[Form], env: &Env, tail: bool) -> Value {
        let first_clause = forms
            .iter()
            .position(|form| matches!(list_head(form), Some("catch" | "finally")))
            .unwrap_or(forms.len());
        let (body, clauses) = forms.split_at(first_clause);
        let mut handlers = Vec::new();
        let mut finally = &[][..];
        for (index, clause) in clauses.iter().enumerate() {
            match list_head(clause) {
                Some("catch") => {
                    let Some(handler) = self.read_catch(clause) else {
                        return Value::new(Type::Unknown);
                    };
                    handlers.push(handler);
                }
                Some("finally") if index + 1 == clauses.len() => {
                    let Kind::List(parts) = &clause.without_metadata().kind else {
                        unreachable!()
                    };
                    finally = &parts[1..];
                }
                Some("finally") => {
                    self.error(
                        "PNX1001",
                        clause.span,
                        "finally must be the last clause of try",
                    );
                    return Value::new(Type::Unknown);
                }
                _ => {
                    self.error(
                        "PNX1001",
                        clause.span,
                        "Only catch or finally can follow a catch clause",
                    );
                    return Value::new(Type::Unknown);
                }
            }
        }
        let previous = self
            .recur_target
            .as_mut()
            .map(|target| std::mem::replace(&mut target.blocked_by_try, true));
        let result = self.try_outcomes(body, handlers, finally, env, tail);
        if let Some((target, previous)) = self.recur_target.as_mut().zip(previous) {
            target.blocked_by_try = previous;
        }
        result
    }

    fn read_catch<'a>(&mut self, form: &'a Form) -> Option<Handler<'a>> {
        let Kind::List(parts) = &form.without_metadata().kind else {
            unreachable!()
        };
        let [_, class, binding, body @ ..] = parts.as_slice() else {
            self.error(
                "PNX1001",
                form.span,
                "catch requires an exception class and binding",
            );
            return None;
        };
        let Some(name) = class.symbol() else {
            self.error("PNX1001", class.span, "catch requires a class symbol");
            return None;
        };
        let Some(binding) = binding
            .symbol()
            .filter(|symbol| !symbol.contains('/') || *symbol == "/")
        else {
            self.error(
                "PNX1001",
                binding.span,
                "catch requires an unqualified binding symbol",
            );
            return None;
        };
        let resolved = ExceptionClass::resolve(name);
        if resolved.is_none() {
            match name.strip_prefix("java.lang.").unwrap_or(name) {
                "Object" | "String" | "Number" | "Boolean" | "Character" | "Long" | "Integer" => {
                    self.error(
                        "PNX1001",
                        class.span,
                        format!("Catch class {name} must extend Throwable"),
                    );
                    return None;
                }
                _ => {
                    self.unsupported(
                        class.span,
                        format!(
                            "Catch class {name} is outside the supported JVM exception classes"
                        ),
                    );
                }
            }
        }
        Some(Handler {
            class: resolved,
            binding,
            body,
        })
    }

    fn try_outcomes(
        &mut self,
        body: &[Form],
        handlers: Vec<Handler<'_>>,
        finally: &[Form],
        env: &Env,
        tail: bool,
    ) -> Value {
        let mut result = self.body(body, env, tail);
        let possible_exception = !body.iter().all(|form| inert(form, env));
        let mut handled = Vec::<ExceptionClass>::new();
        for handler in handlers {
            let reachable =
                possible_exception && !handled.iter().any(|class| class.catches(handler.class));
            let ty = if handler.class == Some(ExceptionClass::ExceptionInfo) {
                Type::ExceptionInfo(Box::new(map(ExtraFields::Open)))
            } else {
                Type::Throwable
            };
            let mut local = env.clone();
            local.insert(handler.binding.to_owned(), Value::new(ty));
            let caught = self.check_path(reachable, |checker| {
                checker.body(handler.body, &local, false)
            });
            result = result.join(caught);
            handled.extend(handler.class);
        }
        let cleanup = self.body(finally, env, false);
        if cleanup.known == Type::Never {
            cleanup
        } else {
            result
        }
    }
}

fn list_head(form: &Form) -> Option<&str> {
    if let Kind::List(parts) = &form.without_metadata().kind {
        parts.first().and_then(Form::symbol)
    } else {
        None
    }
}

fn inert(form: &Form, env: &Env) -> bool {
    match &form.without_metadata().kind {
        Kind::Nil
        | Kind::Bool(_)
        | Kind::Number
        | Kind::Character(_)
        | Kind::String(_)
        | Kind::Keyword(_) => true,
        Kind::Symbol(name) => env.contains_key(name),
        _ => false,
    }
}
