//! Serialize resolved public contracts without source names or spec dependencies.
use std::fmt::Write;

use super::{ExternalDeclaration, Model, is_modeled_call};
use crate::reader::Span;
use crate::types::{ExtraFields, MapType, Signature, Type};

type Encoded = Result<String, &'static str>;

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

    pub(crate) fn exported_signatures(&self) -> impl Iterator<Item = (&str, Span, Encoded)> {
        self.exports.functions.iter().map(|(name, contract)| {
            let signature = if is_modeled_call(name) {
                Err("externs cannot replace a built-in checking rule")
            } else {
                encode_signatures(&contract.signatures)
            };
            (name.as_str(), self.functions[name].span, signature)
        })
    }
}

fn encode_signatures(signatures: &[Signature]) -> Encoded {
    match signatures {
        [one] => encode_signature(one),
        many => Ok(format!("[{}]", join(many.iter().map(encode_signature))?)),
    }
}

fn encode_signature(signature: &Signature) -> Encoded {
    let mut types = signature
        .args
        .iter()
        .map(encode_type)
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(rest) = &signature.rest {
        types.push(format!("& {}", encode_type(rest)?));
    }
    types.push(format!(":-> {}", encode_type(&signature.ret)?));
    let vector = format!("[{}]", types.join(" "));
    let Some(parameters) = &signature.parameters else {
        return Ok(vector);
    };
    let binders = join(parameters.iter().map(|parameter| match &parameter.upper {
        Some(upper) => Ok(format!("({} :< {})", parameter.name, encode_type(upper)?)),
        None => Ok(parameter.name.clone()),
    }))?;
    Ok(format!("(t/all [{binders}] {vector})"))
}

fn encode_callback(signature: &Signature) -> Encoded {
    let mut cat = String::from("(clojure.spec.alpha/cat");
    for (index, ty) in signature.args.iter().enumerate() {
        write!(cat, " :p{index} {}", encode_type(ty)?).unwrap();
    }
    if let Some(rest) = &signature.rest {
        write!(cat, " :rest (clojure.spec.alpha/* {})", encode_type(rest)?).unwrap();
    }
    Ok(format!(
        "(clojure.spec.alpha/fspec :args {cat}) :ret {})",
        encode_type(&signature.ret)?
    ))
}

fn encode_type(ty: &Type) -> Encoded {
    Ok(match ty {
        Type::Never => "#{}".into(),
        Type::Nil => "clojure.core/nil?".into(),
        Type::Boolean => "clojure.core/boolean?".into(),
        Type::Number => "clojure.core/number?".into(),
        Type::String => "clojure.core/string?".into(),
        Type::Character => "clojure.core/char?".into(),
        Type::Keyword => "clojure.core/keyword?".into(),
        Type::Symbol => "clojure.core/symbol?".into(),
        Type::Bool(value) => format!("#{{{value}}}"),
        Type::Char(value) if u32::from(*value) <= 0xffff => {
            format!("#{{\\u{:04x}}}", u32::from(*value))
        }
        Type::Key(_) | Type::Str(_) => format!("#{{{}}}", encode_key(ty)?),
        Type::Parameter(parameter) => parameter.name.clone(),
        Type::Map(map) => encode_map(map)?,
        Type::Set(item) => collection(item, "set?")?,
        Type::Vector(item) => collection(item, "vector?")?,
        Type::Tuple(items) => format!(
            "(clojure.spec.alpha/tuple {})",
            join(items.iter().map(encode_type))?
        ),
        Type::Function(arms) => match arms.as_ref() {
            [signature] if signature.parameters.is_none() => encode_callback(signature)?,
            _ => return Err("only a single non-generic callback signature is supported"),
        },
        Type::Union(items) => encode_union(items)?,
        _ => return Err("a type has no equivalent in the extern syntax"),
    })
}

fn collection(item: &Type, kind: &str) -> Encoded {
    Ok(format!(
        "(clojure.spec.alpha/coll-of {} :kind clojure.core/{kind})",
        encode_type(item)?
    ))
}

fn encode_union(items: &[Type]) -> Encoded {
    if let [Type::Nil, item] | [item, Type::Nil] = items {
        return Ok(format!(
            "(clojure.spec.alpha/nilable {})",
            encode_type(item)?
        ));
    }
    // coll-of seq? includes the empty seq as well as its nonempty arm.
    let is_empty_seq = |ty: &Type| matches!(ty, Type::SeqTuple(xs) if xs.is_empty());
    let seq = items.iter().any(is_empty_seq) && items.iter().any(|ty| matches!(ty, Type::Seq(_)));
    let types = items
        .iter()
        .filter(|ty| !seq || !is_empty_seq(ty))
        .map(|ty| match ty {
            Type::Seq(item) if seq => collection(item, "seq?"),
            _ => encode_type(ty),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if types.len() == 1 {
        return Ok(types.into_iter().next().unwrap());
    }
    let mut out = "(clojure.spec.alpha/or".to_owned();
    for (index, ty) in types.iter().enumerate() {
        write!(out, " :p{index} {ty}").unwrap();
    }
    out.push(')');
    Ok(out)
}

fn encode_map(map: &MapType) -> Encoded {
    match &map.extra {
        ExtraFields::Typed(types) if map.fields.is_empty() => {
            let [key, value] = types.as_ref();
            return Ok(format!(
                "(clojure.spec.alpha/map-of {} {})",
                encode_type(key)?,
                encode_type(value)?
            ));
        }
        ExtraFields::Typed(_) => {
            return Err("maps with both named fields and typed extra entries are not supported");
        }
        ExtraFields::Open if map.fields.is_empty() => return Ok("clojure.core/map?".into()),
        _ => {}
    }
    let fields = |required| {
        join(
            map.fields
                .iter()
                .filter(|(_, field)| field.required == required)
                .map(|(key, field)| {
                    Ok(format!("{} {}", encode_key(key)?, encode_type(&field.ty)?))
                }),
        )
    };
    let required = fields(true)?;
    let optional = fields(false)?;
    let open = map.extra == ExtraFields::Open;
    if !open && optional.is_empty() {
        return Ok(format!("{{{required}}}"));
    }
    let mut out = format!("(t/map {{:req {{{required}}}");
    if !optional.is_empty() {
        write!(out, " :opt {{{optional}}}").unwrap();
    }
    if open {
        out.push_str(" :extra :open");
    }
    out.push_str("})");
    Ok(out)
}

fn encode_key(ty: &Type) -> Encoded {
    match ty {
        Type::Key(key) => Ok(key.clone()),
        // JSON's emitted string escapes are also accepted by the EDN reader.
        Type::Str(key) => {
            Ok(serde_json::to_string(key).expect("string serialization is infallible"))
        }
        _ => Err("map fields require literal keyword or string keys"),
    }
}

fn join(items: impl Iterator<Item = Encoded>) -> Encoded {
    Ok(items.collect::<Result<Vec<_>, _>>()?.join(" "))
}
