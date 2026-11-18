//! Conventional test namespace selection using the same source roots and reader as check.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use parenox::reader::{self, Conditionals, Form, Kind};

use super::namespace_symbol;
use super::report::{Phase, RunError};
use crate::config::deps_paths;
use crate::workspace::source_paths;

pub(super) fn namespaces(root: &Path, aliases: &[String]) -> Result<Vec<String>, RunError> {
    let paths = deps_paths(&root.join("deps.edn"), aliases)
        .map_err(|message| RunError::new(Phase::Project, message))?;
    let sources = source_paths(&paths).map_err(|message| RunError::new(Phase::Project, message))?;
    let mut names = BTreeSet::new();
    for path in sources {
        if !path
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("_test"))
        {
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let source = fs::read_to_string(&path).map_err(|error| {
            RunError::new(Phase::Project, error.to_string())
                .at_file(relative.display().to_string(), None)
        })?;
        let platform = if path
            .extension()
            .is_some_and(|extension| extension == "cljc")
        {
            Conditionals::Select(":clj")
        } else {
            Conditionals::Reject
        };
        let (read, error) = reader::read_for_check(&source, platform, None);
        if let Some(name) = read.forms.iter().find_map(namespace) {
            names.insert(name.to_owned());
        } else {
            let span = error.as_ref().map(|error| error.span);
            let message = error.map_or_else(
                || "Expected a namespace declaration in test source".to_owned(),
                |error| error.message,
            );
            return Err(RunError::new(Phase::Selection, message)
                .at_file(relative.display().to_string(), span));
        }
    }
    if names.is_empty() {
        return Err(RunError::new(
            Phase::Selection,
            "No *_test.clj or *_test.cljc files in the selected source roots",
        ));
    }
    Ok(names.into_iter().collect())
}

fn namespace(form: &Form) -> Option<&str> {
    let Kind::List(items) = &form.without_metadata().kind else {
        return None;
    };
    match items.as_slice() {
        [head, name, ..] if matches!(head.symbol(), Some("ns" | "clojure.core/ns")) => {
            name.symbol().filter(|name| namespace_symbol(name))
        }
        _ => None,
    }
}
