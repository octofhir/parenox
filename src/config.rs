//! Inert EDN project settings for command-line checks.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use parenox::reader::{self, Form, Kind, Span};

pub(super) const DEFAULT_PATH: &str = "parenox.edn";

#[derive(Default)]
pub(super) struct ProjectConfig {
    pub paths: Vec<String>,
    pub externs: Vec<String>,
    pub dependency_externs: Vec<String>,
}

impl ProjectConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let source =
            fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        let forms = reader::read(&source).map_err(|err| invalid(path, err.span, &err.message))?;
        let [
            Form {
                kind: Kind::Map(entries),
                ..
            },
        ] = forms.as_slice()
        else {
            return Err(format!(
                "{}: expected one EDN configuration map",
                path.display()
            ));
        };
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let mut config = Self::default();
        let mut seen = BTreeSet::new();
        for [key, value] in entries.as_chunks::<2>().0 {
            let Kind::Keyword(name) = &key.kind else {
                return Err(invalid(
                    path,
                    key.span,
                    "configuration keys must be keywords",
                ));
            };
            let slot = match name.as_str() {
                ":paths" => &mut config.paths,
                ":externs" => &mut config.externs,
                ":dependency-externs" => &mut config.dependency_externs,
                _ => {
                    return Err(invalid(
                        path,
                        key.span,
                        &format!("unknown configuration key {name}"),
                    ));
                }
            };
            if !seen.insert(name) {
                return Err(invalid(
                    path,
                    key.span,
                    &format!("duplicate configuration key {name}"),
                ));
            }
            let Kind::Vector(values) = &value.kind else {
                return Err(invalid(
                    path,
                    value.span,
                    &format!("{name} must be a vector of paths"),
                ));
            };
            for entry in values {
                if let Kind::String(value) = &entry.kind
                    && !value.is_empty()
                {
                    slot.push(parent.join(value).to_string_lossy().into_owned());
                } else {
                    return Err(invalid(
                        path,
                        entry.span,
                        &format!("{name} requires non-empty path strings"),
                    ));
                }
            }
        }
        Ok(config)
    }
}

fn invalid(path: &Path, span: Span, message: &str) -> String {
    format!(
        "{}:{}:{}: {message}",
        path.display(),
        span.start.line,
        span.start.column
    )
}
