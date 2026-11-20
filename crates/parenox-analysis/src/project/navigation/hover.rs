//! Source-backed hover help without evaluating declarations or metadata.
use crate::checker::Declarations;
use crate::reader::{Form, Span};

use super::{
    InspectedReference, ResolvedVar, ScopedReference, Session, SourceTarget, VarInfo,
    archive::namespace_declarations, declaration::top_level_reference, symbol_at, symbol_form_at,
};

#[derive(Default)]
pub struct VarDocumentation {
    pub documentation: Option<String>,
    pub arglists: Vec<String>,
}

pub struct SourceHover {
    pub info: VarInfo,
    pub documentation: Option<String>,
    pub arglists: Vec<String>,
}

pub struct LocalHover<'a> {
    pub name: &'a str,
    pub inferred_type: String,
    pub span: Span,
}

pub enum DocumentHover<'a> {
    Local(LocalHover<'a>),
    Var(SourceHover),
}

pub fn namespace_documentation(
    source: &str,
    namespace: &str,
    name: &str,
    portable: bool,
) -> Option<VarDocumentation> {
    let (declarations, forms) = namespace_declarations(source, namespace, portable)?;
    declarations.source_var(&forms, name, true)?;
    Some(var_documentation(&declarations, &forms, name, source))
}

impl Session {
    /// Hover a source var or an evaluated local occurrence in the current snapshot.
    pub fn document_hover_at(&self, path: &str, offset: usize) -> Option<DocumentHover<'_>> {
        let resolved = self.hover_target(path, offset);
        if let Some(SourceTarget::Var(target)) = resolved {
            return Some(DocumentHover::Var(target.hover()));
        }
        let globals = self.query_globals.as_ref()?;
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let site = symbol_form_at(&file.forms, offset)?;
        // A qualified symbol cannot be a local. For other names, the checker
        // can evaluate arguments through contracts that navigation cannot see.
        let symbol = site.symbol()?;
        if symbol != "/" && symbol.contains('/') {
            return None;
        }
        let ty = file.declarations.as_ref()?.local_read_type(
            &file.forms,
            file.model.as_ref()?,
            globals,
            &file.source,
            site,
        )?;
        Some(DocumentHover::Local(LocalHover {
            name: &file.source[site.span.start.offset..site.span.end.offset],
            inferred_type: ty.to_string(),
            span: site.span,
        }))
    }

    pub fn hover_at(&self, path: &str, offset: usize) -> Option<SourceHover> {
        let SourceTarget::Var(target) = self.hover_target(path, offset)? else {
            return None;
        };
        Some(target.hover())
    }

    fn hover_target(&self, path: &str, offset: usize) -> Option<SourceTarget<'_>> {
        if let Some((namespace, name)) = self.literal_require_qualified_var(path, offset) {
            return self
                .resolve_source_var(path, namespace, name, None)
                .map(SourceTarget::Var);
        }
        self.source_target(path, offset)
            .or_else(|| match self.inspect_reference(path, offset)? {
                InspectedReference::Local(span) => Some(SourceTarget::Local(span)),
                InspectedReference::Var {
                    path,
                    span,
                    namespace,
                    name,
                } => Some(SourceTarget::Var(ResolvedVar {
                    namespace,
                    name,
                    path,
                    span,
                    source: self.current_file(path)?,
                })),
                _ => None,
            })
    }

    /// Candidate archive var after resolving source lexical bindings and referrals.
    pub fn hover_archive_var(&self, path: &str, offset: usize) -> Option<(&str, &str)> {
        if let Some((namespace, name)) = self.literal_require_var_at(path, offset) {
            return (!self.has_source_namespace(namespace)).then_some((namespace, name));
        }
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        let qualified = declarations
            .referral_at(offset)
            .or_else(|| declarations.qualified_reference(symbol_at(forms, offset)?));
        let (namespace, name) = if let Some(qualified) = qualified {
            qualified
        } else {
            let reference = declarations
                .scoped_reference_at(
                    forms,
                    offset,
                    self.has_standard_test(),
                    |namespace, name, arity| self.navigation_call(namespace, name, arity),
                )
                .or_else(|| {
                    top_level_reference(declarations, forms, symbol_at(forms, offset)?, offset)
                })?;
            match reference {
                ScopedReference::Local(_) => return None,
                ScopedReference::Referred(namespace, name) => (namespace, name),
                ScopedReference::InNamespace(name) => {
                    if let Some((namespace, referred)) = file
                        .model
                        .as_ref()
                        .and_then(|model| model.resolved_referral(name))
                        && declarations.refers_all_from(namespace)
                    {
                        (namespace, referred)
                    } else if declarations.hover_core_visible(forms, name, offset) {
                        ("clojure.core", name)
                    } else {
                        return None;
                    }
                }
            }
        };
        (!self.has_source_namespace(namespace)).then_some((namespace, name))
    }

    pub fn hover_symbol_span(&self, path: &str, offset: usize) -> Option<Span> {
        let (_, forms) = self.current_file(path)?.navigation_source()?;
        Some(symbol_form_at(forms, offset)?.span)
    }
}

impl ResolvedVar<'_> {
    fn hover(&self) -> SourceHover {
        let VarDocumentation {
            documentation,
            arglists,
        } = self.documentation();
        SourceHover {
            info: self.info(),
            documentation,
            arglists,
        }
    }

    pub(super) fn documentation(&self) -> VarDocumentation {
        self.source.navigation_source().map_or_else(
            VarDocumentation::default,
            |(declarations, forms)| {
                var_documentation(declarations, forms, self.name, &self.source.source)
            },
        )
    }
}

fn var_documentation(
    declarations: &Declarations,
    forms: &[Form],
    name: &str,
    source: &str,
) -> VarDocumentation {
    let Some(docs) = declarations.source_documentation(forms, name) else {
        return VarDocumentation::default();
    };
    VarDocumentation {
        documentation: docs.documentation.map(str::to_owned),
        arglists: docs
            .arglists
            .into_iter()
            .filter_map(|span| {
                source
                    .get(span.start.offset..span.end.offset)
                    .map(str::to_owned)
            })
            .collect(),
    }
}
