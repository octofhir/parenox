mod indexing;
mod maps;

pub use indexing::IndexedItem;
pub use maps::{ExtraFields, Field, MapSlot, MapType};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

pub const MAX_TUPLE_ITEMS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Arity {
    Fixed(usize),
    Variadic(usize),
}

impl Arity {
    pub fn new(count: usize, variadic: bool) -> Self {
        if variadic {
            Self::Variadic(count)
        } else {
            Self::Fixed(count)
        }
    }

    pub fn required(self) -> usize {
        match self {
            Self::Fixed(count) | Self::Variadic(count) => count,
        }
    }

    pub fn accepts(self, actual: usize) -> bool {
        match self {
            Self::Fixed(count) => actual == count,
            Self::Variadic(count) => actual >= count,
        }
    }

    pub fn bindings(self) -> usize {
        match self {
            Self::Fixed(count) => count,
            Self::Variadic(count) => count + 1,
        }
    }

    pub fn valid_set(arities: impl Iterator<Item = Self>) -> bool {
        let mut fixed = 0;
        let mut variadic = None;
        for arity in arities {
            match arity {
                Self::Fixed(count) => fixed = fixed.max(count),
                Self::Variadic(count) if variadic.is_none() => variadic = Some(count),
                Self::Variadic(_) => return false,
            }
        }
        variadic.is_none_or(|count| fixed <= count)
    }
}

impl fmt::Display for Arity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed(count) => write!(f, "{count}"),
            Self::Variadic(count) => write!(f, "at least {count}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Signature {
    pub args: Vec<Type>,
    pub rest: Option<Type>,
    pub ret: Type,
}

impl Signature {
    pub fn arity(&self) -> Arity {
        Arity::new(self.args.len(), self.rest.is_some())
    }

    pub fn types(&self) -> impl Iterator<Item = &Type> {
        self.args
            .iter()
            .chain(self.rest.as_ref())
            .chain([&self.ret])
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut args = self
            .args
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if let Some(rest) = &self.rest {
            args.push(format!("& {rest}"));
        }
        write!(f, "[{} -> {}]", args.join(", "), self.ret)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Type {
    Unknown,
    Never,
    Nil,
    Boolean,
    Bool(bool),
    Number,
    Integer(i32),
    String,
    Character,
    Char(char),
    Str(String),
    Keyword,
    Key(String),
    Map(MapType),
    MapEntry(Box<[Type; 2]>),
    Vector(Box<Type>),
    Tuple(Box<[Type]>),
    /// A nonempty seq with an unknown length.
    Seq(Box<Type>),
    /// A sequence of known length, including the empty list.
    SeqTuple(Box<[Type]>),
    Set(Box<Type>),
    Reduced(Box<Type>),
    Throwable,
    ExceptionInfo(Box<Type>),
    Function(Arc<[Signature]>),
    Union(Vec<Type>),
}

impl Type {
    pub fn empty_seq() -> Self {
        Self::SeqTuple(Box::new([]))
    }

    /// General expression type. Exact string values remain available separately as refinements.
    /// Keywords and booleans retain precision for Clojure lookup and truthiness semantics.
    pub fn inferred(&self) -> Type {
        match self {
            Type::Str(_) => Type::String,
            Type::Integer(_) => Type::Number,
            Type::Char(_) => Type::Character,
            Type::Union(xs) => Type::union(xs.iter().map(Type::inferred)),
            Type::MapEntry(items) => {
                let [key, value] = items.as_ref();
                Type::MapEntry(Box::new([key.inferred(), value.inferred()]))
            }
            Type::Vector(t) => Type::Vector(Box::new(t.inferred())),
            Type::Tuple(items) => {
                Type::Vector(Box::new(Type::union(items.iter().map(Type::inferred))))
            }
            Type::Seq(t) => Type::Seq(Box::new(t.inferred())),
            Type::SeqTuple(items) if !items.is_empty() => {
                Type::Seq(Box::new(Type::union(items.iter().map(Type::inferred))))
            }
            Type::Set(t) => Type::Set(Box::new(t.inferred())),
            Type::Reduced(t) => Type::Reduced(Box::new(t.inferred())),
            Type::ExceptionInfo(data) => Type::ExceptionInfo(Box::new(data.inferred())),
            Type::Map(map) => Type::Map(map.inferred()),
            t => t.clone(),
        }
    }

    pub fn requires_precise_type(&self) -> bool {
        match self {
            Type::Str(_) | Type::Char(_) | Type::Tuple(_) | Type::SeqTuple(_) => true,
            Type::Union(xs) => xs.iter().any(Type::requires_precise_type),
            Type::MapEntry(items) => items.iter().any(Type::requires_precise_type),
            Type::Vector(t)
            | Type::Set(t)
            | Type::Seq(t)
            | Type::Reduced(t)
            | Type::ExceptionInfo(t) => t.requires_precise_type(),
            Type::Map(map) => map.any_type(Type::requires_precise_type),
            Type::Function(signatures) => signatures
                .iter()
                .any(|sig| sig.types().any(Type::requires_precise_type)),
            _ => false,
        }
    }

    pub fn union(types: impl IntoIterator<Item = Type>) -> Type {
        let mut set = BTreeSet::new();
        for ty in types {
            match ty {
                Type::Never => {}
                Type::Union(xs) => set.extend(xs),
                t => {
                    set.insert(t);
                }
            }
        }
        if set.contains(&Type::Number) {
            set.retain(|t| !matches!(t, Type::Integer(_)));
        }
        if set.contains(&Type::String) {
            set.retain(|t| !matches!(t, Type::Str(_)));
        }
        if set.contains(&Type::Character) {
            set.retain(|t| !matches!(t, Type::Char(_)));
        }
        if set.contains(&Type::Keyword) {
            set.retain(|t| !matches!(t, Type::Key(_)));
        }
        if set.contains(&Type::Boolean)
            || (set.contains(&Type::Bool(true)) && set.contains(&Type::Bool(false)))
        {
            set.retain(|t| !matches!(t, Type::Bool(_)));
            set.insert(Type::Boolean);
        }
        match set.len() {
            0 => Type::Never,
            1 => set.into_iter().next().unwrap(),
            _ => Type::Union(set.into_iter().collect()),
        }
    }

    /// Conservative join of inferred values. Compact map alternatives into one
    /// shape with optional fields; contract unions continue to use `union`.
    pub fn join_values(self, other: Type) -> Type {
        match (self, other) {
            (Type::Map(left), Type::Map(right)) if left.is_shape() && right.is_shape() => {
                Type::Map(left.join_values(right))
            }
            (left, right) => match Type::union([left, right]) {
                Type::Union(parts)
                    if parts
                        .iter()
                        .filter(|t| matches!(t, Type::Map(map) if map.is_shape()))
                        .count()
                        > 1 =>
                {
                    let mut map: Option<MapType> = None;
                    let mut rest = Vec::with_capacity(parts.len());
                    for part in parts {
                        match part {
                            Type::Map(next) if next.is_shape() => {
                                map = Some(match map {
                                    Some(map) => map.join_values(next),
                                    None => next,
                                });
                            }
                            part => rest.push(part),
                        }
                    }
                    rest.extend(map.map(Type::Map));
                    Type::union(rest)
                }
                other => other,
            },
        }
    }

    pub fn nilable(self) -> Type {
        Type::union([self, Type::Nil])
    }

    pub fn without_nil(&self) -> Type {
        match self {
            Type::Nil => Type::Never,
            Type::Union(xs) => Type::union(xs.iter().map(Type::without_nil)),
            t => t.clone(),
        }
    }

    pub fn truthy(&self) -> Type {
        match self {
            Type::Nil | Type::Bool(false) => Type::Never,
            Type::Boolean => Type::Bool(true),
            Type::Union(xs) => Type::union(xs.iter().map(Type::truthy)),
            t => t.clone(),
        }
    }

    pub fn falsey(&self) -> Type {
        match self {
            Type::Nil => Type::Nil,
            Type::Boolean | Type::Bool(false) => Type::Bool(false),
            Type::Union(xs) => Type::union(xs.iter().map(Type::falsey)),
            Type::Unknown => Type::union([Type::Nil, Type::Bool(false)]),
            _ => Type::Never,
        }
    }

    pub fn entry_item(&self, index: usize) -> Option<Type> {
        match self {
            Type::MapEntry(items) => Some(items.get(index).cloned().unwrap_or(Type::Nil)),
            Type::Never | Type::Unknown => Some(self.clone()),
            Type::Union(parts) => parts
                .iter()
                .map(|part| part.entry_item(index))
                .collect::<Option<Vec<_>>>()
                .map(Type::union),
            _ => None,
        }
    }

    pub fn subtype_of(&self, expected: &Type) -> bool {
        if self == &Type::Never || self == expected {
            return true;
        }
        match (self, expected) {
            (Type::Union(xs), e) => xs.iter().all(|t| t.subtype_of(e)),
            (a, Type::Union(xs)) => xs.iter().any(|e| a.subtype_of(e)),
            (Type::Integer(_), Type::Number)
            | (Type::Str(_), Type::String)
            | (Type::Char(_), Type::Character)
            | (Type::Key(_), Type::Keyword)
            | (Type::ExceptionInfo(_), Type::Throwable)
            | (Type::Bool(_), Type::Boolean) => true,
            (Type::MapEntry(a), Type::MapEntry(e)) => {
                a.iter().zip(e.iter()).all(|(a, e)| a.subtype_of(e))
            }
            (Type::MapEntry(items), Type::Vector(e)) => items.iter().all(|item| item.subtype_of(e)),
            (Type::Tuple(items), Type::Vector(e)) => items.iter().all(|item| item.subtype_of(e)),
            (Type::SeqTuple(items), Type::Seq(e)) => {
                !items.is_empty() && items.iter().all(|item| item.subtype_of(e))
            }
            (Type::Tuple(a), Type::Tuple(e)) | (Type::SeqTuple(a), Type::SeqTuple(e)) => {
                a.len() == e.len() && a.iter().zip(e.iter()).all(|(a, e)| a.subtype_of(e))
            }
            (Type::MapEntry(a), Type::Tuple(e)) => {
                e.len() == 2 && a.iter().zip(e.iter()).all(|(a, e)| a.subtype_of(e))
            }
            (Type::Vector(a), Type::Vector(e))
            | (Type::Set(a), Type::Set(e))
            | (Type::Seq(a), Type::Seq(e))
            | (Type::Reduced(a), Type::Reduced(e))
            | (Type::ExceptionInfo(a), Type::ExceptionInfo(e)) => a.subtype_of(e),
            (Type::Function(actual), Type::Function(expected)) => expected.iter().all(|e| {
                actual.iter().any(|a| {
                    a.arity() == e.arity()
                        && a.args.iter().zip(&e.args).all(|(a, e)| e.subtype_of(a))
                        && a.rest
                            .as_ref()
                            .zip(e.rest.as_ref())
                            .is_none_or(|(a, e)| e.subtype_of(a))
                        && a.ret.subtype_of(&e.ret)
                })
            }),
            (Type::Map(a), Type::Map(e)) => a.subtype_of(e),
            _ => false,
        }
    }

    pub fn contains_unknown(&self) -> bool {
        match self {
            Type::Unknown => true,
            Type::Union(xs) => xs.iter().any(Type::contains_unknown),
            Type::MapEntry(items) => items.iter().any(Type::contains_unknown),
            Type::Tuple(items) | Type::SeqTuple(items) => items.iter().any(Type::contains_unknown),
            Type::Vector(t)
            | Type::Set(t)
            | Type::Seq(t)
            | Type::Reduced(t)
            | Type::ExceptionInfo(t) => t.contains_unknown(),
            Type::Map(m) => m.any_type(Type::contains_unknown),
            Type::Function(signatures) => signatures
                .iter()
                .any(|sig| sig.types().any(Type::contains_unknown)),
            _ => false,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unknown => write!(f, "Unknown"),
            Type::Never => write!(f, "Never"),
            Type::Nil => write!(f, "Nil"),
            Type::Boolean => write!(f, "Boolean"),
            Type::Bool(b) => write!(f, "{b}"),
            Type::Number => write!(f, "Number"),
            Type::Integer(index) => write!(f, "{index}"),
            Type::String => write!(f, "String"),
            Type::Character => write!(f, "Character"),
            Type::Char(c) => match c {
                '\n' => write!(f, "\\newline"),
                ' ' => write!(f, "\\space"),
                '\t' => write!(f, "\\tab"),
                '\r' => write!(f, "\\return"),
                '\u{0008}' => write!(f, "\\backspace"),
                '\u{000c}' => write!(f, "\\formfeed"),
                c if c.is_control() => write!(f, "\\u{:04X}", u32::from(*c)),
                c => write!(f, "\\{c}"),
            },
            Type::Str(s) => write!(f, "{s:?}"),
            Type::Keyword => write!(f, "Keyword"),
            Type::Key(s) => write!(f, "{s}"),
            Type::MapEntry(items) => write!(f, "MapEntry<{}, {}>", items[0], items[1]),
            Type::Vector(t) => write!(f, "Vector<{t}>"),
            Type::Seq(t) => write!(f, "Seq<{t}>"),
            Type::SeqTuple(items) if items.is_empty() => write!(f, "()"),
            Type::Tuple(items) | Type::SeqTuple(items) => {
                let name = if matches!(self, Type::SeqTuple(_)) {
                    "SeqTuple"
                } else {
                    "Tuple"
                };
                let items = items.iter().map(ToString::to_string).collect::<Vec<_>>();
                write!(f, "{name}<{}>", items.join(", "))
            }
            Type::Set(t) => write!(f, "Set<{t}>"),
            Type::Reduced(t) => write!(f, "Reduced<{t}>"),
            Type::Throwable => write!(f, "Throwable"),
            Type::ExceptionInfo(data) => write!(f, "ExceptionInfo<{data}>"),
            Type::Function(signatures) => {
                let cases = signatures
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                write!(f, "{}", cases.join(" or "))
            }
            Type::Union(xs) => {
                let xs = xs.iter().map(ToString::to_string).collect::<Vec<_>>();
                write!(f, "{}", xs.join(" | "))
            }
            Type::Map(map) => map.fmt(f),
        }
    }
}
