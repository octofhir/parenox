//! Inert EDN syntax checks with the shared reader.
use super::{Conditionals, Form, Item, Kind, Position, ReadError, Reader, Span};

/// Reads a stream of data forms and reports the first reader error.
/// Tagged values are handled as data without calling their handlers.
pub fn validate_edn(source: &str) -> Result<(), ReadError> {
    let mut reader = Reader::new(source, false, Conditionals::Reject);
    reader.edn = true;
    while reader.next_form()?.is_some() {}
    Ok(())
}

impl Reader<'_> {
    pub(super) fn edn_dispatch_prefix(&self) -> bool {
        matches!(
            self.source
                .as_bytes()
                .get(self.pos.offset..self.pos.offset.saturating_add(2)),
            Some([b'#', b'(' | b'"' | b':' | b'#' | b'^'])
        )
    }

    pub(super) fn edn_dispatch(&mut self, start: Position) -> Result<Item, ReadError> {
        self.bump(); // #
        let kind = match self.peek() {
            Some(':') => self.edn_map(start)?,
            Some('#') => self.edn_symbolic_value(start)?,
            Some('^') => {
                self.bump();
                self.metadata_form(start)?
            }
            _ => return Err(self.error(start, "Clojure reader form is unavailable in EDN")),
        };
        Ok(Item::Form(Form {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        }))
    }

    fn edn_map(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.bump(); // :
        let name_start = self.pos.offset;
        while self.peek().is_some_and(|ch| !Self::token_boundary(ch)) {
            self.bump();
        }
        let name = &self.source[name_start..self.pos.offset];
        if name.is_empty()
            || name.starts_with(':')
            || name.contains('/')
            || super::numbers::starts_number(name)
        {
            return Err(self.error(start, "EDN namespaced map needs an unqualified namespace"));
        }
        self.skip()?;
        if self.peek() != Some('{') {
            return Err(self.error(start, "Expected a map after the EDN namespace"));
        }
        // Validation needs the map's syntax. Its keys are never resolved or evaluated.
        Ok(self.form()?.kind)
    }

    fn edn_symbolic_value(&mut self, start: Position) -> Result<Kind, ReadError> {
        self.bump(); // #
        let value_start = self.pos.offset;
        while self.peek().is_some_and(|ch| !Self::token_boundary(ch)) {
            self.bump();
        }
        match &self.source[value_start..self.pos.offset] {
            "Inf" | "-Inf" | "NaN" => Ok(Kind::Number),
            _ => Err(self.error(start, "Unknown EDN symbolic value")),
        }
    }
}
