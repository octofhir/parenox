//! Serialize resolved public contracts without source names or spec dependencies.
use std::fmt::{self, Write};

use super::{Contract, ExternalDeclaration, Model, reserves_extern_name};
use crate::checker::model::GuardMode;
use crate::reader::{MAX_NESTING_DEPTH, Span};
use crate::types::{
    ExtraFields, IntegerClass, IntegerRange, IntegerType, JvmArrayClass, LengthBounds, MapType,
    Signature, Type,
};

impl Model {
    pub(crate) fn extern_count(&self) -> usize {
        self.externs
            .iter()
            .flat_map(|file| file.table.namespaces.values())
            .map(|ns| ns.len())
            .sum()
    }

    pub(crate) fn matches_export(&self, parsed: &Model, name: &str, literal: Option<&str>) -> bool {
        let Some((namespace, _)) = name.split_once('/') else {
            return false;
        };
        let declaration = parsed
            .externs
            .as_ref()
            .and_then(|file| file.table.namespaces.get(namespace))
            .and_then(|namespace| namespace.get(name));
        match (declaration, literal) {
            (Some(ExternalDeclaration::Function(parsed)), None) => {
                self.exports.functions.get(name).is_some_and(|source| {
                    source.signatures == parsed.signatures && source.guard == parsed.guard
                })
            }
            (Some(ExternalDeclaration::Literal(parsed)), Some(edn)) => {
                parsed.edn == edn
                    && self
                        .exports
                        .literals
                        .get(name)
                        .is_some_and(|source| source.ty == parsed.ty)
            }
            _ => false,
        }
    }

    pub(crate) fn exported_functions(
        &self,
    ) -> impl Iterator<Item = (&str, Span, Result<&Contract, &'static str>)> {
        self.exports.functions.iter().map(|(name, contract)| {
            let signature = if reserves_extern_name(name) {
                Err("externs cannot replace a built-in checking rule")
            } else {
                Ok(contract)
            };
            (name.as_str(), self.functions[name].span, signature)
        })
    }
}

// A resolved spec can share callback signatures. Inline EDN has to print every
// occurrence, so charge bytes at the point of emission, including repeated types.
const MAX_SNAPSHOT_BYTES: usize = 1024 * 1024;

pub(crate) struct SnapshotWriter {
    bytes: Vec<u8>,
    depth: usize,
}

pub(crate) enum EncodeError {
    Size,
    Depth,
    Unsupported(&'static str),
}

type Encoded = Result<(), EncodeError>;

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size => write!(
                f,
                "snapshot size limit ({MAX_SNAPSHOT_BYTES} bytes) exceeded"
            ),
            Self::Depth => write!(
                f,
                "expanded signature exceeds the reader nesting limit ({MAX_NESTING_DEPTH})"
            ),
            Self::Unsupported(reason) => f.write_str(reason),
        }
    }
}

impl From<fmt::Error> for EncodeError {
    fn from(_: fmt::Error) -> Self {
        Self::Size
    }
}

impl fmt::Write for SnapshotWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.append(text.as_bytes()).map_err(|_| fmt::Error)
    }
}

impl std::io::Write for SnapshotWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.append(bytes)
            .map_err(|_| std::io::Error::other("snapshot size limit exceeded"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SnapshotWriter {
    pub(crate) fn new() -> Self {
        Self {
            bytes: vec![b'{'],
            depth: 1,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn finish(mut self) -> Result<String, EncodeError> {
        self.write_str("}\n")?;
        Ok(String::from_utf8(self.bytes).expect("snapshot is UTF-8"))
    }

    fn append(&mut self, bytes: &[u8]) -> Encoded {
        if bytes.len() > MAX_SNAPSHOT_BYTES - self.bytes.len() {
            return Err(EncodeError::Size);
        }
        let needed = self.bytes.len() + bytes.len();
        if needed > self.bytes.capacity() {
            let capacity = (2 * self.bytes.capacity())
                .max(needed)
                .min(MAX_SNAPSHOT_BYTES);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    // The prefix opens nonempty forms. Account for their deepest child before
    // visiting it. Atom writes (including empty collections) fit at depth + 1.
    pub(super) fn nested(
        &mut self,
        levels: usize,
        prefix: &str,
        suffix: &str,
        encode: impl FnOnce(&mut Self) -> Encoded,
    ) -> Encoded {
        let depth = self.depth + levels;
        if depth >= MAX_NESTING_DEPTH {
            return Err(EncodeError::Depth);
        }
        self.write_str(prefix)?;
        self.depth = depth;
        encode(self)?;
        self.depth -= levels;
        Ok(self.write_str(suffix)?)
    }

    pub(crate) fn function(&mut self, contract: &Contract) -> Encoded {
        let Some(guard) = &contract.guard else {
            return self.signatures(&contract.signatures);
        };
        let mode = match guard.mode {
            GuardMode::Then => ":then",
            GuardMode::Both => ":both",
            GuardMode::Some => ":some",
        };
        self.nested(1, "{:sig ", "}", |out| {
            out.signatures(&contract.signatures)?;
            out.nested(1, " :guard [", "]", |out| {
                write!(out, "{mode} {} ", guard.argument + 1)?;
                out.ty(&guard.ty)
            })
        })
    }

    fn signatures(&mut self, signatures: &[Signature]) -> Encoded {
        match signatures {
            [one] => self.signature(one),
            many => self.nested(1, "[", "]", |out| {
                for (index, signature) in many.iter().enumerate() {
                    out.separator(index)?;
                    out.signature(signature)?;
                }
                Ok(())
            }),
        }
    }

    fn signature(&mut self, signature: &Signature) -> Encoded {
        if signature.nil_preserving {
            return self.nested(2, "(t/nil-preserving [", "])", |out| {
                out.ty(&signature.args[0].without_nil())?;
                out.write_str(" :-> ")?;
                out.ty(&signature.ret.without_nil())
            });
        }
        let Some(parameters) = &signature.parameters else {
            return self.signature_vector(signature);
        };
        self.nested(1, "(t/all ", ")", |out| {
            out.nested(1, "[", "]", |out| {
                for (index, parameter) in parameters.iter().enumerate() {
                    out.separator(index)?;
                    if let Some(upper) = &parameter.upper {
                        out.nested(1, "(", ")", |out| {
                            write!(out, "{} :< ", parameter.name)?;
                            out.ty(upper)
                        })?;
                    } else {
                        out.write_str(&parameter.name)?;
                    }
                }
                Ok(())
            })?;
            out.write_char(' ')?;
            out.signature_vector(signature)
        })
    }

    fn signature_vector(&mut self, signature: &Signature) -> Encoded {
        self.nested(1, "[", "]", |out| {
            for arg in &signature.args {
                out.ty(arg)?;
                out.write_char(' ')?;
            }
            if let Some(rest) = &signature.rest {
                out.write_str("& ")?;
                out.ty(rest)?;
                out.write_char(' ')?;
            }
            out.write_str(":-> ")?;
            out.ty(&signature.ret)
        })
    }

    fn callback(&mut self, signature: &Signature) -> Encoded {
        self.nested(1, "(clojure.spec.alpha/fspec :args ", ")", |out| {
            out.nested(1, "(clojure.spec.alpha/cat", ")", |out| {
                for (index, ty) in signature.args.iter().enumerate() {
                    write!(out, " :p{index} ")?;
                    out.ty(ty)?;
                }
                if let Some(rest) = &signature.rest {
                    out.nested(1, " :rest (clojure.spec.alpha/* ", ")", |out| out.ty(rest))?;
                }
                Ok(())
            })?;
            out.write_str(" :ret ")?;
            out.ty(&signature.ret)
        })
    }

    pub(crate) fn ty(&mut self, ty: &Type) -> Encoded {
        if let Some((predicate, value)) = exact_integer(ty) {
            return self.integer_set(predicate, std::iter::once(value));
        }
        let atom = match ty {
            Type::Never => "#{}",
            Type::Any => "t/any",
            Type::Nil => "clojure.core/nil?",
            Type::Boolean => "clojure.core/boolean?",
            Type::SpecValue => "clojure.core/any?",
            Type::SpecTruthy => "clojure.core/identity",
            Type::SpecCollection(element) => {
                return self.nested(1, "(clojure.spec.alpha/coll-of ", ")", |out| {
                    out.ty(element)
                });
            }
            Type::BoundedCollection(collection, bounds) => {
                return self.bounded_collection(collection, *bounds);
            }
            Type::Number => "clojure.core/number?",
            Type::Integral(IntegerType::ANY) => "clojure.core/integer?",
            Type::Integral(IntegerType::FIXED) => "clojure.core/int?",
            Type::Integral(IntegerType::INT32) => {
                return self.nested(2, "(clojure.spec.alpha/and clojure.core/integer? (clojure.spec.alpha/conformer clojure.core/int", "))", |_| Ok(()));
            }
            Type::Integral(IntegerType::FIXED_INT32) => {
                return self.nested(3, "(clojure.spec.alpha/and clojure.core/int? (clojure.spec.alpha/and clojure.core/integer? (clojure.spec.alpha/conformer clojure.core/int", ")))", |_| Ok(()));
            }
            Type::String => "clojure.core/string?",
            Type::Pattern => "java.util.regex.Pattern",
            Type::JvmIterator => "java.util.Iterator",
            Type::JvmArray(JvmArrayClass::Byte) => "clojure.core/bytes?",
            Type::Character => "clojure.core/char?",
            Type::Keyword => "clojure.core/keyword?",
            Type::SpecKeyword => {
                return self.nested(
                    1,
                    "(clojure.spec.alpha/and clojure.core/keyword? clojure.core/any?",
                    ")",
                    |_| Ok(()),
                );
            }
            Type::Symbol => "clojure.core/symbol?",
            Type::SimpleSymbol => "clojure.core/simple-symbol?",
            Type::QualifiedSymbol => "clojure.core/qualified-symbol?",
            Type::Bool(value) => {
                return self.nested(1, "#{", "}", |out| Ok(write!(out, "{value}")?));
            }
            Type::Char(value) if u32::from(*value) <= 0xffff => {
                return self.nested(1, "#{", "}", |out| {
                    Ok(write!(out, "\\u{:04x}", u32::from(*value))?)
                });
            }
            Type::Key(_) | Type::Str(_) => {
                return self.nested(1, "#{", "}", |out| out.key(ty));
            }
            Type::Sym(symbol) => {
                return self.nested(2, "#{(quote ", ")}", |out| Ok(out.write_str(symbol)?));
            }
            Type::Parameter(parameter) => &parameter.name,
            Type::Map(map) => return self.map(map),
            Type::Set(item) => return self.collection(item, "set?"),
            Type::Vector(item) => return self.collection(item, "vector?"),
            Type::Tuple(items) => {
                return self.nested(1, "(clojure.spec.alpha/tuple", ")", |out| {
                    for item in items {
                        out.write_char(' ')?;
                        out.ty(item)?;
                    }
                    Ok(())
                });
            }
            Type::Function(arms) => match arms.as_ref() {
                [signature] if signature.parameters.is_none() => return self.callback(signature),
                _ => {
                    return Err(EncodeError::Unsupported(
                        "only a single non-generic callback signature is supported",
                    ));
                }
            },
            Type::Union(items) => return self.union(items),
            _ => {
                return Err(EncodeError::Unsupported(
                    "a type has no equivalent in the extern syntax",
                ));
            }
        };
        Ok(self.write_str(atom)?)
    }

    fn collection(&mut self, item: &Type, kind: &str) -> Encoded {
        self.nested(1, "(clojure.spec.alpha/coll-of ", ")", |out| {
            out.ty(item)?;
            Ok(write!(out, " :kind clojure.core/{kind}")?)
        })
    }

    fn bounded_collection(&mut self, ty: &Type, bounds: LengthBounds) -> Encoded {
        let (item, kind) = match ty {
            Type::SpecCollection(item) => (item.as_ref(), None),
            Type::Vector(item) => (item.as_ref(), Some("vector?")),
            Type::Set(item) => (item.as_ref(), Some("set?")),
            Type::Union(parts) => match parts.as_slice() {
                [Type::Seq(item), Type::SeqTuple(empty)] if empty.is_empty() => {
                    (item.as_ref(), Some("seq?"))
                }
                [Type::List(item), Type::ListTuple(empty)] if empty.is_empty() => {
                    (item.as_ref(), Some("list?"))
                }
                _ => {
                    return Err(EncodeError::Unsupported(
                        "bounded collection kind is unavailable",
                    ));
                }
            },
            _ => {
                return Err(EncodeError::Unsupported(
                    "bounded collection kind is unavailable",
                ));
            }
        };
        self.nested(1, "(clojure.spec.alpha/coll-of ", ")", |out| {
            out.ty(item)?;
            if let Some(kind) = kind {
                write!(out, " :kind clojure.core/{kind}")?;
            }
            if bounds.min > 0 {
                write!(out, " :min-count {}", bounds.min)?;
            }
            if let Some(max) = bounds.max {
                write!(out, " :max-count {max}")?;
            }
            Ok(())
        })
    }

    fn union(&mut self, items: &[Type]) -> Encoded {
        if let Some((predicate, _)) = items.first().and_then(exact_integer)
            && items.iter().all(|item| {
                exact_integer(item).is_some_and(|(item_predicate, _)| item_predicate == predicate)
            })
        {
            return self.integer_set(
                predicate,
                items
                    .iter()
                    .filter_map(|item| exact_integer(item).map(|(_, value)| value)),
            );
        }
        if let [Type::Nil, item] | [item, Type::Nil] = items {
            return self.nested(1, "(clojure.spec.alpha/nilable ", ")", |out| out.ty(item));
        }
        // Each sequential coll-of includes the empty case as well as its nonempty arm.
        let is_empty_seq = |ty: &Type| matches!(ty, Type::SeqTuple(xs) if xs.is_empty());
        let is_empty_list = |ty: &Type| matches!(ty, Type::ListTuple(xs) if xs.is_empty());
        let seq =
            items.iter().any(is_empty_seq) && items.iter().any(|ty| matches!(ty, Type::Seq(_)));
        let list =
            items.iter().any(is_empty_list) && items.iter().any(|ty| matches!(ty, Type::List(_)));
        let mut types = items
            .iter()
            .filter(|ty| !(seq && is_empty_seq(ty) || list && is_empty_list(ty)));
        let encode = |out: &mut Self, ty: &Type| match ty {
            Type::Seq(item) if seq => out.collection(item, "seq?"),
            Type::List(item) if list => out.collection(item, "list?"),
            _ => out.ty(ty),
        };
        if types.clone().count() == 1 {
            return encode(self, types.next().unwrap());
        }
        self.nested(1, "(clojure.spec.alpha/or", ")", |out| {
            for (index, ty) in types.enumerate() {
                write!(out, " :p{index} ")?;
                encode(out, ty)?;
            }
            Ok(())
        })
    }

    fn integer_set(&mut self, predicate: &str, values: impl Iterator<Item = i64>) -> Encoded {
        self.nested(2, "(clojure.spec.alpha/and clojure.core/", "})", |out| {
            write!(out, "{predicate} #{{")?;
            for (index, value) in values.enumerate() {
                out.separator(index)?;
                write!(out, "{value}")?;
            }
            Ok(())
        })
    }

    fn map(&mut self, map: &MapType) -> Encoded {
        match &map.extra {
            ExtraFields::Typed(types) if map.fields.is_empty() => {
                let [key, value] = types.as_ref();
                return self.nested(1, "(clojure.spec.alpha/map-of ", ")", |out| {
                    out.ty(key)?;
                    out.write_char(' ')?;
                    out.ty(value)
                });
            }
            ExtraFields::Typed(_) => {
                return Err(EncodeError::Unsupported(
                    "maps with both named fields and typed extra entries are not supported",
                ));
            }
            ExtraFields::Open if map.fields.is_empty() => {
                return Ok(self.write_str("clojure.core/map?")?);
            }
            ExtraFields::Closed if map.fields.is_empty() => return Ok(self.write_str("{}")?),
            _ => {}
        }

        let open = map.extra == ExtraFields::Open;
        let optional = map.fields.values().any(|field| !field.required);
        if !open && !optional {
            return self.nested(1, "{", "}", |out| out.fields(map, true));
        }
        self.nested(2, "(t/map {:req ", "})", |out| {
            if map.fields.values().any(|field| field.required) {
                out.nested(1, "{", "}", |out| out.fields(map, true))?;
            } else {
                out.write_str("{}")?;
            }
            if optional {
                out.nested(1, " :opt {", "}", |out| out.fields(map, false))?;
            }
            if open {
                out.write_str(" :extra :open")?;
            }
            Ok(())
        })
    }

    fn fields(&mut self, map: &MapType, required: bool) -> Encoded {
        let fields = map
            .fields
            .iter()
            .filter(|(_, field)| field.required == required);
        for (index, (key, field)) in fields.enumerate() {
            self.separator(index)?;
            self.key(key)?;
            self.write_char(' ')?;
            self.ty(&field.ty)?;
        }
        Ok(())
    }

    pub(super) fn key(&mut self, ty: &Type) -> Encoded {
        match ty {
            Type::Nil => Ok(self.write_str("nil")?),
            Type::Bool(value) => Ok(write!(self, "{value}")?),
            Type::Char(value) if u32::from(*value) <= 0xffff => {
                Ok(write!(self, "\\u{:04x}", u32::from(*value))?)
            }
            Type::Integer(value) => Ok(write!(self, "{value}")?),
            Type::Key(key) | Type::Sym(key) => Ok(self.write_str(key)?),
            // JSON's emitted string escapes are also accepted by the EDN reader.
            Type::Str(key) => serde_json::to_writer(self, key).map_err(|_| EncodeError::Size),
            _ => Err(EncodeError::Unsupported(
                "map fields require supported scalar literal keys",
            )),
        }
    }

    pub(super) fn separator(&mut self, index: usize) -> fmt::Result {
        if index != 0 {
            self.write_char(' ')?;
        }
        Ok(())
    }
}

fn exact_integer(ty: &Type) -> Option<(&'static str, i64)> {
    match ty {
        Type::Integer(value) => Some(("int?", i64::from(*value))),
        Type::Integral(IntegerType {
            class: IntegerClass::Fixed | IntegerClass::IntOrLong,
            range: IntegerRange::Exact(value),
        }) => Some(("int?", i64::from(*value))),
        Type::Integral(IntegerType {
            class: IntegerClass::Any,
            range: IntegerRange::Exact(value),
        }) => Some(("integer?", i64::from(*value))),
        Type::Integral(IntegerType {
            class: IntegerClass::Fixed | IntegerClass::IntOrLong,
            range: IntegerRange::ExactLong(value),
        }) => Some(("int?", *value)),
        Type::Integral(IntegerType {
            class: IntegerClass::Any,
            range: IntegerRange::ExactLong(value),
        }) => Some(("integer?", *value)),
        _ => None,
    }
}
