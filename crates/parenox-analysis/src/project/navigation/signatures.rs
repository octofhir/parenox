//! Argument positions and parameter lists for source calls.
use std::collections::BTreeMap;

use crate::checker::Declarations;
use crate::reader::{self, Conditionals, Form, Kind, Span};
use crate::syntax::SyntaxKind;
use crate::types::Arity;

use super::{Session, SourceTarget};

pub struct CallSignature {
    pub parameters: Vec<String>,
}

pub struct CallSignatures {
    pub name: String,
    pub documentation: Option<String>,
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
            return self.signatures_at(path, declarations, forms, site);
        }
        caller.parse_error.as_ref()?;
        let (declarations, forms) = recover_call(path, &caller.source)?;
        self.signatures_at(path, &declarations, &forms, call_at(&forms, offset)?)
    }

    fn signatures_at(
        &self,
        path: &str,
        declarations: &Declarations,
        forms: &[Form],
        site: CallSite<'_>,
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
        let wanted = site.count.max(site.argument + 1);
        let active_signature = select_arity(parameters.clauses.keys().copied(), wanted)?;
        let signatures = source_signatures(&target.source.source, parameters.clauses);
        let active_parameter = signatures[active_signature]
            .parameters
            .len()
            .checked_sub(1)
            .map(|last| last.min(site.argument));
        Some(CallSignatures {
            name: target.name.to_owned(),
            documentation: parameters.documentation.map(str::to_owned),
            signatures,
            active_signature,
            active_parameter,
        })
    }
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
            CallSignature { parameters }
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
    argument: usize,
    count: usize,
    top_level: bool,
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
                argument: args.partition_point(|arg| arg.span.end.offset < offset),
                count: args.len(),
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
