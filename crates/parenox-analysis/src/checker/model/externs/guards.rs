//! Function externs with trusted branch refinements.
use std::sync::Arc;

use super::{Contract, ExternalDeclaration, Globals, Model};
use crate::ContractKind;
use crate::checker::model::guards::{GuardMode, VerifiedGuard, scalar_predicate};
use crate::reader::{Form, Span};
use crate::types::Type;

impl Model {
    pub(super) fn read_guarded_extern(
        &mut self,
        entries: &[Form],
        span: Span,
        name: &str,
        path: &str,
        source: &str,
        globals: &Globals,
    ) -> Option<ExternalDeclaration> {
        let fields = match entries {
            [a, first, b, second] => match (a.keyword(), b.keyword()) {
                (Some(":sig"), Some(":guard")) => Some((first, second)),
                (Some(":guard"), Some(":sig")) => Some((second, first)),
                _ => None,
            },
            _ => None,
        };
        let Some((signature, claim)) = fields else {
            self.error(
                "PNX9008",
                span,
                "Expected {:sig [...] :guard [:then argument-index predicate?]}",
            );
            return None;
        };
        let signatures = self.parse_extern_signatures(signature, source, name, globals)?;
        let guard = (|| {
            let [mode, index, predicate] = claim.vector()? else {
                return None;
            };
            let mode = GuardMode::parse(mode)?;
            let argument = Type::integer_literal(index, source)?
                .exact_integer_value()
                .and_then(|index| usize::try_from(index).ok())?
                .checked_sub(1)?;
            if !signatures.iter().all(|signature| {
                mode.supports_signature(signature) && argument < signature.args.len()
            }) {
                return None;
            }
            let ty = scalar_predicate(&self.resolve(predicate.symbol()?))?;
            Some(VerifiedGuard {
                argument,
                arities: signatures
                    .iter()
                    .map(|signature| signature.args.len())
                    .collect(),
                ty,
                mode,
            })
        })();
        let Some(guard) = guard else {
            self.error("PNX9008", claim.span, "Extern guard requires :then, :both, or :some, a supported scalar predicate, and a valid 1-based argument index in every fixed signature. :then and :both require Boolean results");
            return None;
        };
        let mut contract = Contract::new(signatures, name, ContractKind::Extern, path);
        contract.guard = Some(Arc::new(guard));
        Some(ExternalDeclaration::Function(contract))
    }
}
