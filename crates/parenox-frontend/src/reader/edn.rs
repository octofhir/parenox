//! Inert EDN syntax checks with the shared reader.
use super::{Conditionals, Form, Item, Kind, Position, ReadError, Reader, Span};

/// Reads a stream of data forms and reports the first reader error.
/// Tagged values are handled as data without calling their handlers.
pub fn validate_edn(source: &str) -> Result<(), ReadError> {
    let mut reader = data_reader(source);
    while reader.next_form()?.is_some() {}
    Ok(())
}

/// Reads inert EDN forms, with the same syntax rules as `validate_edn`.
pub fn read_edn(source: &str) -> Result<Vec<Form>, ReadError> {
    data_reader(source).read_all()
}

fn data_reader(source: &str) -> Reader<'_> {
    let mut reader = Reader::new(source, false, Conditionals::Reject);
    reader.edn = true;
    reader
}

impl Reader<'_> {
    pub(super) fn edn_tag(&mut self, start: Position) -> Result<Kind, ReadError> {
        // Clojure 1.13's EdnReader dispatch table is indexed by one UTF-16
        // unit and has 256 entries. Once dispatched, the tag can contain Unicode.
        if self.peek().is_some_and(|ch| ch <= '\u{ff}') {
            return self.tagged(start);
        }
        let error = self.error(start, "Clojure EDN tags require an initial Latin-1 letter");
        if self.recover {
            let _ = self.tagged(start);
        }
        Err(error)
    }

    pub(super) fn edn_dispatch_prefix(&self) -> bool {
        matches!(
            self.source
                .as_bytes()
                .get(self.pos.offset..self.pos.offset.saturating_add(2)),
            Some([b'#', b'(' | b'"' | b'^' | b'\'' | b'='])
        )
    }

    pub(super) fn edn_dispatch(&mut self, start: Position) -> Result<Item, ReadError> {
        self.bump(); // #
        let kind = match self.peek() {
            Some('^') => {
                self.bump();
                self.token(super::SyntaxKind::MetadataMark, start);
                self.metadata_form(start)?
            }
            _ => {
                let error = self.error(start, "Clojure reader form is unavailable in EDN");
                self.token(super::SyntaxKind::DispatchMark, start);
                if self.recover && matches!(self.peek(), Some('\'' | '=')) {
                    let mark = self.pos;
                    self.bump();
                    self.token(super::SyntaxKind::ErrorToken, mark);
                }
                return Err(self.recover_prefix_operand(error));
            }
        };
        Ok(Item::Form(Form {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        }))
    }
}
