//! Function specs shared by s/fspec type forms and top-level s/fdef declarations.
use std::collections::{BTreeMap, BTreeSet};

use super::{Names, TypeReader};
use crate::Diagnostic;
use crate::checker::function_forms::MAX_FIXED_PARAMETERS;
use crate::reader::{Form, Kind, Span};
use crate::types::{Signature, Type};

pub(in crate::checker::model) fn read_function_spec(
    names: &Names,
    options: &[Form],
    span: Span,
    source: &str,
    unchecked_inputs: Option<&BTreeSet<String>>,
    diagnostics: &mut Vec<Diagnostic>,
    lookup: impl FnMut(&str, Span) -> Option<Type>,
) -> Option<Vec<Signature>> {
    TypeReader {
        names,
        source,
        parameters: None,
        literal_shapes: false,
        unchecked_inputs,
        diagnostics,
        lookup,
    }
    .fdef(options, span)
}

impl<F: FnMut(&str, Span) -> Option<Type>> TypeReader<'_, F> {
    fn fdef(&mut self, options: &[Form], span: Span) -> Option<Vec<Signature>> {
        let (args, ret) = self.function_spec_options(options, span, "s/fdef")?;
        if self.spec_operator(args) == Some("alt") {
            return self.fixed_alt(args, ret);
        }
        self.cat_signature(args, ret, "s/fdef")
            .map(|signature| vec![signature])
    }

    pub(super) fn function_spec(
        &mut self,
        options: &[Form],
        span: Span,
        operator: &str,
    ) -> Option<Signature> {
        let (args, ret) = self.function_spec_options(options, span, operator)?;
        self.cat_signature(args, ret, operator)
    }

    fn function_spec_options<'a>(
        &mut self,
        options: &'a [Form],
        span: Span,
        operator: &str,
    ) -> Option<(&'a Form, &'a Form)> {
        let (pairs, remainder) = options.as_chunks::<2>();
        if let [key] = remainder {
            self.error(
                "PNX9001",
                key.span,
                format!("{operator} option requires a value"),
            );
            return None;
        }
        let (mut args, mut ret) = (None, None);
        for [key, value] in pairs {
            let slot = match key.keyword() {
                Some(":args") => &mut args,
                Some(":ret") => &mut ret,
                _ => {
                    self.error(
                        "PNX9001",
                        key.span,
                        format!("Only :args and :ret are supported in {operator}"),
                    );
                    return None;
                }
            };
            if slot.replace(value).is_some() {
                self.error("PNX2790", key.span, format!("Duplicate {operator} option"));
                return None;
            }
        }
        let (Some(args), Some(ret)) = (args, ret) else {
            self.error(
                "PNX9001",
                span,
                format!("{operator} requires both :args and :ret"),
            );
            return None;
        };
        Some((args, ret))
    }

    fn spec_operator<'a>(&'a self, form: &'a Form) -> Option<&'a str> {
        let Kind::List(items) = &form.without_metadata().kind else {
            return None;
        };
        let (namespace, operator) = self.names.resolve_parts(items.first()?.symbol()?);
        (namespace == "clojure.spec.alpha").then_some(operator)
    }

    fn regex_pairs<'a>(&self, form: &'a Form, operator: &str) -> Option<&'a [[Form; 2]]> {
        let Kind::List(items) = &form.without_metadata().kind else {
            return None;
        };
        if self.spec_operator(form) != Some(operator) {
            return None;
        }
        let (pairs, extra) = items[1..].as_chunks::<2>();
        extra.is_empty().then_some(pairs)
    }

    fn fixed_alt(&mut self, args: &Form, ret: &Form) -> Option<Vec<Signature>> {
        let Some(branches) = self
            .regex_pairs(args, "alt")
            .filter(|pairs| !pairs.is_empty())
        else {
            self.regex_gap(
                args,
                "s/alt requires keyword tags and at least one s/cat branch",
            );
            return None;
        };
        let mut tags = BTreeSet::new();
        let mut arities = BTreeMap::new();
        for [tag, branch] in branches {
            if !tag
                .keyword()
                .is_some_and(|tag| tags.insert(self.names.key(tag)))
            {
                self.regex_gap(tag, "s/alt requires distinct keyword tags");
                return None;
            }
            let Some(pairs) = self.regex_pairs(branch, "cat") else {
                self.regex_gap(branch, "s/fdef s/alt requires fixed s/cat branches");
                return None;
            };
            let (inputs, _) = self.cat_arguments(pairs, branch, true)?;
            if arities.insert(inputs.len(), inputs).is_some() {
                self.regex_gap(branch, "s/fdef s/alt branches have overlapping arities");
                return None;
            }
        }
        let ret = self.parse(ret);
        Some(
            arities
                .into_values()
                .map(|args| Signature {
                    args,
                    rest: None,
                    ret: ret.clone(),
                    parameters: None,
                })
                .collect(),
        )
    }

    fn cat_signature(&mut self, args: &Form, ret: &Form, operator: &str) -> Option<Signature> {
        let Some(pairs) = self.regex_pairs(args, "cat") else {
            self.error(
                "PNX9001",
                args.span,
                format!("{operator} :args requires (s/cat :tag Type ...)"),
            );
            return None;
        };
        let (args, rest) = self.cat_arguments(pairs, args, false)?;
        Some(Signature {
            args,
            rest,
            ret: self.parse(ret),
            parameters: None,
        })
    }

    fn cat_arguments(
        &mut self,
        pairs: &[[Form; 2]],
        args: &Form,
        fixed_only: bool,
    ) -> Option<(Vec<Type>, Option<Type>)> {
        let mut tags = BTreeSet::new();
        let mut fixed = Vec::new();
        let mut rest = None;
        for (index, [tag, ty]) in pairs.iter().enumerate() {
            if !tag
                .keyword()
                .is_some_and(|tag| tags.insert(self.names.key(tag)))
            {
                self.error("PNX9001", tag.span, "s/cat requires distinct keyword tags");
                return None;
            }
            let operator = self.spec_operator(ty);
            if fixed_only
                && matches!(
                    operator,
                    Some("cat" | "alt" | "*" | "+" | "?" | "&" | "keys*")
                )
            {
                self.regex_gap(ty, "s/fdef s/alt does not support nested argument regexes");
                return None;
            }
            if operator == Some("*")
                && let Kind::List(repeat) = &ty.without_metadata().kind
            {
                if index + 1 != pairs.len() || repeat.len() != 2 {
                    self.error(
                        "PNX9001",
                        ty.span,
                        "s/* requires one type and must be the last s/cat entry",
                    );
                    return None;
                }
                rest = Some(self.parse(&repeat[1]));
            } else {
                let diagnostic_count = self.diagnostics.len();
                let input = self.parse(ty);
                if fixed_only && input == Type::Unknown {
                    // An unresolved spec can itself be an argument regex.
                    if self.diagnostics.len() == diagnostic_count {
                        self.regex_gap(ty, "Cannot determine the arity of this s/cat entry");
                    }
                    return None;
                }
                fixed.push(input);
            }
        }
        if fixed.len() > MAX_FIXED_PARAMETERS {
            self.error(
                "PNX2554",
                args.span,
                "Clojure/JVM supports at most 20 fixed parameters",
            );
            return None;
        }
        Some((fixed, rest))
    }

    fn regex_gap(&mut self, form: &Form, message: &str) {
        self.diagnostics
            .push(Diagnostic::analysis_gap("PNX9001", form.span, message));
    }
}
