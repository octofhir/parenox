//! Documentation for source functions suggested by completion.
use std::fmt::Write;

use super::{FileCache, Session, signatures::source_signatures};

impl Session {
    pub fn completion_namespace<'a>(&'a self, path: &str, qualifier: &'a str) -> Option<&'a str> {
        let (decl, _) = self
            .current_file(path)
            .and_then(FileCache::navigation_source)?;
        Some(decl.qualifier(qualifier))
    }

    /// Recheck the original completion target against the current source snapshot.
    pub fn completion_documentation(
        &self,
        path: &str,
        label: &str,
        expected_namespace: &str,
    ) -> Option<String> {
        let (qualifier, local) = label.split_once('/')?;
        let namespace = self.completion_namespace(path, qualifier)?;
        if namespace != expected_namespace {
            return None;
        }
        let target = self.resolve_source_var(path, namespace, local, None)?;
        let (decl, forms) = target.source.navigation_source()?;
        let parameters = decl.function_parameters(forms, target.name)?;
        let name = format!("{}/{}", target.namespace, target.name);
        let mut result = name.clone();
        for signature in source_signatures(&target.source.source, parameters.clauses) {
            write!(
                result,
                "\n({} [{}])",
                target.name,
                signature.parameters.join(" ")
            )
            .ok()?;
        }
        if let Some(documentation) = parameters.documentation {
            result.push_str("\n\n");
            result.push_str(documentation);
        }
        if let Some(contract) = target
            .source
            .model
            .as_ref()
            .and_then(|model| model.functions.get(&name))
            .and_then(|function| function.contract.as_ref())
        {
            result.push_str("\n\nChecked contracts");
            for signature in contract.signatures.iter() {
                write!(result, "\n{signature}").ok()?;
            }
        }
        Some(result)
    }
}
