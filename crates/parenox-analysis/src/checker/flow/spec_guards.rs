//! Facts from a successful named spec check. A projection need not capture every
//! runtime constraint; it cannot prove success or characterize the false branch.
use crate::Diagnostic;
use crate::checker::{Checker, Env, Value, calls::call_returns, model::SpecConversion};
use crate::reader::{Form, Span};
use crate::types::{
    Compatibility, ExtraFields, Field, IntegerRange, IntegerType, MapSlot, SPEC_INVALID, Type,
};

#[derive(Clone, Copy)]
enum SpecOperation {
    Valid,
    Conform,
}

impl SpecOperation {
    fn success(self, value: Type) -> Type {
        match self {
            Self::Valid => Type::Bool(true),
            Self::Conform => value,
        }
    }

    fn invalid(self) -> Type {
        match self {
            Self::Valid => Type::Bool(false),
            Self::Conform => Type::Key(SPEC_INVALID.into()),
        }
    }
}

impl Checker<'_> {
    pub(in crate::checker) fn is_spec_validation(&self, name: &str) -> bool {
        matches!(self.spec_operation(name), Some(SpecOperation::Valid))
    }

    fn spec_operation(&self, name: &str) -> Option<SpecOperation> {
        let op = match name {
            "clojure.spec.alpha/valid?" => SpecOperation::Valid,
            "clojure.spec.alpha/conform" => SpecOperation::Conform,
            _ => return None,
        };
        (!self
            .globals
            .project_namespaces
            .contains("clojure.spec.alpha"))
        .then_some(op)
    }

    pub(in crate::checker) fn spec_operation_call(
        &mut self,
        name: &str,
        args: &[Form],
        span: Span,
        env: &Env,
    ) -> Option<Value> {
        let op = self.spec_operation(name)?;
        let values: Vec<_> = self.eval_args(args, env, true).collect();
        let returns = call_returns(&values);
        // Clojure's Specize implementation for Keyword ignores the optional
        // form when resolving the registry entry. It is still evaluated.
        let spec = match (op, args) {
            (_, [spec, _]) | (SpecOperation::Valid, [spec, _, _]) => spec,
            _ => {
                let count = match op {
                    SpecOperation::Valid => "2 or 3",
                    SpecOperation::Conform => "2",
                };
                self.error(
                    "PNX2554",
                    span,
                    format!("{name} expects {count} arguments, got {}", args.len()),
                );
                if returns {
                    self.may_throw();
                }
                return Some(Value::new(if returns {
                    Type::Unknown
                } else {
                    Type::Never
                }));
            }
        };
        if !returns {
            return Some(Value::new(Type::Never));
        }
        let key = spec
            .keyword()
            .and_then(|key| self.model.validation_key(key));
        let (ty, conversion) = key
            .as_deref()
            .map(|key| self.read_spec(key))
            .unwrap_or_default();
        if let Some(conversion) = conversion {
            return Some(Value::new(self.convert_spec(
                &values[1].0.known,
                &conversion,
                op,
            )));
        }
        if let SpecOperation::Conform = op {
            return Some(self.unchecked_form(
                spec.span,
                "s/conform requires a named spec with a supported conversion",
            ));
        }
        let gap = match (key, ty) {
            (Some(_), Some(ty)) if usable(ty) => None,
            (Some(key), Some(_)) => Some(format!("s/valid? cannot refine unchecked spec {key}")),
            (Some(key), None) => Some(format!("No source declaration for s/valid? spec {key}")),
            (None, _) => Some("s/valid? requires a literal, resolved named spec".into()),
        };
        // Even with a known projection, the runtime registry can impose additional
        // checks (for example on qualified extra keys of an open map).
        self.may_throw();
        if let Some(reason) = gap {
            self.diagnostics
                .push(Diagnostic::analysis_gap("PNX9001", spec.span, reason));
        }
        Some(Value::new(Type::Boolean))
    }

    fn convert_spec(
        &mut self,
        input: &Type,
        conversion: &SpecConversion,
        op: SpecOperation,
    ) -> Type {
        match input {
            Type::Never => Type::Never,
            Type::Union(members) => Type::union(
                members
                    .iter()
                    .map(|ty| self.convert_spec(ty, conversion, op)),
            ),
            Type::Parameter(parameter) if !matches!(conversion, SpecConversion::Identity) => self
                .convert_spec(
                    parameter.upper.as_ref().unwrap_or(&Type::Unknown),
                    conversion,
                    op,
                ),
            ty => match conversion {
                SpecConversion::CheckedInt32 => self.convert_int32(ty, op),
                SpecConversion::Identity => match ty.compatibility(&Type::SpecTruthy) {
                    Compatibility::Proven => op.success(ty.clone()),
                    Compatibility::Mismatch => op.invalid(),
                    Compatibility::Unproven => {
                        Type::union([op.success(refine(ty, &Type::SpecTruthy)), op.invalid()])
                    }
                },
                SpecConversion::IntEnum(entries) => convert_enum(ty, entries, op),
            },
        }
    }

    fn convert_int32(&mut self, input: &Type, op: SpecOperation) -> Type {
        let output = Type::Integral(IntegerType::FIXED_INT32);
        match input {
            Type::Integral(IntegerType {
                range: IntegerRange::OutsideInt32,
                ..
            }) => {
                self.arithmetic_exception();
                Type::Never
            }
            Type::Integral(IntegerType {
                range: IntegerRange::OutsideInt64,
                ..
            }) => {
                self.illegal_argument();
                Type::Never
            }
            ty if ty.subtype_of(&Type::Integral(IntegerType::INT32)) => {
                // Conversion changes the JVM number class, not the numeric
                // value. Keep a proven in-range literal when available.
                op.success(match ty {
                    Type::Integer(_) => ty.clone(),
                    _ => output,
                })
            }
            ty => match ty.compatibility(&Type::Integral(IntegerType::ANY)) {
                Compatibility::Mismatch => op.invalid(),
                other => {
                    self.arithmetic_exception();
                    self.illegal_argument();
                    let result = op.success(output);
                    if other == Compatibility::Proven {
                        result
                    } else {
                        Type::union([result, op.invalid()])
                    }
                }
            },
        }
    }

    pub(super) fn validation_projection(&self, form: &Form) -> Option<&Type> {
        let key = self.model.validation_key(form.keyword()?)?;
        self.globals.spec(&key).filter(|ty| usable(ty))
    }
}

fn convert_enum(input: &Type, entries: &[(i32, String)], op: SpecOperation) -> Type {
    let mut outcomes = vec![op.invalid()];
    for (key, value) in entries {
        match input.compatibility(&Type::Integer(*key)) {
            Compatibility::Proven => return op.success(Type::Key(value.clone())),
            Compatibility::Unproven => outcomes.push(op.success(Type::Key(value.clone()))),
            Compatibility::Mismatch => {}
        }
    }
    Type::union(outcomes)
}

fn usable(ty: &Type) -> bool {
    match ty {
        Type::Never
        | Type::SpecTruthy
        | Type::Nil
        | Type::Boolean
        | Type::Bool(_)
        | Type::String
        | Type::Str(_)
        | Type::Number
        | Type::Integral(_)
        | Type::Integer(_)
        | Type::Character
        | Type::Char(_)
        | Type::Keyword
        | Type::Key(_)
        | Type::Symbol
        | Type::SimpleSymbol
        | Type::QualifiedSymbol => true,
        Type::Union(parts) => parts.iter().all(usable),
        Type::Map(map) => {
            map.extra == ExtraFields::Open && map.fields.values().all(|field| usable(&field.ty))
        }
        _ => false,
    }
}

pub(super) fn refine(actual: &Type, projection: &Type) -> Type {
    if actual.subtype_of(projection) || matches!(actual, Type::Parameter(_)) {
        return actual.clone();
    }
    if *projection == Type::SpecTruthy {
        return actual.spec_truthy().unwrap_or(Type::SpecTruthy);
    }
    if *actual == Type::SpecTruthy {
        return projection
            .spec_truthy()
            .unwrap_or_else(|| projection.clone());
    }
    if let Some(intersection) = actual.integer_intersection(projection) {
        return intersection;
    }
    match (actual, projection) {
        (Type::Union(variants), projection) => {
            Type::union(variants.iter().map(|actual| refine(actual, projection)))
        }
        (_, Type::Union(members)) if members.iter().all(|ty| matches!(ty, Type::Integer(_))) => {
            Type::union(
                members
                    .iter()
                    .filter(|member| actual.compatibility(member) != Compatibility::Mismatch)
                    .cloned(),
            )
        }
        (Type::Map(actual), Type::Map(projection)) => {
            let mut shape = actual.clone();
            for (key, field) in &projection.fields {
                let (ty, required) = match actual.slot(key) {
                    MapSlot::Present(current) => (refine(current, &field.ty), true),
                    MapSlot::Optional(current) => (refine(current, &field.ty), field.required),
                    MapSlot::Absent if field.required => return Type::Never,
                    MapSlot::Absent => continue,
                    MapSlot::Undeclared => (field.ty.clone(), field.required),
                };
                if required && ty == Type::Never {
                    return Type::Never;
                }
                shape.set_field(key.clone(), Field::new(ty, required));
            }
            Type::Map(shape)
        }
        // The validated value is in the projection, even when the type system
        // cannot represent its intersection with the previous type.
        (_, projection) => projection.clone(),
    }
}
