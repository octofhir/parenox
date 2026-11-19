//! Serialize resolved public contracts without source names or spec dependencies.
use std::fmt::{self, Write};

use super::{ExternalDeclaration, Model, is_modeled_call};
use crate::reader::{MAX_NESTING_DEPTH, Span};
use crate::types::{ExtraFields, IntegerType, MapType, Signature, Type};

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
            (Some(ExternalDeclaration::Function(parsed)), None) => self
                .exports
                .functions
                .get(name)
                .is_some_and(|source| source.signatures == parsed.signatures),
            (Some(ExternalDeclaration::Literal(parsed)), Some(edn)) => {
                parsed.edn == edn && self.exports.literals.get(name) == Some(&parsed.ty)
            }
            _ => false,
        }
    }

    pub(crate) fn exported_signatures(
        &self,
    ) -> impl Iterator<Item = (&str, Span, Result<&[Signature], &'static str>)> {
        self.exports.functions.iter().map(|(name, contract)| {
            let signature = if is_modeled_call(name) {
                Err("externs cannot replace a built-in checking rule")
            } else {
                Ok(contract.signatures.as_ref())
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
    fn nested(
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

    pub(crate) fn signatures(&mut self, signatures: &[Signature]) -> Encoded {
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

    fn ty(&mut self, ty: &Type) -> Encoded {
        let atom = match ty {
            Type::Never => "#{}",
            Type::Nil => "clojure.core/nil?",
            Type::Boolean => "clojure.core/boolean?",
            Type::SpecTruthy => "clojure.core/identity",
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
            Type::Character => "clojure.core/char?",
            Type::Keyword => "clojure.core/keyword?",
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

    fn union(&mut self, items: &[Type]) -> Encoded {
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

    fn key(&mut self, ty: &Type) -> Encoded {
        match ty {
            Type::Key(key) => Ok(self.write_str(key)?),
            // JSON's emitted string escapes are also accepted by the EDN reader.
            Type::Str(key) => serde_json::to_writer(self, key).map_err(|_| EncodeError::Size),
            _ => Err(EncodeError::Unsupported(
                "map fields require literal keyword or string keys",
            )),
        }
    }

    fn separator(&mut self, index: usize) -> fmt::Result {
        if index != 0 {
            self.write_char(' ')?;
        }
        Ok(())
    }
}
