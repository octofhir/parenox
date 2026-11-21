//! Argument positions and parameter lists for source calls.
use std::sync::Arc;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use crate::ContractOrigin;
use crate::checker::{CheckedCall, Declarations};
use crate::reader::{self, Form, Kind, Span};
use crate::syntax::SyntaxKind;
use crate::types::{Arity, MapType, Type};

use super::{
    FileCache, Session, SourceTarget, cached_type_query, recovered_types::close_eof_delimiters,
};

pub struct CallSignature {
    pub parameters: Vec<String>,
    pub result: Option<String>,
    arity: Arity,
}

pub struct CallSignatures {
    pub name: String,
    pub documentation: Option<String>,
    pub contract: Option<ContractOrigin>,
    pub signatures: Vec<CallSignature>,
    pub active_signature: usize,
    pub active_parameter: Option<usize>,
}

#[derive(Clone)]
pub enum ArgumentKeyword {
    Value(String),
    Field {
        keyword: String,
        required: bool,
        value_type: Option<String>,
    },
}

impl ArgumentKeyword {
    fn keyword(&self) -> &str {
        match self {
            Self::Value(keyword) | Self::Field { keyword, .. } => keyword,
        }
    }
}

impl Session {
    /// Complete a written keyword with values from an observed call's declaration.
    pub fn complete_argument_keyword(
        &self,
        path: &str,
        token: Range<usize>,
        offset: usize,
        limit: usize,
    ) -> Option<Vec<ArgumentKeyword>> {
        let caller = self.current_file(path)?;
        let prefix = caller.source.get(token.start..offset)?;
        if !prefix.starts_with(':') || prefix.starts_with("::") || offset > token.end {
            return None;
        }
        if let Some((declarations, forms)) = caller.navigation_source()
            && let Some(site) = call_at(forms, offset)
        {
            let slot = site.keyword_slot(&caller.source, &token)?;
            let checked = self.checked_call(caller, path, &site)?;
            let expected = checked.declared_argument_type(site.argument)?;
            return slot.choices(expected, declarations, prefix, limit);
        }
        let cutoff = caller.parse_error.as_ref()?.span.start.offset;
        if token.start < cutoff {
            return None;
        }
        let fallback = caller.navigation_fallback.as_deref()?;
        let candidates = cached_type_query(&fallback.recovered_keywords, token.start, || {
            let source = repair_keyword_source(&caller.source, &token)?;
            let repaired = self.repaired_type_scope(caller, path, &source, None)?;
            let scope = repaired.scope(&source);
            let site = call_at(scope.forms, token.end)?;
            let slot = site.keyword_slot(scope.source, &token)?;
            let checked = scope.declarations.checked_call(
                scope.forms,
                scope.model,
                scope.globals,
                scope.source,
                site.span,
                site.written,
            )?;
            let expected = checked.declared_argument_type(site.argument)?;
            slot.choices(expected, scope.declarations, ":", usize::MAX)
                .map(Arc::from)
        })?;
        Some(
            candidates
                .iter()
                .filter(|candidate| candidate.keyword().starts_with(prefix))
                .take(limit)
                .cloned()
                .collect(),
        )
    }

    /// Find the innermost source call whose argument list contains the cursor.
    pub fn call_signatures(&self, path: &str, offset: usize) -> Option<CallSignatures> {
        let caller = self.current_file(path)?;
        if !caller.source.is_char_boundary(offset) {
            return None;
        }
        if let Some((declarations, forms)) = caller.navigation_source()
            && let Some(site) = call_at(forms, offset)
        {
            let source = self.signatures_at(path, declarations, forms, &site);
            return match self.checked_call(caller, path, &site) {
                Some(checked) => Some(typed_signatures(&site, &checked, source)),
                None => source,
            };
        }
        caller.parse_error.as_ref()?;
        let (declarations, forms) = self.recovered_delimited_source(caller, path)?.navigation();
        let site = call_at(forms, offset)?;
        let source = self.signatures_at(path, declarations, forms, &site);
        let declared = self.query_globals.as_ref().and_then(|globals| {
            declarations.qualified_call_declaration(globals, site.head.symbol()?, site.wanted())
        });
        match declared {
            Some(signature) => Some(typed_signatures(&site, &signature, source)),
            None => source,
        }
    }

    fn checked_call(
        &self,
        caller: &FileCache,
        path: &str,
        site: &CallSite<'_>,
    ) -> Option<Arc<CheckedCall>> {
        cached_type_query(&caller.checked_calls, site.span.start.offset, || {
            let scope = self.typing_at(caller, path, site.span.start.offset)?;
            scope
                .declarations
                .checked_call(
                    scope.forms,
                    scope.model,
                    scope.globals,
                    scope.source,
                    site.span,
                    site.written,
                )
                .map(Arc::new)
        })
    }

    fn signatures_at(
        &self,
        path: &str,
        declarations: &Declarations,
        forms: &[Form],
        site: &CallSite<'_>,
    ) -> Option<CallSignatures> {
        let head_offset = site.head.without_metadata().span.start.offset;
        let target = match self.source_target_in(path, head_offset, declarations, forms) {
            Some(SourceTarget::Var(target)) => target,
            None if site.top_level => {
                let symbol = site.head.symbol()?;
                if declarations.ordinary_dotted_function(forms, symbol) {
                    return None;
                }
                let (namespace, name) = declarations.top_level_call(symbol);
                let before = (namespace == declarations.namespace()).then_some(head_offset);
                self.resolve_source_var(path, namespace, name, before)?
            }
            _ => return None,
        };
        let (declarations, provider_forms) = target.source.navigation_source()?;
        let parameters = declarations.function_parameters(provider_forms, target.name)?;
        let active_signature = select_arity(parameters.clauses.keys().copied(), site.wanted())?;
        let signatures = source_signatures(&target.source.source, parameters.clauses);
        let active_parameter = signatures[active_signature]
            .parameters
            .len()
            .checked_sub(1)
            .map(|last| last.min(site.argument));
        Some(CallSignatures {
            name: target.name.to_owned(),
            documentation: parameters.documentation.map(str::to_owned),
            contract: None,
            signatures,
            active_signature,
            active_parameter,
        })
    }
}

fn keyword_at_token(form: &Form, token: &Range<usize>) -> bool {
    matches!(form.kind, Kind::Keyword(_))
        && form.span.start.offset == token.start
        && form.span.end.offset == token.end
}

fn complete_keyword_values(
    expected: &Type,
    prefix: &str,
    limit: usize,
) -> Option<Vec<ArgumentKeyword>> {
    let mut values = Vec::new();
    if !keyword_values(expected, &mut values) || values.is_empty() {
        return None;
    }
    values.sort_unstable();
    values.dedup();
    Some(
        values
            .into_iter()
            .filter(|value| value.starts_with(prefix))
            .take(limit)
            .map(|value| ArgumentKeyword::Value(value.to_owned()))
            .collect(),
    )
}

fn keyword_values<'a>(ty: &'a Type, values: &mut Vec<&'a str>) -> bool {
    match ty {
        Type::Key(value) => {
            values.push(value);
            true
        }
        Type::Nil => true,
        Type::Union(parts) => parts.iter().all(|part| keyword_values(part, values)),
        _ => false,
    }
}

fn complete_map_keyword(
    map: &MapType,
    declarations: &Declarations,
    entries: &[Form],
    index: usize,
    prefix: &str,
    limit: usize,
) -> Option<Vec<ArgumentKeyword>> {
    if index % 2 == 1 {
        let key = literal_field_key(declarations, &entries[index - 1])?;
        let expected = map.fields.get(&key)?.value_constraint()?;
        return complete_keyword_values(expected, prefix, limit);
    }

    if !map.fields.keys().any(|key| matches!(key, Type::Key(_))) {
        return None;
    }
    let mut present = BTreeSet::new();
    for (other, form) in entries.iter().enumerate().step_by(2) {
        if other == index {
            continue;
        }
        if let Some(key) = literal_field_key(declarations, form) {
            present.insert(key);
        } else if !matches!(
            &form.kind,
            Kind::Nil
                | Kind::Bool(_)
                | Kind::Number
                | Kind::SymbolicNumber(_)
                | Kind::Character(_)
                | Kind::Utf16String(_)
        ) {
            // Replacing the current key could duplicate a computed sibling.
            return None;
        }
    }
    let mut fields: Vec<_> = map
        .fields
        .iter()
        .filter_map(|(key, field)| {
            let Type::Key(keyword) = key else {
                return None;
            };
            (field.ty != Type::Never && keyword.starts_with(prefix) && !present.contains(key))
                .then_some((keyword, field))
        })
        .collect();
    fields.sort_unstable_by_key(|(keyword, field)| (!field.required, *keyword));
    Some(
        fields
            .into_iter()
            .take(limit)
            .map(|(keyword, field)| ArgumentKeyword::Field {
                keyword: keyword.clone(),
                required: field.required,
                value_type: field.value_constraint().map(ToString::to_string),
            })
            .collect(),
    )
}

fn literal_field_key(declarations: &Declarations, form: &Form) -> Option<Type> {
    match &form.kind {
        Kind::Keyword(keyword) if keyword.starts_with("::") => {
            declarations.resolve_spec_keyword(keyword).map(Type::Key)
        }
        Kind::Keyword(keyword) => Some(Type::Key(keyword.clone())),
        Kind::String(key) => Some(Type::Str(key.clone())),
        _ => None,
    }
}

fn typed_signatures(
    site: &CallSite<'_>,
    checked: &CheckedCall,
    source: Option<CallSignatures>,
) -> CallSignatures {
    let source = source.filter(|_| checked.local_labels().is_none());
    let mut info = source.unwrap_or_else(|| CallSignatures {
        name: checked.origin.as_ref().map_or_else(
            || site.head.symbol().unwrap().to_owned(),
            |origin| origin.name.clone(),
        ),
        documentation: None,
        contract: None,
        signatures: Vec::new(),
        active_signature: 0,
        active_parameter: None,
    });
    let arity = checked.signature.arity();
    let source_arity = info
        .signatures
        .iter()
        .position(|signature| signature.arity == arity);
    let labels = source_arity
        .map(|index| info.signatures[index].parameters.as_slice())
        .or_else(|| checked.local_labels());
    let parameters = checked
        .signature
        .args
        .iter()
        .chain(&checked.signature.rest)
        .enumerate()
        .map(|(index, ty)| {
            let label = labels.and_then(|names| names.get(index));
            if let Some(label) = label {
                format!("{label}: {ty}")
            } else if matches!(arity, Arity::Variadic(required) if index == required) {
                format!("& {ty}")
            } else {
                ty.to_string()
            }
        })
        .collect::<Vec<_>>();
    let signature = CallSignature {
        parameters,
        result: Some(checked.signature.ret.to_string()),
        arity,
    };
    if let Some(index) = source_arity {
        info.signatures[index] = signature;
        info.active_signature = index;
    } else {
        info.active_signature = info.signatures.len();
        info.signatures.push(signature);
    }
    if let Some(count) = checked.argument_count {
        let wanted = count + usize::from(site.argument == site.written.len());
        info.active_signature = select_arity(info.signatures.iter().map(|sig| sig.arity), wanted)
            .unwrap_or(info.active_signature);
    }
    info.active_parameter = checked.explicit_offset.and_then(|offset| {
        info.signatures[info.active_signature]
            .parameters
            .len()
            .checked_sub(1)
            .map(|last| last.min(site.argument + offset))
    });
    info.contract = checked.origin.as_deref().cloned();
    info
}

pub(super) fn source_signatures(
    source: &str,
    clauses: BTreeMap<Arity, Vec<Span>>,
) -> Vec<CallSignature> {
    clauses
        .into_iter()
        .map(|(arity, spans)| {
            let parameters = spans
                .iter()
                .enumerate()
                .map(|(index, span)| {
                    let text = &source[span.start.offset..span.end.offset];
                    if matches!(arity, Arity::Variadic(required) if required == index) {
                        format!("& {text}")
                    } else {
                        text.to_owned()
                    }
                })
                .collect();
            CallSignature {
                parameters,
                result: None,
                arity,
            }
        })
        .collect()
}

fn select_arity(arities: impl Iterator<Item = Arity> + Clone, count: usize) -> Option<usize> {
    arities
        .clone()
        .position(|arity| arity.accepts(count))
        .or_else(|| arities.clone().position(|arity| arity.required() >= count))
        .or_else(|| arities.count().checked_sub(1))
}

struct CallSite<'a> {
    head: &'a Form,
    span: Span,
    written: &'a [Form],
    argument: usize,
    top_level: bool,
}

impl CallSite<'_> {
    fn wanted(&self) -> usize {
        self.written.len().max(self.argument + 1)
    }

    fn keyword_slot(&self, source: &str, token: &Range<usize>) -> Option<KeywordSlot<'_>> {
        let argument = self.written.get(self.argument)?;
        if keyword_at_token(argument, token) {
            return Some(KeywordSlot::Direct);
        }
        let Kind::Map(entries) = &argument.kind else {
            return None;
        };
        if source.as_bytes().get(argument.span.start.offset) != Some(&b'{') {
            return None;
        }
        let index = entries
            .iter()
            .position(|form| keyword_at_token(form, token))?;
        Some(KeywordSlot::Map(entries, index))
    }
}

enum KeywordSlot<'a> {
    Direct,
    Map(&'a [Form], usize),
}

impl KeywordSlot<'_> {
    fn choices(
        &self,
        expected: &Type,
        declarations: &Declarations,
        prefix: &str,
        limit: usize,
    ) -> Option<Vec<ArgumentKeyword>> {
        match self {
            Self::Direct => complete_keyword_values(expected, prefix, limit),
            Self::Map(entries, index) => {
                let Type::Map(map) = expected else {
                    return None;
                };
                complete_map_keyword(map, declarations, entries, *index, prefix, limit)
            }
        }
    }
}

fn call_at(forms: &[Form], offset: usize) -> Option<CallSite<'_>> {
    call_in(child_at(forms, offset)?, offset, true)
}

fn child_at(forms: &[Form], offset: usize) -> Option<&Form> {
    let index = forms.partition_point(|form| form.span.end.offset <= offset);
    let form = forms.get(index)?;
    (form.span.start.offset <= offset).then_some(form)
}

fn call_in(form: &Form, offset: usize, top_level: bool) -> Option<CallSite<'_>> {
    let form = form.without_metadata();
    match &form.kind {
        Kind::List(items) => {
            let (head, args) = items.split_first()?;
            if matches!(
                head.symbol(),
                Some("quote" | "clojure.core/quote" | "ns" | "clojure.core/ns")
            ) {
                return None;
            }
            if let Some(site) =
                child_at(items, offset).and_then(|child| call_in(child, offset, false))
            {
                return Some(site);
            }
            (head.symbol().is_some() && head.span.end.offset < offset).then(|| CallSite {
                head,
                span: form.span,
                written: args,
                argument: args.partition_point(|arg| arg.span.end.offset < offset),
                top_level,
            })
        }
        Kind::Vector(items)
        | Kind::Map(items)
        | Kind::Set(items)
        | Kind::AutoNamespacedMap { entries: items, .. } => {
            call_in(child_at(items, offset)?, offset, false)
        }
        Kind::AnonymousFunction(function) => {
            call_in(child_at(&function.body, offset)?, offset, false)
        }
        _ => None,
    }
}

fn repair_keyword_source(source: &str, token: &Range<usize>) -> Option<String> {
    if !is_last_key_without_value(source, token).unwrap_or(false) {
        return close_eof_delimiters(source);
    }
    let mut repaired = source.to_owned();
    repaired.insert_str(token.end, " nil");
    close_eof_delimiters(&repaired).or(Some(repaired))
}

fn is_last_key_without_value(source: &str, range: &Range<usize>) -> Option<bool> {
    use SyntaxKind::*;
    let tree = reader::parse(source).syntax();
    let token = tree
        .token_at_offset(u32::try_from(range.start).ok()?.into())
        .find(|token| {
            let bytes = token.text_range();
            token.kind() == Keyword
                && usize::from(bytes.start()) == range.start
                && usize::from(bytes.end()) == range.end
        })?;
    let parent = token.parent()?;
    if parent.kind() != Error || parent.first_child_or_token()?.kind() != OpenBrace {
        return Some(false);
    }
    let mut count = 0;
    let mut last = None;
    for entry in parent.children_with_tokens().skip(1).filter(|element| {
        !element.kind().is_trivia() && !matches!(element.kind(), Discard | CloseBrace)
    }) {
        count += 1;
        last = Some(entry);
    }
    Some(count % 2 == 1 && last?.into_token()? == token)
}
