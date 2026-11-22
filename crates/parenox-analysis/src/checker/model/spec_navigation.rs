//! Find named spec references in source-backed type positions.
use super::Declarations;
use super::contracts::function_parts;
use super::type_forms::{read_function_spec, read_type, read_type_with_parameters};
use crate::checker::function_forms::split_rest;
use crate::reader::{Form, Kind, Span};
use crate::types::Type;

impl Declarations {
    pub(crate) fn spec_type_reference_at(
        &self,
        forms: &[Form],
        source: &str,
        offset: usize,
    ) -> Option<String> {
        let index = forms.partition_point(|form| form.span.end.offset <= offset);
        let form = forms.get(index)?.without_metadata();
        let Kind::List(items) = &form.kind else {
            return None;
        };
        let [head, name, tail @ ..] = items.as_slice() else {
            return None;
        };
        let mut diagnostics = Vec::new();
        let mut reference = None;
        let lookup = |key: &str, span: Span| {
            if span_at(span, offset) {
                reference = Some(key.to_owned());
            }
            // Only reference positions matter. A placeholder prevents a
            // missing definition from stopping traversal of sibling types.
            Some(Type::String)
        };
        if self.functions.binary_search(&index).is_ok() {
            let signature = function_parts(tail)
                .1
                .find_map(|[key, value]| (key.keyword() == Some(":t/sig")).then_some(value))
                .or_else(|| name.metadata(":t/sig"))?;
            let ty = signature_type_at(signature, offset)?;
            read_type_with_parameters(
                &self.names,
                ty,
                source,
                None,
                None,
                &mut diagnostics,
                lookup,
            );
        } else if self.fdefs.binary_search(&index).is_ok() {
            read_function_spec(
                &self.names,
                tail,
                form.span,
                source,
                None,
                &mut diagnostics,
                lookup,
            );
        } else if let [body] = tail
            && name.keyword().is_some()
            && self.names.resolve_parts(head.symbol()?) == ("clojure.spec.alpha", "def")
            && span_at(body.span, offset)
        {
            read_type(&self.names, body, source, &mut diagnostics, lookup);
        }
        reference
    }

    pub(crate) fn resolve_spec_keyword(&self, keyword: &str) -> Option<String> {
        // Navigation can use a complete prefix without having validated its
        // reader namespace tokens. Don't guess an unresolved auto-alias.
        self.names.validation_key(keyword)
    }
}

fn signature_type_at(form: &Form, offset: usize) -> Option<&Form> {
    let form = form.quote_operand()?.without_metadata();
    let (Kind::Vector(items) | Kind::List(items)) = &form.kind else {
        return None;
    };
    let signature = if let [op, _, signature] = items.as_slice()
        && matches!(&form.kind, Kind::List(_))
        && op.symbol() == Some("t/all")
    {
        signature
    } else if !items.is_empty() && items.iter().all(|arm| arm.vector().is_some()) {
        items.iter().find(|arm| span_at(arm.span, offset))?
    } else {
        form
    };
    let [parameters @ .., arrow, result] = signature.vector()? else {
        return None;
    };
    if arrow.keyword() != Some(":->") || parameters.iter().any(|t| t.keyword() == Some(":->")) {
        return None;
    }
    let (args, rest) = split_rest(parameters).ok()?;
    args.iter()
        .chain(rest)
        .chain([result])
        .find(|form| span_at(form.span, offset))
}

fn span_at(span: Span, offset: usize) -> bool {
    (span.start.offset..span.end.offset).contains(&offset)
}
