use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Type {
    Unknown,
    Never,
    Nil,
    Boolean,
    Bool(bool),
    Number,
    String,
    Character,
    Char(char),
    Str(String),
    Keyword,
    Key(String),
    Map(MapType),
    MapEntry(Box<[Type; 2]>),
    Vector(Box<Type>),
    Set(Box<Type>),
    Function(Vec<Type>, Box<Type>),
    Union(Vec<Type>),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MapType {
    pub fields: BTreeMap<String, Field>,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Field {
    pub ty: Type,
    pub required: bool,
}

impl MapType {
    fn join_values(mut self, mut other: Self) -> Self {
        self.fields.retain(|key, field| {
            if let Some(right) = other.fields.remove(key) {
                let left = std::mem::replace(&mut field.ty, Type::Never);
                field.ty = left.join_values(right.ty);
                field.required &= right.required;
                true
            } else if other.open {
                // The other path can have an arbitrary undeclared value here.
                // Dropping the declaration keeps a later lookup an explicit gap.
                false
            } else {
                field.required = false;
                true
            }
        });
        if !self.open {
            for (key, mut field) in other.fields {
                field.required = false;
                self.fields.insert(key, field);
            }
        }
        self.open |= other.open;
        self
    }
}

impl Type {
    /// General expression type. Exact string values remain available separately as refinements.
    /// Keywords and booleans retain precision for Clojure lookup and truthiness semantics.
    pub fn inferred(&self) -> Type {
        match self {
            Type::Str(_) => Type::String,
            Type::Char(_) => Type::Character,
            Type::Union(xs) => Type::union(xs.iter().map(Type::inferred)),
            Type::MapEntry(items) => {
                let [key, value] = items.as_ref();
                Type::MapEntry(Box::new([key.inferred(), value.inferred()]))
            }
            Type::Vector(t) => Type::Vector(Box::new(t.inferred())),
            Type::Set(t) => Type::Set(Box::new(t.inferred())),
            Type::Map(map) => Type::Map(MapType {
                fields: map
                    .fields
                    .iter()
                    .map(|(key, field)| {
                        (
                            key.clone(),
                            Field {
                                ty: field.ty.inferred(),
                                required: field.required,
                            },
                        )
                    })
                    .collect(),
                open: map.open,
            }),
            t => t.clone(),
        }
    }

    pub fn constrains_literal_values(&self) -> bool {
        match self {
            Type::Str(_) | Type::Char(_) => true,
            Type::Union(xs) => xs.iter().any(Type::constrains_literal_values),
            Type::MapEntry(items) => items.iter().any(Type::constrains_literal_values),
            Type::Vector(t) | Type::Set(t) => t.constrains_literal_values(),
            Type::Map(map) => map
                .fields
                .values()
                .any(|f| f.ty.constrains_literal_values()),
            Type::Function(args, ret) => {
                args.iter().any(Type::constrains_literal_values) || ret.constrains_literal_values()
            }
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
            (Type::Map(left), Type::Map(right)) => Type::Map(left.join_values(right)),
            (left, right) => match Type::union([left, right]) {
                Type::Union(parts)
                    if parts.iter().filter(|t| matches!(t, Type::Map(_))).count() > 1 =>
                {
                    let mut map: Option<MapType> = None;
                    let mut rest = Vec::with_capacity(parts.len());
                    for part in parts {
                        if let Type::Map(next) = part {
                            map = Some(match map {
                                Some(map) => map.join_values(next),
                                None => next,
                            });
                        } else {
                            rest.push(part);
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

    /// Project an entry by position. Sequential binding also accepts nil.
    pub fn entry_item(&self, index: usize, allow_nil: bool) -> Option<Type> {
        match self {
            Type::MapEntry(items) => Some(items.get(index).cloned().unwrap_or(Type::Nil)),
            Type::Never | Type::Unknown => Some(self.clone()),
            Type::Nil if allow_nil => Some(Type::Nil),
            Type::Union(parts) => parts
                .iter()
                .map(|part| part.entry_item(index, allow_nil))
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
            (Type::Str(_), Type::String)
            | (Type::Char(_), Type::Character)
            | (Type::Key(_), Type::Keyword)
            | (Type::Bool(_), Type::Boolean) => true,
            (Type::MapEntry(a), Type::MapEntry(e)) => {
                a.iter().zip(e.iter()).all(|(a, e)| a.subtype_of(e))
            }
            (Type::MapEntry(items), Type::Vector(e)) => items.iter().all(|item| item.subtype_of(e)),
            (Type::Vector(a), Type::Vector(e)) | (Type::Set(a), Type::Set(e)) => a.subtype_of(e),
            (Type::Function(args, ret), Type::Function(e_args, e_ret)) => {
                args.len() == e_args.len()
                    && args.iter().zip(e_args).all(|(a, e)| e.subtype_of(a))
                    && ret.subtype_of(e_ret)
            }
            (Type::Map(a), Type::Map(e)) => {
                e.fields.iter().all(|(key, field)| match a.fields.get(key) {
                    Some(actual) => {
                        (!field.required || actual.required) && actual.ty.subtype_of(&field.ty)
                    }
                    None => !field.required && !a.open,
                }) && (e.open || (!a.open && a.fields.keys().all(|k| e.fields.contains_key(k))))
            }
            _ => false,
        }
    }

    pub fn contains_unknown(&self) -> bool {
        match self {
            Type::Unknown => true,
            Type::Union(xs) => xs.iter().any(Type::contains_unknown),
            Type::MapEntry(items) => items.iter().any(Type::contains_unknown),
            Type::Vector(t) | Type::Set(t) => t.contains_unknown(),
            Type::Map(m) => m.fields.values().any(|f| f.ty.contains_unknown()),
            Type::Function(args, ret) => {
                args.iter().any(Type::contains_unknown) || ret.contains_unknown()
            }
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
            Type::Set(t) => write!(f, "Set<{t}>"),
            Type::Function(args, ret) => {
                let args = args.iter().map(ToString::to_string).collect::<Vec<_>>();
                write!(f, "[{} -> {ret}]", args.join(", "))
            }
            Type::Union(xs) => {
                let xs = xs.iter().map(ToString::to_string).collect::<Vec<_>>();
                write!(f, "{}", xs.join(" | "))
            }
            Type::Map(m) => {
                let fields = m
                    .fields
                    .iter()
                    .map(|(k, v)| format!("{k}{}: {}", if v.required { "" } else { "?" }, v.ty))
                    .collect::<Vec<_>>();
                write!(
                    f,
                    "{{{}{}}}",
                    fields.join(", "),
                    if m.open { ", ..." } else { "" }
                )
            }
        }
    }
}
