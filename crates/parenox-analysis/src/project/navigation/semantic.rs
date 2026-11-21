//! Source symbols classified from the project and lexical reference index.
use std::collections::BTreeMap;

use crate::SourceSymbolKind;
use crate::checker::ScopedReference;
use crate::core_names::known_core_function;
use crate::reader::{Kind, Span};

use super::{Session, SourceTarget, symbol_at};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticSymbolKind {
    Namespace,
    Type,
    Function,
    Macro,
    Class,
    Variable,
    Method,
    Interface,
}

#[derive(Clone, Copy, Debug)]
pub struct SemanticSymbol {
    pub span: Span,
    pub kind: SemanticSymbolKind,
    pub definition: bool,
    pub default_library: bool,
}

pub enum InspectedReference<'a> {
    Local(Span),
    Spec {
        path: &'a str,
        span: Span,
    },
    Var {
        path: &'a str,
        span: Span,
        namespace: &'a str,
        name: &'a str,
    },
    Core {
        name: &'a str,
        kind: SemanticSymbolKind,
    },
}

impl Session {
    /// Identify a cursor occurrence without treating semantic coloring as name resolution.
    pub fn inspect_reference(&self, path: &str, offset: usize) -> Option<InspectedReference<'_>> {
        if let Some(definition) = self.spec_definition(path, offset) {
            return Some(InspectedReference::Spec {
                path: self.files.get_key_value(&definition.path)?.0,
                span: definition.span,
            });
        }
        let file = self.current_file(path)?;
        let (declarations, forms) = file.navigation_source()?;
        let spelling = symbol_at(forms, offset)?;
        let references = self.collect_file_references(
            path,
            file,
            self.has_standard_test(),
            false,
            &mut BTreeMap::new(),
        );
        let reference = references
            .uses
            .iter()
            .find(|(span, _)| (span.start.offset..span.end.offset).contains(&offset))
            .map(|(_, reference)| *reference)
            .or_else(|| {
                super::declaration::top_level_reference(declarations, forms, spelling, offset)
            });
        let target = self
            .reference_target(path, offset, file, &references)
            .or_else(|| match reference? {
                ScopedReference::Referred(namespace, name) => self
                    .resolve_source_var(path, namespace, name, None)
                    .map(SourceTarget::Var),
                ScopedReference::InNamespace(name) => self
                    .resolve_source_var(path, declarations.namespace(), name, Some(offset))
                    .map(SourceTarget::Var),
                ScopedReference::Local(span) => Some(SourceTarget::Local(span)),
            });
        match target {
            Some(SourceTarget::Local(span)) => Some(InspectedReference::Local(span)),
            Some(SourceTarget::Var(var)) => Some(InspectedReference::Var {
                path: var.path,
                span: var.span,
                namespace: var.namespace,
                name: var.name,
            }),
            None => {
                let name = match reference? {
                    ScopedReference::Referred("clojure.core", name) => name,
                    ScopedReference::InNamespace(name)
                        if declarations.core_completion_visible(forms, name, offset) =>
                    {
                        name
                    }
                    _ => return None,
                };
                Some(InspectedReference::Core {
                    name,
                    kind: core_kind(name)?,
                })
            }
        }
    }

    /// Classify source declarations and lexical occurrences in the current snapshot.
    pub fn semantic_symbols(&self, path: &str) -> Vec<SemanticSymbol> {
        use SemanticSymbolKind::*;

        let Some(file) = self.current_file(path) else {
            return Vec::new();
        };
        let Some((decl, forms)) = file.navigation_source() else {
            return Vec::new();
        };
        let outline = decl.outline(forms);
        let mut result = Vec::new();
        let mut declarations = BTreeMap::new();
        for symbol in &outline {
            if let Some(kind) = declaration_kind(symbol.kind, true) {
                result.push(SemanticSymbol {
                    span: symbol.selection,
                    kind,
                    definition: !matches!(kind, Namespace | Interface),
                    default_library: false,
                });
            }
            if let Some(kind) = declaration_kind(symbol.kind, false) {
                declarations
                    .entry(symbol.name)
                    .and_modify(|entry| *entry = None)
                    .or_insert(Some((symbol.selection, kind)));
            }
        }

        let standard_test = self.has_standard_test();
        let mut providers = BTreeMap::new();
        let mut outlines = BTreeMap::new();
        let references =
            self.collect_file_references(path, file, standard_test, false, &mut providers);
        result.extend(references.bindings.into_iter().map(|span| SemanticSymbol {
            span,
            kind: Variable,
            definition: false,
            default_library: false,
        }));
        for (span, reference) in references.uses {
            let spelling = &file.source[span.start.offset..span.end.offset];
            let kind = match reference {
                ScopedReference::Local(_) => Some(Variable),
                ScopedReference::InNamespace(name) => declarations
                    .get(name)
                    .and_then(|entry| *entry)
                    .filter(|(definition, _)| definition.start.offset <= span.start.offset)
                    .map(|(_, kind)| kind)
                    .or_else(|| {
                        core_kind(name).filter(|_| {
                            decl.core_completion_visible(forms, name, span.start.offset)
                        })
                    }),
                ScopedReference::Referred(namespace, name) => {
                    let symbols = outlines.entry(namespace).or_insert_with(|| {
                        self.navigation_provider(namespace)
                            .map(|(_, _, source, forms)| {
                                source
                                    .outline(forms)
                                    .into_iter()
                                    .filter_map(|symbol| {
                                        Some((symbol.name, declaration_kind(symbol.kind, false)?))
                                    })
                                    .collect::<BTreeMap<_, _>>()
                            })
                    });
                    if let Some(symbols) = symbols {
                        symbols.get(name).copied()
                    } else {
                        library_kind(namespace, name)
                            .or_else(|| java_kind(spelling))
                            .or(Some(Function))
                    }
                }
            };
            if let Some(kind) = kind.or_else(|| java_kind(spelling)) {
                let dynamic = !matches!(reference, ScopedReference::Local(_))
                    && kind == Function
                    && spelling.len() > 2
                    && spelling.starts_with('*')
                    && spelling.ends_with('*');
                result.push(SemanticSymbol {
                    span,
                    kind: if dynamic { Variable } else { kind },
                    definition: false,
                    default_library: dynamic,
                });
            }
        }

        // The binding traversal starts at definitions. Classify the heads of
        // top-level forms here, where no local binding can shadow them.
        for form in forms {
            let candidate = match &form.without_metadata().kind {
                Kind::List(items) => items.first(),
                Kind::Symbol(_) => Some(form),
                _ => None,
            };
            let Some(head) = candidate else {
                continue;
            };
            let Some(name) = head.symbol() else { continue };
            let span = head.without_metadata().span;
            let (namespace, referred) = decl.top_level_call(name);
            let kind = if let Some((namespace, local)) = decl.qualified_reference(name) {
                library_kind(namespace, local).or_else(|| java_kind(name))
            } else if let Some((definition, kind)) = declarations.get(name).and_then(|entry| *entry)
                && definition.start.offset < span.start.offset
            {
                Some(kind)
            } else if namespace != decl.namespace() {
                library_kind(namespace, referred).or(Some(Function))
            } else if decl.core_completion_visible(forms, name, span.start.offset) {
                core_kind(name).or_else(|| java_kind(name))
            } else {
                None
            };
            if let Some(kind) = kind {
                result.push(SemanticSymbol {
                    span,
                    kind,
                    definition: false,
                    default_library: false,
                });
            }
        }
        result.sort_unstable_by_key(|symbol| symbol.span.start.offset);
        result
    }
}

fn declaration_kind(kind: SourceSymbolKind, definition: bool) -> Option<SemanticSymbolKind> {
    use SemanticSymbolKind as Semantic;
    Some(match kind {
        SourceSymbolKind::Namespace if definition => Semantic::Namespace,
        SourceSymbolKind::Type if !definition => Semantic::Type,
        SourceSymbolKind::Macro if !definition => Semantic::Macro,
        SourceSymbolKind::Protocol if definition => Semantic::Interface,
        SourceSymbolKind::Namespace | SourceSymbolKind::Spec => return None,
        // Var and protocol method declarations use the same presentation as
        // function vars in Clojure LSP.
        _ => Semantic::Function,
    })
}

fn library_kind(namespace: &str, name: &str) -> Option<SemanticSymbolKind> {
    use SemanticSymbolKind::{Function, Macro};
    match namespace {
        "clojure.core" => core_kind(name),
        "clojure.test" if matches!(name, "are" | "deftest" | "deftest-" | "is" | "testing") => {
            Some(Macro)
        }
        "clojure.spec.alpha"
            if matches!(
                name,
                "and"
                    | "alt"
                    | "cat"
                    | "coll-of"
                    | "def"
                    | "every"
                    | "fdef"
                    | "fspec"
                    | "keys"
                    | "map-of"
                    | "merge"
                    | "nilable"
                    | "or"
                    | "tuple"
                    | "*"
                    | "+"
                    | "?"
            ) =>
        {
            Some(Macro)
        }
        "clojure.test" | "clojure.string" | "clojure.set" | "clojure.spec.alpha" => Some(Function),
        _ => None,
    }
}

fn core_kind(name: &str) -> Option<SemanticSymbolKind> {
    use SemanticSymbolKind::{Function, Macro};
    Some(match name {
        "->" | "->>" | "and" | "as->" | "assert" | "binding" | "case" | "comment" | "cond"
        | "cond->" | "cond->>" | "def" | "defmacro" | "defmethod" | "defmulti" | "defn"
        | "defn-" | "defonce" | "defprotocol" | "defrecord" | "deftype" | "doseq" | "dotimes"
        | "fn" | "for" | "if-let" | "if-not" | "if-some" | "let" | "letfn" | "loop" | "or"
        | "some->" | "some->>" | "when" | "when-first" | "when-let" | "when-not" | "when-some"
        | "with-open" => Macro,
        _ if known_core_function(name) => Function,
        _ => return None,
    })
}

fn java_kind(spelling: &str) -> Option<SemanticSymbolKind> {
    if spelling.len() > 1 && spelling.starts_with('.') && !spelling.starts_with("..") {
        return Some(SemanticSymbolKind::Method);
    }
    let class = spelling
        .split_once('/')
        .map_or(spelling, |(class, _)| class);
    let class = class.strip_suffix('.').unwrap_or(class);
    let class_name = class.strip_prefix("java.lang.").unwrap_or(class);
    matches!(
        class_name,
        "Boolean"
            | "Byte"
            | "Character"
            | "Class"
            | "Double"
            | "Exception"
            | "Float"
            | "Integer"
            | "Long"
            | "Math"
            | "Number"
            | "Object"
            | "Short"
            | "String"
            | "StringBuilder"
            | "System"
            | "Throwable"
    )
    .then_some(SemanticSymbolKind::Class)
}
