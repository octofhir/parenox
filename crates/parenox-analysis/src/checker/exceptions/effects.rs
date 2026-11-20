//! Possible exceptions from the current try body or handler.
use super::handlers::ExceptionClass;
use super::{ExtraFields, map};
use crate::checker::Checker;
use crate::types::Type;

#[derive(Clone, Default)]
pub(in crate::checker) struct Throws {
    unknown: bool,
    known: Vec<(ExceptionClass, Type)>,
}

impl Throws {
    fn unknown(&mut self) {
        self.unknown = true;
        self.known.clear();
    }

    fn record(&mut self, class: ExceptionClass, ty: Type) {
        if self.unknown
            || self
                .known
                .iter()
                .any(|(previous, value)| *previous == class && *value == ty)
        {
            return;
        }
        if self.known.len() >= 32 {
            self.unknown();
        } else {
            self.known.push((class, ty));
        }
    }

    pub(super) fn throw_value(&mut self, ty: &Type) {
        match ty {
            Type::Never => {}
            Type::Throwable | Type::Unknown => self.unknown(),
            Type::ExceptionInfo(_) => self.record(ExceptionClass::ExceptionInfo, ty.clone()),
            Type::Nil => self.record(ExceptionClass::NullPointerException, Type::Throwable),
            Type::Union(parts) => {
                for part in parts {
                    self.throw_value(part);
                }
            }
            _ => self.record(ExceptionClass::ClassCastException, Type::Throwable),
        }
    }

    pub(super) fn can_throw(&self) -> bool {
        self.unknown || !self.known.is_empty()
    }

    pub(in crate::checker) fn extend(&mut self, other: Self) {
        if other.unknown {
            self.unknown();
        } else {
            for (class, ty) in other.known {
                self.record(class, ty);
            }
        }
    }

    pub(super) fn take_matching(&mut self, class: Option<ExceptionClass>) -> Option<Type> {
        if self.unknown {
            let bound = if class == Some(ExceptionClass::ExceptionInfo) {
                Type::ExceptionInfo(Box::new(map(ExtraFields::Open)))
            } else {
                Type::Throwable
            };
            if class == Some(ExceptionClass::Throwable) {
                *self = Self::default();
            }
            return Some(bound);
        }
        let Some(class) = class else {
            return self.can_throw().then_some(Type::Throwable);
        };
        let caught = Type::union(
            self.known
                .extract_if(.., |(thrown, _)| class.catches(Some(*thrown)))
                .map(|(_, ty)| ty),
        );
        (caught != Type::Never).then_some(caught)
    }
}

impl Checker<'_> {
    pub(in crate::checker) fn assertion_error(&mut self) {
        if let Some(throws) = &mut self.throws {
            throws.record(ExceptionClass::AssertionError, Type::Throwable);
        }
    }

    pub(in crate::checker) fn may_throw(&mut self) {
        if let Some(throws) = &mut self.throws {
            throws.unknown();
        }
    }

    pub(in crate::checker) fn class_cast(&mut self) {
        if let Some(throws) = &mut self.throws {
            throws.record(ExceptionClass::ClassCastException, Type::Throwable);
        }
    }

    pub(in crate::checker) fn null_pointer(&mut self) {
        if let Some(throws) = &mut self.throws {
            throws.record(ExceptionClass::NullPointerException, Type::Throwable);
        }
    }

    pub(in crate::checker) fn illegal_argument(&mut self) {
        if let Some(throws) = &mut self.throws {
            throws.record(ExceptionClass::IllegalArgumentException, Type::Throwable);
        }
    }

    pub(in crate::checker) fn arithmetic_exception(&mut self) {
        if let Some(throws) = &mut self.throws {
            throws.record(ExceptionClass::ArithmeticException, Type::Throwable);
        }
    }
}
