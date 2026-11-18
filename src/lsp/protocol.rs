//! Content-Length framing for the stdio transport.
use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

pub(super) fn read_message(input: &mut impl BufRead, buffer: &mut Vec<u8>) -> io::Result<bool> {
    let mut header = Vec::new();
    let mut length = None;
    let mut remaining = MAX_HEADER_BYTES;
    loop {
        header.clear();
        let read =
            Read::take(&mut *input, (remaining + 1) as u64).read_until(b'\n', &mut header)?;
        if read == 0 && remaining == MAX_HEADER_BYTES {
            return Ok(false);
        }
        if read > remaining || !header.ends_with(b"\r\n") {
            return Err(invalid("Invalid LSP message headers"));
        }
        remaining -= read;
        if header == b"\r\n" {
            break;
        }
        let line = std::str::from_utf8(&header[..read - 2])
            .map_err(|_| invalid("Invalid LSP header encoding"))?;
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("Invalid LSP header"))?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err(invalid("Duplicate Content-Length"));
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|&n| n <= MAX_MESSAGE_BYTES)
                    .ok_or_else(|| invalid("Invalid LSP message length"))?,
            );
        }
    }
    buffer.resize(length.ok_or_else(|| invalid("Missing Content-Length"))?, 0);
    input.read_exact(buffer)?;
    Ok(true)
}

pub(super) fn write_message(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
