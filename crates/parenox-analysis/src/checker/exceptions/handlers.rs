//! Clojure try, catch and finally scopes.
use super::Throws;
use crate::checker::bindings::dotted_binding_error;
use crate::checker::{Checker, Env, Value, metadata::JavaTypeHint};
use crate::reader::{Form, Kind};
use crate::types::Type;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ExceptionClass {
    Throwable,
    Error,
    AssertionError,
    Exception,
    RuntimeException,
    IllegalArgumentException,
    ArityException,
    IllegalStateException,
    ClassCastException,
    NullPointerException,
    NegativeArraySizeException,
    ArithmeticException,
    IndexOutOfBoundsException,
    ExceptionInfo,
    IOException,
    FileNotFoundException,
}

impl ExceptionClass {
    fn resolve(name: &str) -> Option<Self> {
        match name {
            "ExceptionInfo" | "clojure.lang.ExceptionInfo" => return Some(Self::ExceptionInfo),
            "clojure.lang.ArityException" => return Some(Self::ArityException),
            _ => {}
        }
        match name {
            "java.io.IOException" => return Some(Self::IOException),
            "java.io.FileNotFoundException" => return Some(Self::FileNotFoundException),
            _ => {}
        }
        match name.strip_prefix("java.lang.").unwrap_or(name) {
            "Throwable" => Some(Self::Throwable),
            "Error" => Some(Self::Error),
            "AssertionError" => Some(Self::AssertionError),
            "Exception" => Some(Self::Exception),
            "RuntimeException" => Some(Self::RuntimeException),
            "IllegalArgumentException" => Some(Self::IllegalArgumentException),
            "IllegalStateException" => Some(Self::IllegalStateException),
            "ClassCastException" => Some(Self::ClassCastException),
            "NullPointerException" => Some(Self::NullPointerException),
            "NegativeArraySizeException" => Some(Self::NegativeArraySizeException),
            "ArithmeticException" => Some(Self::ArithmeticException),
            "IndexOutOfBoundsException" => Some(Self::IndexOutOfBoundsException),
            _ => None,
        }
    }

    fn parent(self) -> Option<Self> {
        match self {
            Self::Throwable => None,
            Self::Error | Self::Exception => Some(Self::Throwable),
            Self::AssertionError => Some(Self::Error),
            Self::ArityException => Some(Self::IllegalArgumentException),
            Self::RuntimeException | Self::IOException => Some(Self::Exception),
            Self::FileNotFoundException => Some(Self::IOException),
            _ => Some(Self::RuntimeException),
        }
    }

    pub(super) fn catches(self, class: Option<Self>) -> bool {
        self == Self::Throwable
            || class.is_some_and(|class| self == class || self.catches(class.parent()))
    }
}

struct Handler<'a> {
    class: Option<ExceptionClass>,
    class_name: &'a str,
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
        let binding_span = binding.without_metadata().span;
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
        if let Some(diagnostic) = dotted_binding_error(binding, binding_span) {
            self.diagnostics.push(diagnostic);
            return None;
        }
        let qualified = self.model.class_name(name);
        let resolved = ExceptionClass::resolve(qualified);
        if resolved.is_none() {
            match qualified.strip_prefix("java.lang.").unwrap_or(qualified) {
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
            class_name: name,
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
        let enclosing = self.throws.replace(Throws::default());
        let mut result = self.body(body, env, tail);
        let mut pending = self.throws.replace(Throws::default()).unwrap();
        let mut handled = Vec::<ExceptionClass>::new();
        for handler in handlers {
            let ty = if handled.iter().any(|class| class.catches(handler.class)) {
                None
            } else {
                pending.take_matching(handler.class)
            };
            let reachable = ty.is_some();
            let mut local = env.clone();
            local.insert(
                handler.binding.to_owned(),
                Value::new(ty.unwrap_or(Type::Never)),
            );
            local.set_java_hint(
                handler.binding,
                JavaTypeHint::Symbol(self.model.class_name(handler.class_name).to_owned()),
            );
            let caught = self.check_path(reachable, |checker| {
                checker.body(handler.body, &local, false)
            });
            result = result.join(caught);
            handled.extend(handler.class);
        }
        let handler_throws = self.throws.replace(Throws::default()).unwrap();
        pending.extend(handler_throws);
        let cleanup = self.check_path(
            result.known != Type::Never || pending.can_throw(),
            |checker| checker.body(finally, env, false),
        );
        let cleanup_throws = std::mem::replace(&mut self.throws, enclosing).unwrap();
        if let Some(enclosing) = &mut self.throws {
            enclosing.extend(cleanup_throws);
            if cleanup.known != Type::Never {
                enclosing.extend(pending);
            }
        }
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
