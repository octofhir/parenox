//! Type representation, comparison and specialization for Parenox contracts.
mod compatibility;
mod indexing;
mod integers;
mod maps;
mod mismatch;
mod parameters;

pub use compatibility::{Compatibility, extern_drift};
pub use indexing::IndexedItem;
pub use integers::{IntegerClass, IntegerRange, IntegerType};
pub use maps::{ExtraFields, Field, MapSlot, MapType};
pub use mismatch::{ExternMismatch, ExternMismatchReason, ExternPathStep};
pub use parameters::{
    CallbackWitness, InputWitness, MAX_TYPE_PARAMETERS, Parameter, Parameters, TypeArguments,
};
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

pub const MAX_TUPLE_ITEMS: usize = 32;
pub const SPEC_INVALID: &str = ":clojure.spec.alpha/invalid";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Arity {
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
        self.required() + usize::from(matches!(self, Self::Variadic(_)))
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
pub struct Signature {
    pub args: Vec<Type>,
    pub rest: Option<Type>,
    pub ret: Type,
    pub parameters: Option<Arc<Parameters>>,
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
        if let Some(parameters) = &self.parameters {
            write!(f, "all [")?;
            for (index, parameter) in parameters.iter().enumerate() {
                if index > 0 {
                    write!(f, " ")?;
                }
                if let Some(upper) = &parameter.upper {
                    write!(f, "({} :< {upper})", parameter.name)?;
                } else {
                    write!(f, "{}", parameter.name)?;
                }
            }
            write!(f, "] ")?;
        }
        write!(f, "[{} -> {}]", args.join(", "), self.ret)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Type {
    Unknown,
    /// A value other than the reserved spec-invalid keyword.
    SpecValue,
    /// A truthy value that can be returned by a spec predicate.
    SpecTruthy,
    /// A Clojure persistent collection constrained by its immediate members.
    /// A map's immediate members are its entries, not the keys and values.
    SpecCollection(Box<Type>),
    Parameter(Arc<Parameter>),
    Never,
    Nil,
    Boolean,
    Bool(bool),
    Number,
    /// Integral values with independent JVM class and numeric range constraints.
    Integral(IntegerType),
    /// An exact fixed-precision integer in the range used by indexed reads.
    Integer(i32),
    /// Numeric values for which Clojure's integer? predicate is false.
    NonIntegral,
    String,
    Pattern,
    Character,
    Char(char),
    Str(String),
    Keyword,
    /// A keyword other than the reserved conformance failure marker.
    SpecKeyword,
    Key(String),
    Symbol,
    SimpleSymbol,
    QualifiedSymbol,
    Map(MapType),
    MapEntry(Box<[Type; 2]>),
    Vector(Box<Type>),
    Tuple(Box<[Type]>),
    /// A nonempty seq with an unknown length.
    Seq(Box<Type>),
    /// A sequence of known length, including the empty list.
    SeqTuple(Box<[Type]>),
    /// A nonempty persistent list with an unknown length.
    List(Box<Type>),
    /// A persistent list of known length.
    ListTuple(Box<[Type]>),
    Set(Box<Type>),
    Reduced(Box<Type>),
    Throwable,
    ExceptionInfo(Box<Type>),
    Function(Arc<[Signature]>),
    Union(Vec<Type>),
}

impl Type {
    /// Whether this domain consists only of maps or strings, excluding map entries.
    pub fn excludes_map_entry(&self) -> bool {
        match self {
            Type::Map(_) | Type::String | Type::Str(_) => true,
            Type::Union(parts) => parts.iter().all(Type::excludes_map_entry),
            _ => false,
        }
    }

    pub fn empty_seq() -> Self {
        Self::SeqTuple(Box::new([]))
    }

    pub fn empty_list() -> Self {
        Self::ListTuple(Box::new([]))
    }

    /// General expression type. Exact string values remain available separately as refinements.
    /// Keywords and booleans retain precision for Clojure lookup and truthiness semantics.
    pub fn inferred(&self) -> Type {
        match self {
            Type::Str(_) => Type::String,
            Type::Integer(_)
            | Type::Integral(IntegerType {
                range:
                    IntegerRange::ExactLong(_) | IntegerRange::OutsideInt32 | IntegerRange::OutsideInt64,
                ..
            })
            | Type::NonIntegral => Type::Number,
            Type::Integral(integer) => Type::Integral(integer.without_value()),
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
            Type::List(t) => Type::List(Box::new(t.inferred())),
            Type::SeqTuple(items) if !items.is_empty() => {
                Type::Seq(Box::new(Type::union(items.iter().map(Type::inferred))))
            }
            Type::ListTuple(items) if !items.is_empty() => {
                Type::List(Box::new(Type::union(items.iter().map(Type::inferred))))
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
            Type::Str(_)
            | Type::Char(_)
            | Type::Integral(_)
            | Type::Integer(_)
            | Type::Tuple(_)
            | Type::SeqTuple(_)
            | Type::ListTuple(_) => true,
            Type::Union(xs) => xs.iter().any(Type::requires_precise_type),
            Type::MapEntry(items) => items.iter().any(Type::requires_precise_type),
            Type::SpecCollection(t)
            | Type::Vector(t)
            | Type::Set(t)
            | Type::Seq(t)
            | Type::List(t)
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
        if set.len() <= 1 {
            return set.pop_first().unwrap_or(Type::Never);
        }
        if set.contains(&Type::Number) {
            set.retain(|t| !matches!(t, Type::Integral(_) | Type::Integer(_) | Type::NonIntegral));
        } else {
            for cover in [
                IntegerType::ANY,
                IntegerType::FIXED,
                IntegerType::INT32,
                IntegerType::FIXED_INT32,
            ] {
                let cover = Type::Integral(cover);
                if set.contains(&cover) {
                    set.retain(|t| {
                        *t == cover || t.integer_type().is_none() || !t.subtype_of(&cover)
                    });
                }
            }
        }
        if set.contains(&Type::String) {
            set.retain(|t| !matches!(t, Type::Str(_)));
        }
        if set.contains(&Type::Character) {
            set.retain(|t| !matches!(t, Type::Char(_)));
        }
        if set.contains(&Type::Keyword) {
            set.retain(|t| !matches!(t, Type::SpecKeyword | Type::Key(_)));
        } else if set.contains(&Type::SpecKeyword) {
            set.retain(|t| !matches!(t, Type::Key(key) if key != SPEC_INVALID));
        }
        if set.contains(&Type::Symbol)
            || (set.contains(&Type::SimpleSymbol) && set.contains(&Type::QualifiedSymbol))
        {
            set.remove(&Type::SimpleSymbol);
            set.remove(&Type::QualifiedSymbol);
            set.insert(Type::Symbol);
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
            Type::SpecValue => Type::union([Type::Bool(false), Type::SpecTruthy]),
            Type::Union(xs) => Type::union(xs.iter().map(Type::without_nil)),
            t => t.clone(),
        }
    }

    pub fn truthy(&self) -> Type {
        match self {
            Type::Nil | Type::Bool(false) => Type::Never,
            Type::SpecValue => Type::SpecTruthy,
            Type::Boolean => Type::Bool(true),
            Type::Union(xs) => Type::union(xs.iter().map(Type::truthy)),
            t => t.clone(),
        }
    }

    pub fn spec_truthy(&self) -> Option<Type> {
        self.spec_value().map(|ty| ty.truthy())
    }

    pub fn spec_value(&self) -> Option<Type> {
        match self {
            Type::Unknown => Some(Type::SpecValue),
            Type::Keyword => Some(Type::SpecKeyword),
            Type::Parameter(_) => None,
            Type::Key(key) if key == SPEC_INVALID => Some(Type::Never),
            Type::Union(members) => members
                .iter()
                .map(Type::spec_value)
                .collect::<Option<Vec<_>>>()
                .map(Type::union),
            ty => Some(ty.clone()),
        }
    }

    pub fn falsey(&self) -> Type {
        match self {
            Type::Nil => Type::Nil,
            Type::Boolean | Type::Bool(false) => Type::Bool(false),
            Type::Union(xs) => Type::union(xs.iter().map(Type::falsey)),
            Type::Unknown | Type::SpecValue => Type::union([Type::Nil, Type::Bool(false)]),
            // Without a bound we cannot exclude either branch. Keep the
            // identity of the value when checking a polymorphic body.
            Type::Parameter(parameter)
                if parameter
                    .upper
                    .as_ref()
                    .is_none_or(|bound| bound.falsey() != Type::Never) =>
            {
                self.clone()
            }
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

    pub fn contains_unknown(&self) -> bool {
        self.any_component(|ty| matches!(ty, Type::Unknown))
    }

    fn any_component(&self, predicate: impl Fn(&Type) -> bool + Copy) -> bool {
        if predicate(self) {
            return true;
        }
        match self {
            Type::Union(xs) => xs.iter().any(|ty| ty.any_component(predicate)),
            Type::MapEntry(items) => items.iter().any(|ty| ty.any_component(predicate)),
            Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => {
                items.iter().any(|ty| ty.any_component(predicate))
            }
            Type::SpecCollection(t)
            | Type::Vector(t)
            | Type::Set(t)
            | Type::Seq(t)
            | Type::List(t)
            | Type::Reduced(t)
            | Type::ExceptionInfo(t) => t.any_component(predicate),
            Type::Map(m) => m.any_type(|ty| ty.any_component(predicate)),
            Type::Function(signatures) => signatures
                .iter()
                .any(|sig| sig.types().any(|ty| ty.any_component(predicate))),
            _ => false,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unknown => write!(f, "Unknown"),
            Type::SpecValue => write!(f, "Spec value"),
            Type::SpecTruthy => write!(f, "Truthy spec value"),
            Type::SpecCollection(element) if **element == Type::SpecValue => {
                write!(f, "Collection of spec values")
            }
            Type::SpecCollection(element) => write!(f, "Collection<{element}>"),
            Type::Parameter(parameter) => write!(f, "{}", parameter.name),
            Type::Never => write!(f, "Never"),
            Type::Nil => write!(f, "Nil"),
            Type::Boolean => write!(f, "Boolean"),
            Type::Bool(b) => write!(f, "{b}"),
            Type::Number => write!(f, "Number"),
            Type::Integral(ty) => write!(f, "{ty}"),
            Type::Integer(index) => write!(f, "{index}"),
            Type::NonIntegral => write!(f, "Number that is not a Clojure integer"),
            Type::String => write!(f, "String"),
            Type::Pattern => write!(f, "Pattern"),
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
            Type::SpecKeyword => write!(f, "Keyword excluding {SPEC_INVALID}"),
            Type::Key(s) => write!(f, "{s}"),
            Type::Symbol => write!(f, "Symbol"),
            Type::SimpleSymbol => write!(f, "SimpleSymbol"),
            Type::QualifiedSymbol => write!(f, "QualifiedSymbol"),
            Type::MapEntry(items) => write!(f, "MapEntry<{}, {}>", items[0], items[1]),
            Type::Vector(t) => write!(f, "Vector<{t}>"),
            Type::Seq(t) => write!(f, "Seq<{t}>"),
            Type::List(t) => write!(f, "List<{t}>"),
            Type::ListTuple(items) if items.is_empty() => write!(f, "EmptyList"),
            Type::SeqTuple(items) if items.is_empty() => write!(f, "()"),
            Type::Tuple(items) | Type::SeqTuple(items) | Type::ListTuple(items) => {
                let name = match self {
                    Type::ListTuple(_) => "ListTuple",
                    Type::SeqTuple(_) => "SeqTuple",
                    _ => "Tuple",
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
