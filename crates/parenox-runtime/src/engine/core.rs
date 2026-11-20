//! Core functions supported by the Parenox evaluator.
use std::io::Write;

use super::Value;

#[derive(Clone, Copy)]
pub(super) enum Builtin {
    Add,
    Inc,
    Str,
    Keyword,
    Println,
    Prn,
}

impl Builtin {
    pub(super) fn named(name: &str) -> Option<Self> {
        Some(match name {
            "+" => Self::Add,
            "inc" => Self::Inc,
            "str" => Self::Str,
            "keyword" => Self::Keyword,
            "println" => Self::Println,
            "prn" => Self::Prn,
            _ => return None,
        })
    }

    pub(super) fn check_inline_arity(self, count: usize) -> Result<(), String> {
        if matches!(self, Self::Inc) && count != 1 {
            return Err(arity("inc", count));
        }
        Ok(())
    }

    pub(super) fn apply(self, args: &[Value], output: &mut dyn Write) -> Result<Value, String> {
        match self {
            Self::Add => args
                .iter()
                .try_fold(0i64, |total, arg| {
                    total.checked_add(integer(arg)?).ok_or_else(overflow)
                })
                .map(Value::Integer),
            Self::Inc => match args {
                [value] => integer(value)?
                    .checked_add(1)
                    .map(Value::Integer)
                    .ok_or_else(overflow),
                _ => Err(arity("inc", args.len())),
            },
            Self::Str => {
                let mut text = String::new();
                for value in args {
                    text.push_str(&value.text());
                }
                Ok(Value::String(text.into()))
            }
            Self::Keyword => match args {
                [value @ Value::Keyword { .. }] => Ok(value.clone()),
                [Value::String(name) | Value::Symbol(name)] => Ok(Value::keyword(name)),
                [_] => Ok(Value::Nil),
                [Value::Nil, Value::String(name)] => Ok(Value::keyword_parts(None, name)),
                [Value::String(namespace), Value::String(name)] => {
                    Ok(Value::keyword_parts(Some(namespace), name))
                }
                [_, _] => Err("keyword requires a string namespace and name".into()),
                _ => Err(arity("keyword", args.len())),
            },
            Self::Println | Self::Prn => {
                print_line(output, args, matches!(self, Self::Prn))
                    .map_err(|error| format!("write eval output: {error}"))?;
                Ok(Value::Nil)
            }
        }
    }
}

fn integer(value: &Value) -> Result<i64, String> {
    if let Value::Integer(value) = value {
        Ok(*value)
    } else {
        Err(format!("Expected Long, got {value}"))
    }
}

fn overflow() -> String {
    "long overflow".into()
}

fn arity(name: &str, count: usize) -> String {
    format!("Wrong number of args ({count}) passed to: {name}")
}

fn print_line(output: &mut dyn Write, args: &[Value], readable: bool) -> std::io::Result<()> {
    for (index, arg) in args.iter().enumerate() {
        if index != 0 {
            output.write_all(b" ")?;
        }
        write!(output, "{}", arg.display(readable))?;
    }
    writeln!(output)
}
