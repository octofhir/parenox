//! Resolve Java class names without loading a class.
use super::{FileCache, Session, SymbolSite, source_symbol_form_at, symbol_form_at};

impl Session {
    /// Return an imported Java class or a qualified class candidate for the
    /// classpath index to confirm. Clojure namespace bindings take priority.
    pub fn completion_java_class<'a>(&'a self, path: &str, qualifier: &'a str) -> Option<&'a str> {
        let (declarations, _) = self
            .current_file(path)
            .and_then(FileCache::navigation_source)?;
        let candidate = declarations.java_completion_candidate(qualifier)?;
        (!self.has_source_namespace(qualifier)).then_some(candidate)
    }

    /// Resolve imports, evaluated class names, constructor operands and type hints.
    pub fn definition_java_class(&self, path: &str, offset: usize) -> Option<&str> {
        let file = self.current_file(path)?;
        if !file.source.is_char_boundary(offset) {
            return None;
        }
        let (declarations, forms) = file.navigation_source()?;
        if let Some(class) = declarations.java_import_at(offset) {
            return Some(class);
        }
        let resolve =
            |namespace: &str, name: &str, arity| self.navigation_call(namespace, name, arity);
        let Some(form) = symbol_form_at(forms, offset) else {
            let symbol = source_symbol_form_at(forms, offset, SymbolSite::TypeHint)?.symbol()?;
            let class = declarations.java_class_candidate(symbol)?;
            let evaluated = declarations.evaluated_java_class_at(
                forms,
                offset,
                self.has_standard_test(),
                resolve,
            )?;
            return (evaluated == symbol).then_some(class);
        };
        let symbol = form.symbol()?;
        if let Some((qualifier, member)) = symbol.split_once('/') {
            if member.is_empty()
                || member.contains('/')
                || offset - form.span.start.offset > qualifier.len()
                || self.has_source_namespace(qualifier)
                || (declarations.has_top_level_require
                    && self
                        .completion_require_alias(path, qualifier, offset)
                        .is_some())
            {
                return None;
            }
            let class = declarations.java_completion_candidate(qualifier)?;
            let evaluated = declarations.evaluated_reference_at(
                forms,
                offset,
                self.has_standard_test(),
                resolve,
            )?;
            return (evaluated.0 == symbol).then_some(class);
        }
        let spelling = symbol.strip_suffix('.').unwrap_or(symbol);
        let class = declarations.java_class_candidate(spelling)?;
        let evaluated = declarations.evaluated_java_class_at(
            forms,
            offset,
            self.has_standard_test(),
            resolve,
        )?;
        (evaluated == spelling).then_some(class)
    }
}
