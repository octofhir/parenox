//! Built-in spec conversions whose input checks and effects are known.
mod guarded_map;
mod live_lookup;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{
    Names, generation_options, literal_union_member, spec_field_key, spec_set_accepts_member,
};
use crate::checker::model::Declarations;
use crate::checker::model::specs::{
    EnumKind, MapFieldConversion, SpecConversion, StructuredConversion,
};
use crate::checker::model::vars::{adjacent_literal_def, prior_initializer};
use crate::reader::{Form, Kind};
use crate::types::{IntegerClass, IntegerType, SPEC_INVALID, Type};

const MAX_ENUM_ENTRIES: usize = 32;

pub(in crate::checker::model) fn source_conversion(
    names: &Names,
    form: &Form,
) -> Option<SpecConversion> {
    if let Kind::Set(items) = &form.without_metadata().kind
        && items.len() <= MAX_ENUM_ENTRIES
    {
        let members = items
            .iter()
            .map(|item| literal_union_member(names, item, false))
            .collect::<Option<Vec<_>>>()?;
        // Spec uses the set's result as a predicate and returns the input.
        return Some(SpecConversion::IdentityDomain(Arc::new(Type::union(
            members.into_iter().filter(spec_set_accepts_member),
        ))));
    }
    if resolves(names, form, "clojure.core", "identity") {
        return Some(SpecConversion::Identity);
    }
    if resolves(names, form, "clojure.core", "any?") {
        return Some(SpecConversion::Any);
    }
    if resolves(names, form, "clojure.core", "keyword?") {
        return Some(SpecConversion::Keyword);
    }
    if resolves(names, form, "clojure.core", "string?") {
        return Some(SpecConversion::String);
    }
    let Kind::List(items) = &form.without_metadata().kind else {
        return None;
    };
    let (head, stages) = items.split_first()?;
    if names.spec_core_scope_known
        && let [function] = stages
        && resolves(names, head, "clojure.spec.alpha", "conformer")
        && resolves(names, function, "clojure.core", "name")
    {
        return Some(SpecConversion::Name);
    }
    if let [function] = stages
        && resolves(names, head, "clojure.spec.alpha", "conformer")
        && let Some(conversion) = wrapper_conversion(names, function)
    {
        return Some(conversion);
    }
    if resolves(names, head, "clojure.spec.alpha", "coll-of")
        && let [element, options @ ..] = stages
        && resolves(names, element, "clojure.core", "any?")
        && generation_options(options)
    {
        return Some(SpecConversion::AnyCollection);
    }
    if !resolves(names, head, "clojure.spec.alpha", "and") {
        return None;
    }
    let [predicate, stage] = stages else {
        return None;
    };
    if matches!(predicate.kind, Kind::Symbol(_))
        && resolves(names, predicate, "clojure.core", "string?")
        && let Some(value) = compared_string(names, stage)
    {
        return Some(SpecConversion::IdentityDomain(Arc::new(Type::Str(
            value.to_owned(),
        ))));
    }
    let any = |form| resolves(names, form, "clojure.core", "any?");
    let keyword = |form| resolves(names, form, "clojure.core", "keyword?");
    if (any(predicate) && keyword(stage)) || (keyword(predicate) && any(stage)) {
        return Some(SpecConversion::Keyword);
    }
    let Kind::List(items) = &stage.without_metadata().kind else {
        return None;
    };
    let [head, function] = items.as_slice() else {
        return None;
    };
    if !resolves(names, head, "clojure.spec.alpha", "conformer") {
        return None;
    }
    if resolves(names, function, "clojure.core", "second") {
        return tagged_identity(names, predicate);
    }
    (resolves(names, predicate, "clojure.core", "integer?")
        && resolves(names, function, "clojure.core", "int"))
    .then_some(SpecConversion::CheckedInt32)
}

fn wrapper_conversion(names: &Names, function: &Form) -> Option<SpecConversion> {
    if let Kind::AnonymousFunction(function) = &function.kind
        && function.required == 1
        && !function.variadic
    {
        return conditional_wrapper(names, &function.body, "%1");
    }
    let (param, body) = unary_function(names, function)?;
    if let Kind::List(items) = &body.kind {
        return conditional_wrapper(names, items, param);
    }
    wrapper_key(names, body, param).map(|key| SpecConversion::WrapField(Arc::new(key)))
}

fn unary_function<'a>(names: &Names, function: &'a Form) -> Option<(&'a str, &'a Form)> {
    let Kind::List(items) = &function.kind else {
        return None;
    };
    let [head, params, body] = items.as_slice() else {
        return None;
    };
    let Kind::Vector(params) = &params.kind else {
        return None;
    };
    let [param] = params.as_slice() else {
        return None;
    };
    let Kind::Symbol(param) = &param.kind else {
        return None;
    };
    if !matches!(head.kind, Kind::Symbol(_))
        || !resolves(names, head, "clojure.core", "fn")
        || param == "&"
        || param.contains('/')
    {
        return None;
    }
    Some((param, body))
}

pub(in crate::checker::model) fn live_lookup_projection(
    names: &Names,
    namespace: &str,
    form: &Form,
) -> Option<SpecConversion> {
    live_lookup::parse(names, namespace, form)
}

pub(in crate::checker::model) fn live_lookup_conversion(
    declarations: &Declarations,
    forms: &[Form],
    form: &Form,
) -> Option<SpecConversion> {
    let conversion = live_lookup_projection(&declarations.names, declarations.namespace(), form)?;
    let SpecConversion::LiveLookupMap(projection) = &conversion else {
        return None;
    };
    declarations.live_lookup_initializer_span(forms, &projection.var, form.span.start.offset)?;
    Some(conversion)
}

fn conditional_wrapper(names: &Names, body: &[Form], param: &str) -> Option<SpecConversion> {
    let core = |form: &Form, name| {
        form.symbol() != Some(param) && resolves(names, form, "clojure.core", name)
    };
    let test = |form: &Form, name| {
        matches!(&form.kind, Kind::List(items)
            if matches!(items.as_slice(), [predicate, input]
                if core(predicate, name) && is_wrapper_input(input, param)))
    };
    match body {
        [head, vector, result] if core(head, "when") && test(vector, "vector?") => {
            let key = wrapper_key(names, result, param)?;
            Some(SpecConversion::VectorWrapField(Arc::new(key)))
        }
        [head, guard, result] if core(head, "when") => {
            guarded_map::parse(names, guard, result, param)
        }
        [head, vector, wrapped, map, identity, fallback, otherwise]
            if core(head, "cond")
                && test(vector, "vector?")
                && test(map, "map?")
                && is_wrapper_input(identity, param)
                && matches!(&fallback.kind, Kind::Keyword(key) if key == ":else") =>
        {
            let key = wrapper_key(names, wrapped, param)?;
            (key == wrapper_key(names, otherwise, param)?)
                .then(|| SpecConversion::MapOrWrapField(Arc::new(key)))
        }
        _ => None,
    }
}

fn wrapper_key(names: &Names, result: &Form, param: &str) -> Option<Type> {
    let Kind::Map(fields) = &result.kind else {
        return None;
    };
    let [key, value] = fields.as_slice() else {
        return None;
    };
    if !is_wrapper_input(value, param) {
        return None;
    }
    projection_key(names, key)
}

fn projection_key(names: &Names, key: &Form) -> Option<Type> {
    match &key.kind {
        Kind::Keyword(key) if key.starts_with("::") => names.validation_key(key).map(Type::Key),
        Kind::Keyword(key) => Some(Type::Key(key.clone())),
        Kind::String(key) => Some(Type::Str(key.clone())),
        _ => None,
    }
}

fn is_wrapper_input(form: &Form, param: &str) -> bool {
    matches!(&form.kind, Kind::Symbol(value) | Kind::AnonymousArgument(value) if value == param)
}

fn compared_string<'a>(names: &Names, predicate: &'a Form) -> Option<&'a str> {
    let Kind::AnonymousFunction(function) = &predicate.kind else {
        return None;
    };
    if function.required != 1 || function.variadic {
        return None;
    }
    let [head, left, right] = function.body.as_slice() else {
        return None;
    };
    if !matches!(head.kind, Kind::Symbol(_)) || !resolves(names, head, "clojure.core", "=") {
        return None;
    }
    match (&left.kind, &right.kind) {
        (Kind::AnonymousArgument(input), Kind::String(value))
        | (Kind::String(value), Kind::AnonymousArgument(input))
            if input == "%1" =>
        {
            Some(value)
        }
        _ => None,
    }
}

/// `second` returns the original value only when the chosen predicate preserves it.
fn tagged_identity(names: &Names, choice: &Form) -> Option<SpecConversion> {
    let branches = tagged_branches(names, choice)?;
    let mut domains = Vec::new();
    for [_, predicate] in branches {
        let domain = if resolves(names, predicate, "clojure.core", "keyword?") {
            Type::SpecKeyword
        } else if resolves(names, predicate, "clojure.core", "int?") {
            Type::Integral(IntegerType::FIXED)
        } else {
            return None;
        };
        domains.push(domain);
    }
    Some(SpecConversion::IdentityDomain(Arc::new(Type::union(
        domains,
    ))))
}

fn tagged_branches<'a>(names: &Names, choice: &'a Form) -> Option<&'a [[Form; 2]]> {
    let Kind::List(items) = &choice.without_metadata().kind else {
        return None;
    };
    let (head, args) = items.split_first()?;
    let (branches, unmatched) = args.as_chunks::<2>();
    if !resolves(names, head, "clojure.spec.alpha", "or")
        || branches.is_empty()
        || !unmatched.is_empty()
    {
        return None;
    }
    let mut tags = BTreeSet::new();
    for [tag, _] in branches {
        if !tags.insert(names.key(tag.keyword()?)) {
            return None;
        }
    }
    Some(branches)
}

pub(in crate::checker::model) fn named_tagged_branches<'a>(
    names: &Names,
    form: &'a Form,
) -> Option<&'a [[Form; 2]]> {
    let branches = tagged_branches(names, form)?;
    branches
        .iter()
        .all(|[_, spec]| spec.keyword().is_some())
        .then_some(branches)
}

pub(in crate::checker::model) fn tagged_or(
    names: &Names,
    form: &Form,
    lookup: impl Fn(&str) -> Option<SpecConversion>,
) -> Option<SpecConversion> {
    let branches = named_tagged_branches(names, form)?;
    let conversions = branches
        .iter()
        .map(|[tag, spec]| {
            let tag = tag.keyword()?;
            if tag.starts_with("::") {
                names.validation_key(tag)?;
            }
            let conversion = lookup(&names.validation_key(spec.keyword()?)?)?;
            if !matches!(&conversion, SpecConversion::Structured(spec)
                if matches!(spec.as_ref(), StructuredConversion::Map(_)))
            {
                return None;
            }
            Some((names.key(tag), conversion))
        })
        .collect::<Option<_>>()?;
    Some(SpecConversion::TaggedOr(conversions))
}

pub(in crate::checker::model) fn projected_collection_child<'a>(
    names: &Names,
    form: &'a Form,
) -> Option<&'a Form> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [and, collection, projection] = items.as_slice() else {
        return None;
    };
    let (Kind::List(collection), Kind::List(projection)) = (&collection.kind, &projection.kind)
    else {
        return None;
    };
    let ([coll_of, child], [conformer, mapping]) = (collection.as_slice(), projection.as_slice())
    else {
        return None;
    };
    let Kind::AnonymousFunction(mapping) = &mapping.kind else {
        return None;
    };
    let [map, second, input] = mapping.body.as_slice() else {
        return None;
    };
    (matches!(child.kind, Kind::Keyword(_))
        && mapping.required == 1
        && !mapping.variadic
        && matches!(&input.kind, Kind::AnonymousArgument(input) if input == "%1")
        && matches!(map.kind, Kind::Symbol(_))
        && matches!(second.kind, Kind::Symbol(_))
        && resolves(names, and, "clojure.spec.alpha", "and")
        && resolves(names, coll_of, "clojure.spec.alpha", "coll-of")
        && resolves(names, conformer, "clojure.spec.alpha", "conformer")
        && resolves(names, map, "clojure.core", "map")
        && resolves(names, second, "clojure.core", "second"))
    .then_some(child)
}

pub(in crate::checker::model) fn string_keyed_map_child<'a>(
    names: &Names,
    form: &'a Form,
) -> Option<&'a Form> {
    let Kind::List(items) = &form.kind else {
        return None;
    };
    let [coll_of, entry, kind, map] = items.as_slice() else {
        return None;
    };
    let Kind::List(entry) = &entry.kind else {
        return None;
    };
    let [tuple, string, child] = entry.as_slice() else {
        return None;
    };
    (matches!(child.kind, Kind::Keyword(_))
        && kind.keyword() == Some(":kind")
        && resolves(names, coll_of, "clojure.spec.alpha", "coll-of")
        && resolves(names, tuple, "clojure.spec.alpha", "tuple")
        && resolves(names, string, "clojure.core", "string?")
        && resolves(names, map, "clojure.core", "map?"))
    .then_some(child)
}

fn projected_pair<'a>(names: &Names, form: &'a Form) -> Option<[&'a Form; 2]> {
    let Kind::List(items) = &form.without_metadata().kind else {
        return None;
    };
    let [head, choice, stage] = items.as_slice() else {
        return None;
    };
    if !resolves(names, head, "clojure.spec.alpha", "and") {
        return None;
    }
    let Kind::List(items) = &stage.without_metadata().kind else {
        return None;
    };
    let [conformer, function] = items.as_slice() else {
        return None;
    };
    if !resolves(names, conformer, "clojure.spec.alpha", "conformer")
        || !resolves(names, function, "clojure.core", "second")
    {
        return None;
    }
    let [[_, left], [_, right]] = tagged_branches(names, choice)? else {
        return None;
    };
    Some([left, right])
}

pub(in crate::checker::model) fn tagged_projection_branches<'a>(
    names: &Names,
    form: &'a Form,
) -> Option<[&'a Form; 2]> {
    let [left, right] = projected_pair(names, form)?;
    let alternative = |form: &Form| {
        resolves(names, form, "clojure.core", "string?")
            || matches!(&form.without_metadata().kind, Kind::List(items)
                if matches!(items.as_slice(), [head, _, options @ ..]
                    if resolves(names, head, "clojure.spec.alpha", "coll-of")
                        && generation_options(options)))
    };
    let map = |form: &Form| {
        form.keyword().is_some()
            || matches!(&form.without_metadata().kind, Kind::List(items)
                if items.first().is_some_and(|head| resolves(names, head, "clojure.spec.alpha", "keys")))
    };
    ((map(left) && alternative(right)) || (map(right) && alternative(left)))
        .then_some([left, right])
}

/// Remove the tag from a string or collection and a map choice.
pub(in crate::checker::model) fn tagged_projection(
    names: &Names,
    form: &Form,
    input: &Type,
    lookup: impl Fn(&str) -> Option<SpecConversion>,
) -> Option<SpecConversion> {
    let [left, right] = tagged_projection_branches(names, form)?;
    let branch = |form: &Form| {
        if resolves(names, form, "clojure.core", "string?") {
            return Some(SpecConversion::String);
        }
        if let Some(key) = form.keyword() {
            let conversion = lookup(&names.key(key))?;
            return matches!(&conversion, SpecConversion::Structured(map)
                if matches!(map.as_ref(), StructuredConversion::Map(_)))
            .then_some(conversion);
        }
        let Type::Union(parts) = input else {
            return None;
        };
        parts
            .iter()
            .find_map(|ty| structured_conversion(names, form, ty, &lookup))
    };
    Some(SpecConversion::TaggedChoice(
        [branch(left)?, branch(right)?].into(),
    ))
}

/// Compose exact child conversions that the spec resolver has already indexed.
pub(in crate::checker::model) fn structured_conversion(
    names: &Names,
    form: &Form,
    input: &Type,
    lookup: impl Fn(&str) -> Option<SpecConversion>,
) -> Option<SpecConversion> {
    let Kind::List(items) = &form.without_metadata().kind else {
        return None;
    };
    let (head, args) = items.split_first()?;
    let conversion = match (input, args) {
        (Type::Map(map), options) if resolves(names, head, "clojure.spec.alpha", "keys") => {
            let (pairs, unmatched) = options.as_chunks::<2>();
            if !unmatched.is_empty() {
                return None;
            }
            let mut fields = BTreeMap::new();
            for [directive, keys] in pairs {
                let directive = directive.keyword()?;
                if !matches!(directive, ":req" | ":opt" | ":req-un" | ":opt-un") {
                    return None;
                }
                for key in keys.vector()? {
                    let qualified = names.key(key.keyword()?);
                    let key = Type::Key(spec_field_key(&qualified, directive)?);
                    let field = map.fields.get(&key)?;
                    let conversion = if field.value_constraint().is_some() {
                        Some(lookup(&qualified)?)
                    } else {
                        None
                    };
                    fields.insert(
                        key,
                        MapFieldConversion {
                            registration: qualified,
                            required: field.required,
                            conversion,
                        },
                    );
                }
            }
            StructuredConversion::Map(fields)
        }
        (Type::SpecCollection(item), [child, options @ ..])
            if **item != Type::SpecValue
                && resolves(names, head, "clojure.spec.alpha", "coll-of")
                && generation_options(options) =>
        {
            let (element, registration) = if let Some(key) = child.keyword() {
                let name = names.key(key);
                let element = lookup(&name)?;
                let registration = options
                    .is_empty()
                    .then(|| names.validation_key(key))
                    .flatten();
                (element, registration)
            } else {
                (source_conversion(names, child)?, None)
            };
            StructuredConversion::Collection {
                element,
                registration,
            }
        }
        _ => return None,
    };
    Some(SpecConversion::Structured(Arc::new(conversion)))
}

/// Exact source patterns where the same finite table checks and converts the input.
pub(in crate::checker::model) fn enum_conversion(
    declarations: &Declarations,
    forms: &[Form],
    source: &str,
    form: &Form,
) -> Option<SpecConversion> {
    let names = &declarations.names;
    let Some([left, right]) = projected_pair(names, form) else {
        return table_conversion(declarations, forms, source, form, false);
    };
    let is_string = |form| resolves(names, form, "clojure.core", "string?");
    let (table, string_first) = match (is_string(left), is_string(right)) {
        (true, false) => (right, true),
        (false, true) => (left, false),
        _ => return None,
    };
    let enumeration = table_conversion(declarations, forms, source, table, true)?;
    let mut branches = [enumeration, SpecConversion::String];
    if string_first {
        branches.swap(0, 1);
    }
    Some(SpecConversion::TaggedChoice(branches.into()))
}

fn table_conversion(
    declarations: &Declarations,
    forms: &[Form],
    source: &str,
    form: &Form,
    keyword_strings: bool,
) -> Option<SpecConversion> {
    let names = &declarations.names;
    let ([predicate, lookup, convert], plain) = enum_stages(names, form)?;
    let integer_keys = resolves(names, predicate, "clojure.core", "int?");
    let inverted = resolves(names, predicate, "clojure.core", "integer?");
    let keyword_keys = resolves(names, predicate, "clojure.core", "keyword?");
    if !names.spec_core_scope_known
        || !(integer_keys || inverted || keyword_keys)
        || (keyword_strings && !keyword_keys)
    {
        return None;
    }
    let registration = || enum_registration(declarations, forms, form);
    if integer_keys
        && plain
        && let Some(entries) = matching_integer_keywords(lookup, convert, source)
        && registration().is_some()
    {
        return Some(fixed_integer_keywords(entries));
    }
    let local = local_var(names, lookup)?;
    if local_var(names, convert)? != local {
        return None;
    }
    if integer_keys
        && plain
        && matches!(
            (&lookup.kind, &convert.kind),
            (Kind::Symbol(_), Kind::Symbol(_))
        )
        && let Some(index) = registration()
        && let Some((name, table)) = adjacent_literal_def(declarations, forms, index)
        && name == local
        && let Some(entries) = integer_keyword_entries(table, source)
    {
        return Some(fixed_integer_keywords(entries));
    }
    let initializer = prior_initializer(declarations, forms, local, form.span.start.offset)?;
    if inverted {
        declarations.reader_namespace_before(forms, form.span.start.offset)?;
        return inverted_enum(declarations, forms, source, initializer);
    }
    if keyword_strings {
        declarations.reader_namespace_before(forms, initializer.span.start.offset)?;
        if !matches!(initializer.kind, Kind::Map(_)) {
            return None;
        }
    }
    let entries = initializer.map()?.as_chunks::<2>().0;
    if entries.len() > MAX_ENUM_ENTRIES {
        return None;
    }
    let mut values = BTreeMap::new();
    for [key, value] in entries {
        if keyword_strings
            && (!matches!(key.kind, Kind::Keyword(_)) || matches!(value.kind, Kind::Metadata(..)))
        {
            return None;
        }
        let key = if integer_keys {
            let value = fixed_integer(key, source)?.exact_integer_value()?;
            Type::exact_integer(value, IntegerClass::Fixed)
        } else {
            Type::Key(names.key(key.keyword()?))
        };
        let value = match &value.without_metadata().kind {
            Kind::Nil | Kind::Bool(false) => None,
            Kind::Keyword(keyword) => {
                let keyword = names.key(keyword);
                if keyword == SPEC_INVALID {
                    None
                } else if integer_keys {
                    Some(Type::Key(keyword))
                } else {
                    return None;
                }
            }
            Kind::String(value) if keyword_strings => Some(Type::Str(value.clone())),
            _ if !integer_keys && !keyword_strings => Some(fixed_integer(value, source)?),
            _ => return None,
        };
        let value = value.filter(|_| !matches!(&key, Type::Key(name) if name == SPEC_INVALID));
        if values.insert(key, value).is_some() {
            return None;
        }
    }
    Some(SpecConversion::Enum {
        entries: values
            .into_iter()
            .filter_map(|(key, value)| Some((key, value?)))
            .collect(),
        kind: EnumKind::LocalTable,
    })
}

fn enum_stages<'a>(names: &Names, form: &'a Form) -> Option<([&'a Form; 3], bool)> {
    let Kind::List(items) = &form.without_metadata().kind else {
        return None;
    };
    let [head, predicate, lookup, stage] = items.as_slice() else {
        return None;
    };
    let Kind::List(stage_items) = &stage.without_metadata().kind else {
        return None;
    };
    let [conformer, convert] = stage_items.as_slice() else {
        return None;
    };
    let plain = matches!(form.kind, Kind::List(_))
        && matches!(stage.kind, Kind::List(_))
        && [head, predicate, conformer]
            .iter()
            .all(|form| matches!(form.kind, Kind::Symbol(_)));
    (resolves(names, head, "clojure.spec.alpha", "and")
        && resolves(names, conformer, "clojure.spec.alpha", "conformer"))
    .then_some(([predicate, lookup, convert], plain))
}

pub(in crate::checker::model) fn literal_integer_enum(
    names: &Names,
    form: &Form,
    source: &str,
) -> Option<SpecConversion> {
    let ([predicate, lookup, convert], plain) = enum_stages(names, form)?;
    if !plain
        || !resolves(names, predicate, "clojure.core", "int?")
        || has_reader_discard(form, source)
    {
        return None;
    }
    matching_integer_keywords(lookup, convert, source).map(fixed_integer_keywords)
}

fn enum_registration(
    declarations: &Declarations,
    forms: &[Form],
    spec_body: &Form,
) -> Option<usize> {
    let index = forms.partition_point(|form| form.span.end.offset <= spec_body.span.start.offset);
    let registration = forms.get(index)?;
    let Kind::List(items) = &registration.kind else {
        return None;
    };
    let [head, name, body] = items.as_slice() else {
        return None;
    };
    (body.span == spec_body.span
        && matches!(name.kind, Kind::Keyword(_))
        && matches!(head.kind, Kind::Symbol(_))
        && resolves(&declarations.names, head, "clojure.spec.alpha", "def")
        && !declarations.has_unchecked_dispatch(registration.span))
    .then_some(index)
}

fn has_reader_discard(form: &Form, source: &str) -> bool {
    let text = &source[form.span.start.offset..form.span.end.offset];
    text.contains("#_") || text.contains("#=")
}

fn matching_integer_keywords(
    lookup: &Form,
    convert: &Form,
    source: &str,
) -> Option<BTreeMap<Type, Type>> {
    let entries = integer_keyword_entries(lookup, source)?;
    (entries == integer_keyword_entries(convert, source)?).then_some(entries)
}

fn integer_keyword_entries(form: &Form, source: &str) -> Option<BTreeMap<Type, Type>> {
    let Kind::Map(items) = &form.kind else {
        return None;
    };
    let (pairs, unpaired) = items.as_chunks::<2>();
    if !unpaired.is_empty() || !(1..=MAX_ENUM_ENTRIES).contains(&pairs.len()) {
        return None;
    }
    let mut entries = BTreeMap::new();
    for [key, value] in pairs {
        if !matches!(key.kind, Kind::Number) {
            return None;
        }
        let key = fixed_integer(key, source)?;
        let Kind::Keyword(value) = &value.kind else {
            return None;
        };
        if value.starts_with("::")
            || value == SPEC_INVALID
            || entries.insert(key, Type::Key(value.clone())).is_some()
        {
            return None;
        }
    }
    Some(entries)
}

fn fixed_integer_keywords(entries: BTreeMap<Type, Type>) -> SpecConversion {
    SpecConversion::Enum {
        entries: entries
            .into_iter()
            .map(|(key, value)| {
                let code = key.exact_integer_value().expect("fixed integer enum key");
                (Type::exact_integer(code, IntegerClass::Fixed), value)
            })
            .collect(),
        kind: EnumKind::FixedIntegerKeywords,
    }
}

fn inverted_enum(
    declarations: &Declarations,
    forms: &[Form],
    source: &str,
    initializer: &Form,
) -> Option<SpecConversion> {
    let entries = inverted_entries(declarations, forms, source, initializer)?;
    Some(SpecConversion::Enum {
        entries: entries
            .into_iter()
            .filter_map(|(code, _, key)| {
                let key = declarations.names.key(key.keyword()?);
                (key != SPEC_INVALID)
                    .then(|| (Type::exact_integer(code, IntegerClass::Any), Type::Key(key)))
            })
            .collect(),
        kind: EnumKind::Inverted,
    })
}

/// Materialize the same bounded inversion as inert map data, including markers.
pub(in crate::checker::model) fn inverted_literal(
    declarations: &Declarations,
    forms: &[Form],
    source: &str,
    initializer: &Form,
) -> Option<Form> {
    let entries = inverted_entries(declarations, forms, source, initializer)?;
    Some(Form {
        kind: Kind::Map(
            entries
                .into_iter()
                .flat_map(|(_, code, key)| [code.clone(), key.clone()])
                .collect(),
        ),
        span: initializer.span,
    })
}

fn inverted_entries<'a>(
    declarations: &'a Declarations,
    forms: &'a [Form],
    source: &str,
    initializer: &'a Form,
) -> Option<Vec<(i32, &'a Form, &'a Form)>> {
    let names = &declarations.names;
    let Kind::List(items) = &initializer.without_metadata().kind else {
        return None;
    };
    let [invert, table] = items.as_slice() else {
        return None;
    };
    if !names.spec_core_scope_known
        || !names.loads_library("clojure.set")
        || !resolves(names, invert, "clojure.set", "map-invert")
        || declarations.has_unchecked_dispatch(initializer.span)
    {
        return None;
    }
    declarations.reader_namespace_before(forms, initializer.span.start.offset)?;
    let table = prior_initializer(
        declarations,
        forms,
        local_var(names, table)?,
        initializer.span.start.offset,
    )?;
    declarations.reader_namespace_before(forms, table.span.start.offset)?;
    let entries = table.map()?.as_chunks::<2>().0;
    if entries.len() > MAX_ENUM_ENTRIES {
        return None;
    }
    let mut codes = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut values = Vec::with_capacity(entries.len());
    for [key, number] in entries {
        let Type::Integer(code) = fixed_integer(number, source)? else {
            return None;
        };
        // map-invert would choose a winner by map traversal order.
        if !codes.insert(code) || !keys.insert(names.key(key.keyword()?)) {
            return None;
        }
        values.push((code, number, key));
    }
    values.sort_unstable_by_key(|(code, _, _)| *code);
    Some(values)
}

impl Declarations {
    pub(crate) fn live_lookup_initializer_span(
        &self,
        forms: &[Form],
        var: &str,
        before: usize,
    ) -> Option<crate::reader::Span> {
        let (namespace, local) = var.split_once('/')?;
        if namespace != self.namespace() {
            return None;
        }
        let index = forms.partition_point(|form| form.span.end.offset <= before);
        let (name, value) = adjacent_literal_def(self, forms, index)?;
        if name != local {
            return None;
        }
        let Kind::Map(entries) = &value.kind else {
            return None;
        };
        (entries.len() <= 2 * MAX_ENUM_ENTRIES
            && entries.iter().all(|entry| {
                matches!(
                    entry.kind,
                    Kind::Keyword(_) | Kind::String(_) | Kind::Number | Kind::Bool(_) | Kind::Nil
                )
            }))
        .then_some(value.span)
    }
}

fn fixed_integer(form: &Form, source: &str) -> Option<Type> {
    let ty = Type::integer_literal(form, source)?;
    matches!(ty, Type::Integer(_)).then_some(ty)
}

fn local_var<'a>(names: &Names, form: &'a Form) -> Option<&'a str> {
    let symbol = form.symbol()?;
    if let Some((_, local)) = symbol.split_once('/') {
        (names.resolve_parts(symbol).0 == names.namespace).then_some(local)
    } else {
        (!names.refers.contains_key(symbol) && !names.imports.contains_key(symbol))
            .then_some(symbol)
    }
}

pub(in crate::checker::model) fn resolves(
    names: &Names,
    form: &Form,
    namespace: &str,
    name: &str,
) -> bool {
    form.symbol().is_some_and(|symbol| {
        names.resolve_parts(symbol) == (namespace, name)
            && (symbol.contains('/')
                || (names.spec_core_scope_known && !names.spec_core_shadows.contains(symbol)))
    })
}
