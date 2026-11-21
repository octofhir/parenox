//! Observations made by the checker at one requested source occurrence.
use std::{collections::BTreeMap, sync::Arc};

use super::{Checker, Env, Value, calls::CallArguments};
use crate::ContractOrigin;
use crate::reader::{Form, Span};
use crate::types::{Signature, Type};

pub(super) enum TypeQuery<'a> {
    Local { name: &'a str, span: Span, ty: Type },
    Lookup { span: Span, ty: Type },
    Call(Box<CallQuery<'a>>),
    Bindings(BindingQueries),
    Completion(CompletionQuery<'a>),
}

pub(super) struct CompletionQuery<'a> {
    span: Span,
    types: BTreeMap<&'a str, Option<Type>>,
}

pub(super) struct BindingQueries {
    enclosing: Span,
    types: BTreeMap<usize, BindingObservation>,
}

struct BindingObservation {
    span: Span,
    ty: Option<Type>,
    literal_initializer: bool,
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
    declaration: CallDeclaration,
}

#[derive(PartialEq, Eq)]
enum CallDeclaration {
    Typed(Signature),
    Inferred {
        definition: Span,
        labels: Option<Vec<String>>,
    },
}

pub(super) struct InferredCall {
    pub definition: Span,
    pub labels: Option<Vec<String>>,
    pub args: Vec<Type>,
}

impl CheckedCall {
    pub(super) fn declaration(signature: &Signature, origin: &Arc<ContractOrigin>) -> Self {
        Self {
            signature: signature.clone(),
            origin: Some(origin.clone()),
            explicit_offset: None,
            argument_count: None,
            declaration: CallDeclaration::Typed(signature.clone()),
        }
    }

    pub(crate) fn local_labels(&self) -> Option<&[String]> {
        match &self.declaration {
            CallDeclaration::Inferred { labels, .. } => labels.as_deref(),
            CallDeclaration::Typed(_) => None,
        }
    }

    pub(crate) fn declared_argument_type(&self, written: usize) -> Option<&Type> {
        let CallDeclaration::Typed(signature) = &self.declaration else {
            return None;
        };
        let count = self.argument_count?;
        let index = written.checked_add(self.explicit_offset?)?;
        if index >= count || !signature.arity().accepts(count) {
            return None;
        }
        signature.args.get(index).or(signature.rest.as_ref())
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

    pub(super) fn completion(span: Span, names: &[&'a str]) -> Self {
        Self::Completion(CompletionQuery {
            span,
            types: names
                .iter()
                .map(|&name| (name, Some(Type::Never)))
                .collect(),
        })
    }

    pub(super) fn bindings(
        enclosing: Span,
        sites: impl Iterator<Item = (Span, bool)>,
    ) -> Option<Self> {
        let types: BTreeMap<_, _> = sites
            .map(|(span, literal_initializer)| {
                (
                    span.start.offset,
                    BindingObservation {
                        span,
                        ty: Some(Type::Never),
                        literal_initializer,
                    },
                )
            })
            .collect();
        (!types.is_empty()).then_some(Self::Bindings(BindingQueries { enclosing, types }))
    }

    pub(super) fn span(&self) -> Span {
        match self {
            Self::Local { span, .. } | Self::Lookup { span, .. } => *span,
            Self::Call(query) => query.span,
            Self::Bindings(query) => query.enclosing,
            Self::Completion(query) => query.span,
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

    pub(super) fn into_completion_types(self) -> Option<BTreeMap<String, String>> {
        let Self::Completion(query) = self else {
            return None;
        };
        Some(
            query
                .types
                .into_iter()
                .filter_map(|(name, ty)| {
                    let ty = ty.filter(completion_detail_type)?;
                    Some((name.to_owned(), ty.to_string()))
                })
                .collect(),
        )
    }

    pub(super) fn includes_literal_bindings(&self) -> bool {
        matches!(self, Self::Bindings(query)
            if query.types.values().any(|binding| binding.literal_initializer))
    }

    pub(super) fn into_binding_types(
        self,
        literal_only: bool,
    ) -> impl Iterator<Item = (Span, Type)> {
        let types = match self {
            Self::Bindings(query) => query.types,
            _ => BTreeMap::new(),
        };
        types.into_values().filter_map(move |binding| {
            let ty = binding.ty?;
            (ty != Type::Never && (!literal_only || binding.literal_initializer))
                .then_some((binding.span, ty))
        })
    }
}

fn completion_detail_type(ty: &Type) -> bool {
    !matches!(ty, Type::Never | Type::Any) && !ty.contains_unknown()
}

impl<'model> Checker<'model> {
    pub(super) fn observe_completion(&mut self, span: Span, env: &Env) {
        if self.path_reachable
            && let Some(TypeQuery::Completion(query)) = &mut self.type_query
            && query.span == span
        {
            for (&name, observed) in &mut query.types {
                let Some(previous) = observed else {
                    continue;
                };
                let Some(actual) = env
                    .get(name)
                    .map(|value| &value.known)
                    .filter(|ty| completion_detail_type(ty))
                else {
                    // Every reachable visit must support the detail.
                    *observed = None;
                    continue;
                };
                if previous != actual {
                    *previous =
                        Type::union([std::mem::replace(previous, Type::Never), actual.clone()]);
                }
            }
        }
    }

    pub(super) fn observe_binding(&mut self, name: &str, span: Span, value: &Value) {
        self.observe_local(name, span, value);
        if self.path_reachable
            && let Some(TypeQuery::Bindings(query)) = &mut self.type_query
            && let Some(binding) = query.types.get_mut(&span.start.offset)
            && binding.span == span
            && let Some(previous) = &mut binding.ty
        {
            if value.known == Type::Any || value.known.contains_unknown() {
                binding.ty = None;
            } else if *previous != value.known {
                let combined = Type::union([
                    std::mem::replace(previous, Type::Never),
                    value.known.clone(),
                ]);
                binding.ty =
                    (!combined.contains_unknown() && combined != Type::Any).then_some(combined);
            }
        }
    }

    pub(super) fn withhold_literals_after_opaque_form(&mut self) {
        if self.path_reachable
            && self.literal_binding_query
            && let Some(TypeQuery::Bindings(query)) = &mut self.type_query
        {
            // An opaque form can alter which following expressions are
            // evaluated in this lexical body.
            for binding in query.types.values_mut() {
                if binding.literal_initializer && binding.ty == Some(Type::Never) {
                    binding.ty = None;
                }
            }
        }
    }

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
        let Some(query) = self.call_query_at(span) else {
            return;
        };
        let Some(forms) = arguments
            .written_forms()
            .filter(|forms| forms.len() == arguments.values.len())
        else {
            return;
        };
        let checked = CheckedCall {
            signature: signature.clone(),
            origin: origin.cloned(),
            explicit_offset: None,
            argument_count: None,
            declaration: CallDeclaration::Typed(declared.clone()),
        };
        if !query.record(Some(checked), forms) {
            self.type_query = None;
        }
    }

    pub(super) fn observes_inferred_call_at(&self, span: Span) -> bool {
        self.path_reachable
            && matches!(&self.type_query, Some(TypeQuery::Call(query))
                if query.span == span && !query.declared_only)
    }

    pub(super) fn observe_inferred_call(
        &mut self,
        span: Span,
        forms: &[Form],
        call: Option<InferredCall>,
        result: &Value,
    ) {
        let Some(query) = self.call_query_at(span) else {
            return;
        };
        let checked = call
            .filter(|_| result.known != Type::Never && !result.known.contains_unknown())
            .map(|call| CheckedCall {
                signature: Signature {
                    args: call.args,
                    ret: result.known.clone(),
                    rest: None,
                    parameters: None,
                    nil_preserving: false,
                },
                origin: None,
                explicit_offset: None,
                argument_count: None,
                declaration: CallDeclaration::Inferred {
                    definition: call.definition,
                    labels: call.labels,
                },
            });
        if !query.record(checked, forms) {
            self.type_query = None;
        }
    }

    fn call_query_at(&mut self, span: Span) -> Option<&mut CallQuery<'model>> {
        if self.path_reachable
            && let Some(TypeQuery::Call(query)) = &mut self.type_query
            && query.span == span
        {
            Some(query)
        } else {
            None
        }
    }

    pub(super) fn withhold_type_facts(&mut self, enclosing: Span) {
        if !self.path_reachable {
            return;
        }
        if let Some(query) = &mut self.type_query {
            if let TypeQuery::Bindings(batch) = query {
                for (_, binding) in batch
                    .types
                    .range_mut(enclosing.start.offset..enclosing.end.offset)
                {
                    if binding.span.end.offset <= enclosing.end.offset {
                        binding.ty = None;
                    }
                }
                return;
            }
            let site = query.span();
            if enclosing.start.offset <= site.start.offset
                && site.end.offset <= enclosing.end.offset
            {
                match query {
                    TypeQuery::Local { .. }
                    | TypeQuery::Lookup { .. }
                    | TypeQuery::Completion(_) => {
                        self.type_query = None;
                    }
                    TypeQuery::Call(query) => query.use_declaration(),
                    TypeQuery::Bindings(_) => unreachable!(),
                }
            }
        }
    }
}

impl CallQuery<'_> {
    fn record(&mut self, checked: Option<CheckedCall>, forms: &[Form]) -> bool {
        let explicit_offset = if self.written.is_empty() {
            // An implicit-only call does not prove which slot new written
            // arguments would occupy.
            forms.is_empty().then_some(0)
        } else {
            let Some(offset) = explicit_offset(self.written, forms) else {
                return true;
            };
            Some(offset)
        };
        let Some(mut checked) = checked else {
            self.use_declaration();
            return true;
        };
        checked.argument_count = Some(forms.len());
        checked.explicit_offset = explicit_offset;
        if let Some(previous) = &self.checked {
            if previous.declaration != checked.declaration
                || previous.origin != checked.origin
                || previous.explicit_offset != checked.explicit_offset
                || previous.argument_count != checked.argument_count
            {
                return false;
            }
            if previous.signature != checked.signature {
                self.use_declaration();
            }
        } else {
            self.checked = Some(checked);
            if self.declared_only {
                self.use_declaration();
            }
        }
        true
    }

    fn use_declaration(&mut self) {
        self.declared_only = true;
        if let Some(checked) = &mut self.checked {
            match &checked.declaration {
                CallDeclaration::Typed(declared) => checked.signature = declared.clone(),
                CallDeclaration::Inferred { .. } => self.checked = None,
            }
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
