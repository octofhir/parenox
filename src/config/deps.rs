//! Source roots from a project deps.edn, following tools.deps path composition.
use std::collections::BTreeSet;
use std::path::Path;

use parenox::reader::{Form, Kind};

use super::{invalid, read_map};

pub(crate) fn deps_paths(path: &Path, selected: &[String]) -> Result<Vec<String>, String> {
    let manifest = read_map(path)?;
    let aliases = field(path, &manifest, ":aliases")?
        .map(|form| {
            form.map()
                .ok_or_else(|| invalid(path, form.span, ":aliases must be a map"))
        })
        .transpose()?
        .unwrap_or_default();
    let mut added = Vec::new();
    let mut paths = Vec::new();
    let mut replacements = Vec::new();
    let mut replaced = false;
    for name in selected {
        let name = if name.starts_with(':') {
            name.clone()
        } else {
            format!(":{name}")
        };
        let alias = field(path, aliases, &name)?
            .ok_or_else(|| format!("{}: unknown project alias {name}", path.display()))?;
        let settings = alias
            .map()
            .ok_or_else(|| invalid(path, alias.span, &format!("alias {name} must be a map")))?;
        for (key, roots) in [
            (":extra-paths", &mut added),
            (":paths", &mut paths),
            (":replace-paths", &mut replacements),
        ] {
            if let Some(forms) = field(path, settings, key)? {
                roots.extend(path_vector(path, forms)?);
                replaced |= key != ":extra-paths";
            }
        }
    }
    if replaced {
        added.extend(paths);
        added.extend(replacements);
    } else if let Some(base) = field(path, &manifest, ":paths")? {
        added.extend(path_vector(path, base)?);
    }

    let mut resolved = PathAliases {
        path,
        aliases,
        active: BTreeSet::new(),
        expanded: BTreeSet::new(),
        seen: BTreeSet::new(),
        paths: Vec::new(),
    };
    for form in added {
        resolved.expand(form)?;
    }
    if !replaced && field(path, &manifest, ":paths")?.is_none() {
        resolved.add("src")?;
    }
    Ok(resolved.paths)
}

fn field<'a>(path: &Path, map: &'a [Form], name: &str) -> Result<Option<&'a Form>, String> {
    let mut entries = map
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|[key, _]| key.keyword() == Some(name));
    let value = entries.next().map(|[_, value]| value);
    if let Some([key, _]) = entries.next() {
        return Err(invalid(path, key.span, &format!("duplicate key {name}")));
    }
    Ok(value)
}

fn path_vector<'a>(path: &Path, form: &'a Form) -> Result<&'a [Form], String> {
    form.vector()
        .ok_or_else(|| invalid(path, form.span, "expected a vector of source paths"))
}

struct PathAliases<'a> {
    path: &'a Path,
    aliases: &'a [Form],
    active: BTreeSet<&'a str>,
    expanded: BTreeSet<&'a str>,
    seen: BTreeSet<String>,
    paths: Vec<String>,
}

impl<'a> PathAliases<'a> {
    fn expand(&mut self, form: &'a Form) -> Result<(), String> {
        match &form.kind {
            Kind::String(path) if !path.is_empty() => self.add(path),
            Kind::Keyword(name) => {
                if self.expanded.contains(name.as_str()) {
                    return Ok(());
                }
                if self.active.len() >= 128 || !self.active.insert(name) {
                    return Err(invalid(
                        self.path,
                        form.span,
                        &format!("cyclic or too deeply nested path alias {name}"),
                    ));
                }
                let paths = field(self.path, self.aliases, name)?.ok_or_else(|| {
                    invalid(self.path, form.span, &format!("unknown path alias {name}"))
                })?;
                for path in path_vector(self.path, paths)? {
                    self.expand(path)?;
                }
                self.active.remove(name.as_str());
                self.expanded.insert(name);
                Ok(())
            }
            _ => Err(invalid(
                self.path,
                form.span,
                "expected a non-empty path string or alias",
            )),
        }
    }

    fn add(&mut self, root: &str) -> Result<(), String> {
        let root = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(root);
        // tools.deps allows roots such as resources or generated files that do not exist yet.
        if root
            .try_exists()
            .map_err(|err| format!("{}: {err}", root.display()))?
        {
            let root = root.to_string_lossy().into_owned();
            if self.seen.insert(root.clone()) {
                self.paths.push(root);
            }
        }
        Ok(())
    }
}
