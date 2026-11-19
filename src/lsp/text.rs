//! File URI and UTF-16 position conversion at the protocol boundary.
use std::fmt::Write;
use std::ops;
use std::path::{Component, Path, PathBuf};

use parenox::reader::Span;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(super) struct Position {
    line: usize,
    character: usize,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Range {
    start: Position,
    end: Position,
}

pub(super) struct Lines<'a> {
    source: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut starts = vec![0];
        let mut bytes = source.bytes().enumerate().peekable();
        while let Some((i, byte)) = bytes.next() {
            match byte {
                b'\r' if bytes.peek().is_some_and(|&(_, next)| next == b'\n') => {}
                b'\r' | b'\n' => starts.push(i + 1),
                _ => {}
            }
        }
        Self { source, starts }
    }

    fn line_text(&self, line: usize) -> Option<&str> {
        let start = *self.starts.get(line)?;
        let end = self
            .starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.source.len());
        Some(self.source[start..end].trim_end_matches(['\r', '\n']))
    }

    pub fn offset(&self, position: Position) -> Option<usize> {
        let text = self.line_text(position.line)?;
        let start = self.starts[position.line];
        let mut character = 0;
        for (byte, ch) in text.char_indices() {
            if character == position.character {
                return Some(start + byte);
            }
            character += ch.len_utf16();
            if character > position.character {
                return None;
            }
        }
        Some(start + text.len())
    }

    pub fn position(&self, offset: usize) -> Option<Position> {
        if !self.source.is_char_boundary(offset) {
            return None;
        }
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        let text = self.line_text(line)?;
        let byte = (offset - self.starts[line]).min(text.len());
        Some(Position {
            line,
            character: text[..byte].encode_utf16().count(),
        })
    }

    pub fn range(&self, span: Span) -> Option<Range> {
        Some(Range {
            start: self.position(span.start.offset)?,
            end: self.position(span.end.offset)?,
        })
    }

    pub fn byte_range(&self, range: Range) -> Option<ops::Range<usize>> {
        let start = self.offset(range.start)?;
        let end = self.offset(range.end)?;
        (start <= end).then_some(start..end)
    }
}

pub(super) fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

pub(super) fn file_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let path = if path.starts_with('/') {
        path
    } else {
        path.strip_prefix("localhost")?
    };
    if !path.starts_with('/') || path.contains(['#', '?']) {
        return None;
    }
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    let path = std::str::from_utf8(&decoded).ok()?;
    if path.contains('\0') {
        return None;
    }
    #[cfg(windows)]
    let path = if path.as_bytes().get(2) == Some(&b':') {
        &path[1..]
    } else {
        path
    };
    let path = Path::new(path);
    path.is_absolute().then(|| normalize(path))
}

pub(super) fn file_uri(path: &str) -> String {
    #[cfg(windows)]
    let normalized = path.replace('\\', "/");
    #[cfg(windows)]
    let path = normalized.as_str();
    let mut uri = String::from("file://");
    if !path.starts_with('/') {
        uri.push('/');
    }
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            uri.push(byte.into());
        } else {
            write!(uri, "%{byte:02X}").unwrap();
        }
    }
    uri
}
