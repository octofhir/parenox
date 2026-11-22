//! Connect failed type obligations to the literal expression that supplied them.
use crate::checker::Checker;
use crate::reader::{Form, Kind};
use crate::types::Type;
use crate::{Diagnostic, ExternMismatchReason, ExternPathStep};

impl Checker<'_> {
    pub(super) fn refine_argument_mismatch(
        &mut self,
        diagnostic: usize,
        argument: usize,
        form: &Form,
        actual: &Type,
        expected: &Type,
    ) {
        let Some(failure) = actual.explain_mismatch(expected).filter(|failure| {
            failure.path.iter().any(|step| {
                matches!(
                    step,
                    ExternPathStep::Field { .. }
                        | ExternPathStep::Item { .. }
                        | ExternPathStep::Element
                        | ExternPathStep::MapKey
                        | ExternPathStep::MapValue
                )
            })
        }) else {
            return;
        };
        let leaf = match &failure.reason {
            ExternMismatchReason::TypeMismatch { expected, actual } => self
                .literal_mismatch_site(form, &failure.path)
                .map(|site| (site.span, expected.clone(), actual.clone())),
            _ => None,
        };
        let fallback = leaf
            .is_none()
            .then(|| format!("Argument {argument} → {failure}"));
        let diagnostic = &mut self.diagnostics[diagnostic];
        if let Some((span, expected, actual)) = leaf {
            let context = std::iter::once(format!("Argument {argument}"))
                .chain(failure.path.iter().map(ToString::to_string))
                .collect::<Vec<_>>()
                .join(" → ");
            let mut localized = Diagnostic::type_mismatch(span, &context, expected, actual);
            localized.contract = diagnostic.contract.take();
            localized.notes = std::mem::take(&mut diagnostic.notes);
            *diagnostic = localized;
        }
        diagnostic.notes.extend(fallback);
        diagnostic.mismatch_path = failure.path;
        diagnostic
            .mismatch_path
            .insert(0, ExternPathStep::Argument { index: argument });
    }

    fn literal_mismatch_site<'form>(
        &self,
        mut form: &'form Form,
        path: &[ExternPathStep],
    ) -> Option<&'form Form> {
        let mut path = path.iter().peekable();
        while let Some(step) = path.next() {
            form = match step {
                ExternPathStep::Field { key } => {
                    let entries = form.map()?.as_chunks::<2>().0;
                    let [key, value] = if let [entry] = entries
                        && matches!(entry[0].kind, Kind::Symbol(_))
                    {
                        // A checked single-entry map has only one written
                        // value that can supply the field in this mismatch.
                        entry
                    } else {
                        let mut fields = entries.iter().filter(|[candidate, _]| {
                            self.written_map_key(candidate)
                                .is_some_and(|candidate| candidate.to_string() == *key)
                        });
                        let field = fields.next()?;
                        if fields.next().is_some() {
                            return None;
                        }
                        field
                    };
                    if matches!(path.peek(), Some(ExternPathStep::MapKey)) {
                        path.next();
                        key
                    } else {
                        value
                    }
                }
                ExternPathStep::Item { index } => form.vector()?.get(*index)?,
                _ => return None,
            };
        }
        Some(form.without_metadata())
    }
}
