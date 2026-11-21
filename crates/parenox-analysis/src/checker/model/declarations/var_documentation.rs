//! Literal var help extracted from already resolved declarations.
use super::{
    Declarations, SourceDeclaration, function_parts, lexical_function_clauses, multimethod_parts,
};
use crate::checker::function_forms::Clause;
use crate::reader::{Form, Kind, Span};

pub(crate) struct SourceDocumentation<'a> {
    pub documentation: Option<&'a str>,
    pub arglists: Vec<Span>,
}

impl Declarations {
    pub(crate) fn source_documentation<'a>(
        &'a self,
        forms: &'a [Form],
        name: &'a str,
    ) -> Option<SourceDocumentation<'a>> {
        let mut declarations = self.source_declarations(forms, name);
        let declaration = declarations.next()?;
        if declarations.next().is_some() {
            return None;
        }
        Some(match declaration {
            SourceDeclaration::Function([head, name, tail @ ..]) => {
                function_documentation(head, name, tail)
            }
            SourceDeclaration::Value([head, name, tail @ ..]) => {
                let (namespace, operator) = self.names.resolve_parts(head.symbol()?);
                if (namespace, operator) == ("clojure.core", "defmacro") {
                    return Some(function_documentation(head, name, tail));
                }
                if operator == "defmulti" && self.is_unshadowed_core_operator(forms, head, operator)
                {
                    let (attrs, _) = multimethod_parts(tail);
                    let explicit = attribute(attrs.as_chunks::<2>().0.iter(), ":arglists")
                        .or_else(|| name.metadata(":arglists"));
                    let arglists = match explicit {
                        Some(form) => declared_arglists(form),
                        None => self
                            .multimethod_clauses(forms, tail)
                            .map(|clauses| clause_arglists(clauses.into_values()))
                            .unwrap_or_default(),
                    };
                    return Some(SourceDocumentation {
                        documentation: multimethod_docstring(name, tail),
                        arglists,
                    });
                }
                let positional_doc = (namespace == "clojure.core"
                    && matches!((operator, tail), ("def" | "defmulti", [_, _, ..])))
                .then_some(tail)
                .and_then(|tail| tail.first())
                .filter(|first| first.is_string());
                SourceDocumentation {
                    documentation: docstring(positional_doc.or_else(|| name.metadata(":doc"))),
                    arglists: name
                        .metadata(":arglists")
                        .map_or_else(Vec::new, declared_arglists),
                }
            }
            SourceDeclaration::Protocol(var) => {
                if var.method {
                    SourceDocumentation {
                        documentation: docstring(
                            var.tail
                                .last()
                                .filter(|last| last.is_string())
                                .or_else(|| var.name.metadata(":doc")),
                        ),
                        arglists: var
                            .tail
                            .iter()
                            .take_while(|args| args.vector().is_some())
                            .map(|args| args.without_metadata().span)
                            .collect(),
                    }
                } else {
                    SourceDocumentation {
                        documentation: docstring(
                            var.tail
                                .first()
                                .filter(|first| first.is_string())
                                .or_else(|| var.name.metadata(":doc")),
                        ),
                        arglists: Vec::new(),
                    }
                }
            }
            _ => return None,
        })
    }

    /// Unqualified core names are usable only where an unresolved referral
    /// cannot replace them.
    pub(crate) fn hover_core_visible(&self, forms: &[Form], name: &str, offset: usize) -> bool {
        self.all_refers.is_empty()
            && self.has_known_var_surface(forms)
            && self.core_completion_visible(forms, name, offset)
            && !self.top_level_names(forms).contains(name)
    }
}

fn function_documentation<'a>(
    head: &Form,
    name: &'a Form,
    tail: &'a [Form],
) -> SourceDocumentation<'a> {
    let (arms, attrs) = function_parts(tail);
    let arglists = if let Some(value) = attribute(attrs, ":arglists") {
        declared_arglists(value)
    } else {
        lexical_function_clauses(arms, head.span)
            .map(|clauses| clause_arglists(clauses.into_values()))
            .unwrap_or_default()
    };
    SourceDocumentation {
        documentation: function_docstring(name, tail),
        arglists,
    }
}

fn clause_arglists<'a>(clauses: impl Iterator<Item = Clause<'a>>) -> Vec<Span> {
    let mut arglists: Vec<_> = clauses
        .map(|clause| clause.parameter_vector.without_metadata().span)
        .collect();
    arglists.sort_by_key(|span| span.start.offset);
    arglists
}

pub(super) fn function_docstring<'a>(name: &'a Form, tail: &'a [Form]) -> Option<&'a str> {
    let (_, attrs) = function_parts(tail);
    docstring(
        attribute(attrs, ":doc")
            .or_else(|| tail.first().filter(|form| form.is_string()))
            .or_else(|| name.metadata(":doc")),
    )
}

pub(super) fn multimethod_docstring<'a>(name: &'a Form, tail: &'a [Form]) -> Option<&'a str> {
    let (attrs, _) = multimethod_parts(tail);
    docstring(
        tail.first()
            .filter(|form| form.is_string())
            .or_else(|| attribute(attrs.as_chunks::<2>().0.iter(), ":doc"))
            .or_else(|| name.metadata(":doc")),
    )
}

fn attribute<'a>(mut attrs: impl Iterator<Item = &'a [Form; 2]>, key: &str) -> Option<&'a Form> {
    attrs.find_map(|[name, value]| (name.keyword() == Some(key)).then_some(value))
}

fn docstring(form: Option<&Form>) -> Option<&str> {
    match &form?.without_metadata().kind {
        Kind::String(text) => Some(text),
        _ => None,
    }
}

fn declared_arglists(form: &Form) -> Vec<Span> {
    let literal = match &form.without_metadata().kind {
        Kind::Quote(value) => value.as_ref(),
        Kind::List(items)
            if let [op, value] = items.as_slice()
                && op.symbol() == Some("quote") =>
        {
            value
        }
        _ => return Vec::new(),
    };
    let args = match &literal.without_metadata().kind {
        Kind::List(args) | Kind::Vector(args) => args,
        _ => return Vec::new(),
    };
    args.iter()
        .map(|arglist| arglist.vector().map(|_| arglist.without_metadata().span))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}
