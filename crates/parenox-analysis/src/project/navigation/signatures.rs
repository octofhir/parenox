//! Argument positions and parameter lists for source calls.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ContractOrigin;
use crate::checker::{CheckedCall, Declarations};
use crate::reader::{self, Conditionals, Form, Kind, Span};
use crate::syntax::SyntaxKind;
use crate::types::Arity;

use super::{FileCache, Session, SourceTarget, cached_type_query};

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

impl Session {
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
        let (declarations, forms) = recover_call(path, &caller.source)?;
        let site = call_at(&forms, offset)?;
        let source = self.signatures_at(path, &declarations, &forms, &site);
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
                    &caller.source,
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
                let (namespace, name) = declarations.top_level_call(site.head.symbol()?);
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

/// Repair missing closing delimiters at EOF for this query alone.
fn recover_call(path: &str, source: &str) -> Option<(Declarations, Vec<Form>)> {
    use SyntaxKind::*;
    let tree = reader::parse(source).syntax();
    let mut closers = Vec::new();
    for node in tree
        .descendants()
        .filter(|node| node.kind() == Error && usize::from(node.text_range().end()) == source.len())
    {
        let closer = match node.first_child_or_token()?.kind() {
            OpenParen | OpenAnonymousFunction => ')',
            OpenBracket => ']',
            OpenBrace | OpenSet => '}',
            _ => return None,
        };
        closers.push(closer);
    }
    if closers.is_empty() {
        return None;
    }
    let mut repaired = source.to_owned();
    repaired.push('\n');
    repaired.extend(closers.into_iter().rev());
    let conditionals = if path.ends_with(".cljc") {
        Conditionals::Select(":clj")
    } else {
        Conditionals::Reject
    };
    let (mut read, error) = reader::read_for_check(&repaired, conditionals, None);
    if error.is_some() {
        return None;
    }
    let declarations = Declarations::read_source(&repaired, &mut read).ok()?;
    Some((declarations, read.forms))
}
