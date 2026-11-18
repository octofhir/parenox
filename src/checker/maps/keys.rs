//! Lookup and presence for keys supplied as expressions.
use super::super::{Checker, Env, Value, calls::call_returns};
use super::{Lookup, key_presence, narrow_key_presence};
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

impl Checker<'_> {
    pub(in crate::checker) fn literal_map_key(&self, form: &Form) -> Option<Type> {
        match &form.without_metadata().kind {
            Kind::Keyword(key) => Some(Type::Key(self.model.key(key))),
            Kind::String(key) => Some(Type::Str(key.clone())),
            _ => None,
        }
    }

    pub(super) fn read_map_key(
        &mut self,
        form: &Form,
        env: &Env,
        name: &str,
        allow_index: bool,
    ) -> Option<Type> {
        let key = self.infer(form, env);
        self.validate_map_key(key, form.span, name, allow_index)
    }

    pub(super) fn validate_map_key(
        &mut self,
        key: Value,
        span: Span,
        name: &str,
        allow_index: bool,
    ) -> Option<Type> {
        if key.known.is_lookup_key(allow_index) {
            Some(key.known)
        } else {
            self.unsupported(
                span,
                format!(
                    "{name} analysis supports keyword and string keys{}, got {}",
                    if allow_index {
                        " and known integer indices"
                    } else {
                        ""
                    },
                    key.ty
                ),
            );
            None
        }
    }

    pub(in crate::checker) fn lookup_form(
        &mut self,
        xs: &[Form],
        span: Span,
        env: &Env,
        name: &str,
    ) -> Value {
        let required = name == "clojure.core/req!";
        if !(3..=if required { 3 } else { 4 }).contains(&xs.len()) {
            self.error(
                "PNX2554",
                span,
                format!(
                    "Invalid arity for {name}: expected a collection and key{}",
                    if required {
                        ""
                    } else {
                        " and optional default"
                    }
                ),
            );
            return Value::new(Type::Unknown);
        }
        let args: Vec<_> = self.eval_args(&xs[1..], env, true).collect();
        if !call_returns(&args) {
            return Value::new(Type::Never);
        }
        let mut args = args.into_iter();
        let (target, _) = args.next().unwrap();
        let (key, key_span) = args.next().unwrap();
        let default = args.next().map(|(value, _)| value);
        let Some(key) = self.validate_map_key(key, key_span, name, true) else {
            self.may_throw();
            return Value::new(Type::Unknown);
        };
        if target.known.contains_unknown() || key.contains_unknown() {
            self.may_throw();
        }
        let presence = env.presence(&xs[1], &xs[2]);
        if required {
            if presence.or_else(|| key_presence(&target.known, &key)) != Some(true) {
                self.illegal_argument();
            }
            let target = if presence == Some(false) {
                Value::new(Type::Nil)
            } else {
                target
            };
            self.checked_lookup(&target, &key, Lookup::Required, key_span)
        } else {
            self.lookup_with_presence(&target, &key, default.as_ref(), key_span, presence)
        }
    }

    pub(super) fn lookup_with_presence(
        &mut self,
        target: &Value,
        key: &Type,
        default: Option<&Value>,
        span: Span,
        presence: Option<bool>,
    ) -> Value {
        match presence {
            Some(true) => self.lookup_value(target, key, Lookup::Required, span),
            Some(false) => default.cloned().unwrap_or_else(|| Value::new(Type::Nil)),
            None => self.lookup_default(target, key, default, span),
        }
    }

    pub(in crate::checker) fn narrow_contains(&self, args: &[Form], truth: bool, env: &mut Env) {
        let [target, key] = args else { return };
        let Some(collection) = target.symbol() else {
            return;
        };
        let key_symbol = key
            .symbol()
            .and_then(|name| env.get(name).map(|v| (name, &v.known)));
        let exact = self.literal_map_key(key).or_else(|| {
            key_symbol
                .filter(|(_, ty)| ty.is_literal_map_key())
                .map(|(_, ty)| ty.clone())
        });
        if let Some(key) = exact {
            if let Some(value) = env.get_mut(collection) {
                value.narrow(narrow_key_presence(&value.known, &key, truth));
            }
        } else if let Some((key_name, _)) = key_symbol
            && env.contains_key(collection)
        {
            env.set_presence(collection, key_name, truth);
        }
    }
}
