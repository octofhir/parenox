//! The bounded bencode subset used by the nREPL transport.
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::str;

pub(super) const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_VALUES: usize = 65_536;
const MAX_DEPTH: usize = 32;

pub(super) type Dict = BTreeMap<String, Value>;

pub(super) enum Value {
    Bytes(Vec<u8>),
    Integer,
    List(Vec<Value>),
    Dict(Dict),
}

impl Value {
    pub(super) fn text(&self) -> io::Result<&str> {
        match self {
            Self::Bytes(bytes) => str::from_utf8(bytes).map_err(invalid),
            _ => Err(invalid("expected bencode text")),
        }
    }

    pub(super) fn dict(&self) -> io::Result<&Dict> {
        match self {
            Self::Dict(dict) => Ok(dict),
            _ => Err(invalid("expected a bencode dictionary")),
        }
    }

    pub(super) fn list(&self) -> io::Result<&[Value]> {
        match self {
            Self::List(list) => Ok(list),
            _ => Err(invalid("expected a bencode list")),
        }
    }
}

pub(super) fn encode(fields: &BTreeMap<&str, &str>) -> Vec<u8> {
    let mut bytes = vec![b'd'];
    for (key, value) in fields {
        for text in [key, value] {
            bytes.extend_from_slice(text.len().to_string().as_bytes());
            bytes.push(b':');
            bytes.extend_from_slice(text.as_bytes());
        }
    }
    bytes.push(b'e');
    bytes
}

pub(super) struct Decoder<R> {
    input: R,
    bytes_left: usize,
    values_left: usize,
}

impl<R: Read> Decoder<R> {
    pub(super) fn new(input: R) -> Self {
        Self {
            input,
            bytes_left: MAX_BYTES,
            values_left: MAX_VALUES,
        }
    }

    pub(super) fn message(&mut self) -> io::Result<Dict> {
        if self.byte()? != b'd' {
            return Err(invalid("expected an nREPL message dictionary"));
        }
        self.dictionary(0)
    }

    fn value(&mut self, first: u8, depth: usize) -> io::Result<Value> {
        if depth >= MAX_DEPTH {
            return Err(invalid("bencode nesting limit exceeded"));
        }
        self.values_left = self
            .values_left
            .checked_sub(1)
            .ok_or_else(|| invalid("bencode value limit exceeded"))?;
        match first {
            b'd' => self.dictionary(depth).map(Value::Dict),
            b'l' => {
                let mut values = Vec::new();
                loop {
                    let first = self.byte()?;
                    if first == b'e' {
                        return Ok(Value::List(values));
                    }
                    values.push(self.value(first, depth + 1)?);
                }
            }
            b'i' => {
                let mut digits = [0; 20]; // Enough for an i64, including the sign.
                let mut len = 0;
                loop {
                    let byte = self.byte()?;
                    if byte == b'e' {
                        break;
                    }
                    let slot = digits
                        .get_mut(len)
                        .ok_or_else(|| invalid("bencode integer too long"))?;
                    *slot = byte;
                    len += 1;
                }
                let raw = str::from_utf8(&digits[..len]).map_err(invalid)?;
                raw.parse::<i64>().map_err(invalid)?;
                Ok(Value::Integer)
            }
            b'0'..=b'9' => self.bytes(first).map(Value::Bytes),
            _ => Err(invalid("invalid bencode value")),
        }
    }

    fn dictionary(&mut self, depth: usize) -> io::Result<Dict> {
        let mut dict = Dict::new();
        loop {
            let first = self.byte()?;
            if first == b'e' {
                return Ok(dict);
            }
            let key = String::from_utf8(self.bytes(first)?).map_err(invalid)?;
            let first = self.byte()?;
            let value = self.value(first, depth + 1)?;
            if dict.insert(key, value).is_some() {
                return Err(invalid("duplicate bencode dictionary key"));
            }
        }
    }

    fn bytes(&mut self, first: u8) -> io::Result<Vec<u8>> {
        let mut len = digit(first)?;
        loop {
            let byte = self.byte()?;
            if byte == b':' {
                break;
            }
            let next = digit(byte)?;
            len = len
                .checked_mul(10)
                .and_then(|len| len.checked_add(next))
                .filter(|len| *len <= self.bytes_left)
                .ok_or_else(|| invalid("nREPL response exceeds byte limit"))?;
        }
        self.take(len)?;
        let mut bytes = vec![0; len];
        self.input.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn byte(&mut self) -> io::Result<u8> {
        self.take(1)?;
        let mut byte = [0];
        self.input.read_exact(&mut byte)?;
        Ok(byte[0])
    }

    fn take(&mut self, len: usize) -> io::Result<()> {
        self.bytes_left = self
            .bytes_left
            .checked_sub(len)
            .ok_or_else(|| invalid("nREPL response exceeds byte limit"))?;
        Ok(())
    }
}

fn digit(byte: u8) -> io::Result<usize> {
    if !byte.is_ascii_digit() {
        return Err(invalid("invalid bencode string length"));
    }
    Ok(usize::from(byte - b'0'))
}

pub(super) fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
