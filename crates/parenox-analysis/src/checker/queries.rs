//! Observations made by the checker at one requested source occurrence.
use std::sync::Arc;

use super::{Checker, Value, calls::CallArguments};
use crate::ContractOrigin;
use crate::reader::{Form, Span};
use crate::types::{Signature, Type};

pub(super) enum TypeQuery<'a> {
    Local { name: &'a str, span: Span, ty: Type },
    Lookup { span: Span, ty: Type },
    Call(Box<CallQuery<'a>>),
}

pub(super) struct CallQuery<'a> {
    span: Span,
    written: &'a [Form],
    checked: Option<CheckedCall>,
    declared_only: bool,
}

pub(crate) struct CheckedCall {
    pub signature: Signature,
    pub origin: Option<Arc<ContractOrigin>>,
    pub explicit_offset: Option<usize>,
    pub argument_count: Option<usize>,
    declared: Signature,
}

impl CheckedCall {
    pub(super) fn declaration(signature: &Signature, origin: &Arc<ContractOrigin>) -> Self {
        Self {
            signature: signature.clone(),
            origin: Some(origin.clone()),
            explicit_offset: None,
            argument_count: None,
            declared: signature.clone(),
        }
    }
}

impl<'a> TypeQuery<'a> {
    pub(super) fn local(site: &'a Form) -> Option<Self> {
        Some(Self::Local {
            name: site.symbol()?,
            span: site.span,
            ty: Type::Never,
        })
    }

    pub(super) fn call(span: Span, written: &'a [Form]) -> Self {
        Self::Call(Box::new(CallQuery {
            span,
            written,
            checked: None,
            declared_only: false,
        }))
    }

    pub(super) fn lookup(site: &Form) -> Self {
        Self::Lookup {
            span: site.span,
            ty: Type::Never,
        }
    }

    pub(super) fn span(&self) -> Span {
        match self {
            Self::Local { span, .. } | Self::Lookup { span, .. } => *span,
            Self::Call(query) => query.span,
        }
    }

    pub(super) fn into_value_type(self) -> Option<Type> {
        match self {
            Self::Local { ty, .. } | Self::Lookup { ty, .. }
                if !matches!(ty, Type::Never | Type::Unknown) =>
            {
                Some(ty)
            }
            _ => None,
        }
    }

    pub(super) fn into_call(self) -> Option<CheckedCall> {
        match self {
            Self::Call(query) => query.checked,
            _ => None,
        }
    }
}

impl Checker<'_> {
    pub(super) fn observe_local(&mut self, name: &str, span: Span, value: &Value) {
        if self.path_reachable
            && let Some(TypeQuery::Local {
                name: target,
                span: at,
                ty,
            }) = &mut self.type_query
            && *target == name
            && *at == span
            && *ty != value.known
        {
            *ty = Type::union([std::mem::replace(ty, Type::Never), value.known.clone()]);
        }
    }

    pub(super) fn observe_lookup(&mut self, span: Span, value: &Value) {
        if self.path_reachable
            && let Some(TypeQuery::Lookup { span: at, ty }) = &mut self.type_query
            && *at == span
            && *ty != value.known
        {
            *ty = Type::union([std::mem::replace(ty, Type::Never), value.known.clone()]);
        }
    }

    pub(super) fn observe_call(
        &mut self,
        span: Span,
        arguments: CallArguments<'_>,
        declared: &Signature,
        signature: &Signature,
        origin: Option<&Arc<ContractOrigin>>,
    ) {
        let Some(TypeQuery::Call(query)) = &mut self.type_query else {
            return;
        };
        if query.span != span
            || !self.path_reachable
            || arguments.forms.len() != arguments.values.len()
        {
            return;
        }
        let argument_count = Some(arguments.forms.len());
        let explicit_offset = if query.written.is_empty() {
            // An implicit-only call does not prove which slot new written
            // arguments would occupy.
            arguments.forms.is_empty().then_some(0)
        } else {
            let Some(offset) = explicit_offset(query.written, arguments.forms) else {
                return;
            };
            Some(offset)
        };
        if let Some(previous) = &mut query.checked {
            if previous.declared != *declared
                || previous.origin.as_ref() != origin
                || previous.explicit_offset != explicit_offset
                || previous.argument_count != argument_count
            {
                self.type_query = None;
            } else if previous.signature != *signature {
                query.use_declaration();
            }
        } else {
            query.checked = Some(CheckedCall {
                signature: if query.declared_only {
                    declared
                } else {
                    signature
                }
                .clone(),
                origin: origin.cloned(),
                explicit_offset,
                argument_count,
                declared: declared.clone(),
            });
        }
    }

    pub(super) fn withhold_type_facts(&mut self, enclosing: Span) {
        if !self.path_reachable {
            return;
        }
        if let Some(query) = &mut self.type_query {
            let site = query.span();
            if enclosing.start.offset <= site.start.offset
                && site.end.offset <= enclosing.end.offset
            {
                match query {
                    TypeQuery::Local { .. } | TypeQuery::Lookup { .. } => self.type_query = None,
                    TypeQuery::Call(query) => query.use_declaration(),
                }
            }
        }
    }
}

impl CallQuery<'_> {
    fn use_declaration(&mut self) {
        self.declared_only = true;
        if let Some(checked) = &mut self.checked {
            checked.signature = checked.declared.clone();
        }
    }
}

fn explicit_offset(written: &[Form], evaluated: &[Form]) -> Option<usize> {
    // Thread expansion keeps the spans of explicit arguments and inserts a
    // synthetic argument before or after them. Match the explicit forms.
    let mut matches = evaluated
        .windows(written.len())
        .enumerate()
        .filter(|(_, forms)| {
            forms
                .iter()
                .zip(written)
                .all(|(actual, original)| actual.span == original.span)
        });
    let (offset, _) = matches.next()?;
    matches.next().is_none().then_some(offset)
}
