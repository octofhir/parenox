//! File URI and UTF-16 position conversion at the protocol boundary.
use std::fmt::Write;
use std::ops;
use std::path::{Component, Path, PathBuf};

use parenox_analysis::reader::Span;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Position {
    pub line: usize,
    pub character: usize,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEdit<'a> {
    range: Range,
    new_text: &'a str,
}

pub(crate) struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub(crate) fn new(source: &str) -> Self {
        let mut starts = vec![0];
        let mut bytes = source.bytes().enumerate().peekable();
        while let Some((i, byte)) = bytes.next() {
            match byte {
                b'\r' if bytes.peek().is_some_and(|&(_, next)| next == b'\n') => {}
                b'\r' | b'\n' => starts.push(i + 1),
                _ => {}
            }
        }
        Self { starts }
    }

    fn line_text<'a>(&self, source: &'a str, line: usize) -> Option<&'a str> {
        let start = *self.starts.get(line)?;
        let end = self.starts.get(line + 1).copied().unwrap_or(source.len());
        Some(source[start..end].trim_end_matches(['\r', '\n']))
    }

    fn position(&self, source: &str, offset: usize) -> Option<Position> {
        if !source.is_char_boundary(offset) {
            return None;
        }
        let line = self.starts.partition_point(|&start| start <= offset) - 1;
        let text = self.line_text(source, line)?;
        let byte = (offset - self.starts[line]).min(text.len());
        Some(Position {
            line,
            character: text[..byte].encode_utf16().count(),
        })
    }

    pub(crate) fn range(&self, source: &str, span: Span) -> Option<Range> {
        Some(Range {
            start: self.position(source, span.start.offset)?,
            end: self.position(source, span.end.offset)?,
        })
    }
}

pub struct Lines<'a> {
    source: &'a str,
    index: LineIndex,
}

impl<'a> Lines<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            index: LineIndex::new(source),
        }
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn offset(&self, position: Position) -> Option<usize> {
        let text = self.index.line_text(self.source, position.line)?;
        let start = self.index.starts[position.line];
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
        self.index.position(self.source, offset)
    }

    pub fn range(&self, span: Span) -> Option<Range> {
        self.index.range(self.source, span)
    }

    pub fn byte_range(&self, range: Range) -> Option<ops::Range<usize>> {
        let start = self.offset(range.start)?;
        let end = self.offset(range.end)?;
        (start <= end).then_some(start..end)
    }

    pub fn line_selection(&self, range: Range) -> Option<ops::Range<usize>> {
        let start = self.offset(range.start)?;
        let end = self.offset(range.end)?;
        if start > end {
            return None;
        }
        // The end is exclusive. An empty range formats just the cursor line.
        let ends_inside_line = end != self.index.starts[range.end.line];
        let end_line = range.end.line + usize::from(start == end || ends_inside_line);
        Some(
            self.index.starts[range.start.line]
                ..self
                    .index
                    .starts
                    .get(end_line)
                    .copied()
                    .unwrap_or(self.source.len()),
        )
    }

    pub fn formatting_edits<'b>(&self, formatted: &'b str) -> Vec<TextEdit<'b>> {
        let result = Lines::new(formatted);
        let starts = &self.index.starts;
        let new_starts = &result.index.starts;
        // Local edits keep untouched lines and their editor markers in place.
        // The current formatter preserves physical line breaks, except for
        // optionally adding a final newline.
        let extra_final_line = new_starts.len() == starts.len() + 1
            && new_starts.last() == Some(&formatted.len())
            && !self.source.ends_with(['\r', '\n']);
        if starts.len() != new_starts.len() && !extra_final_line {
            return self
                .formatting_edit(0..self.source.len(), formatted)
                .into_iter()
                .collect();
        }
        starts
            .iter()
            .enumerate()
            .filter_map(|(line, &start)| {
                let end = starts.get(line + 1).copied().unwrap_or(self.source.len());
                let new_start = new_starts[line];
                let new_end = new_starts.get(line + 1).copied().unwrap_or(formatted.len());
                let before = &self.source[start..end];
                let after = &formatted[new_start..new_end];
                if before == after {
                    return None;
                }
                let prefix = common_bytes(before.chars(), after.chars());
                let suffix = common_bytes(
                    before[prefix..].chars().rev(),
                    after[prefix..].chars().rev(),
                );
                self.formatting_edit(
                    start + prefix..end - suffix,
                    &after[prefix..after.len() - suffix],
                )
            })
            .collect()
    }

    fn formatting_edit<'b>(
        &self,
        bytes: ops::Range<usize>,
        new_text: &'b str,
    ) -> Option<TextEdit<'b>> {
        Some(TextEdit {
            range: Range {
                start: self.position(bytes.start)?,
                end: self.position(bytes.end)?,
            },
            new_text,
        })
    }
}

fn common_bytes(a: impl Iterator<Item = char>, b: impl Iterator<Item = char>) -> usize {
    a.zip(b)
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum()
}

pub fn normalize(path: &Path) -> PathBuf {
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

pub fn file_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let path = if path.starts_with('/') {
        path
    } else {
        path.strip_prefix("localhost")?
    };
    if !path.starts_with('/') || path.contains(['#', '?']) {
        return None;
    }
    let decoded = decode_uri_path(path)?;
    let path = decoded.as_str();
    #[cfg(windows)]
    let path = if path.as_bytes().get(2) == Some(&b':') {
        &path[1..]
    } else {
        path
    };
    let path = Path::new(path);
    path.is_absolute().then(|| normalize(path))
}

pub(crate) fn decode_uri_path(path: &str) -> Option<String> {
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
    let decoded = String::from_utf8(decoded).ok()?;
    (!decoded.contains('\0')).then_some(decoded)
}

pub fn file_uri(path: &str) -> String {
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
