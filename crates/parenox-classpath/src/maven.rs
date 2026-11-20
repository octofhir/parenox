//! The locally cached portion of direct tools.deps coordinates.
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use parenox_frontend::reader::{self, Form};

use super::existing_jars;

pub fn manifest_jars(manifest: &Path, aliases: &[String]) -> Vec<PathBuf> {
    let forms = read_map(manifest).unwrap_or_default();
    let entries = forms.first().and_then(Form::map).unwrap_or_default();
    let root = manifest.parent().unwrap_or_else(|| Path::new("."));
    let repo = field(entries, ":mvn/local-repo")
        .and_then(string)
        .map(|location| root.join(location))
        .or_else(|| {
            env::var_os("HOME")
                .or_else(|| env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".m2/repository"))
        });
    let mut deps = coordinates(entries, ":deps");
    let mut extra = BTreeMap::new();
    let mut replacements = None;
    let mut overrides = BTreeMap::new();
    if let Some(alias_options) = field(entries, ":aliases").and_then(Form::map) {
        for alias in aliases {
            let keyword = format!(":{}", alias.trim_start_matches(':'));
            let Some(options) = field(alias_options, &keyword).and_then(Form::map) else {
                continue;
            };
            if field(options, ":replace-deps").is_some() || field(options, ":deps").is_some() {
                let replace = replacements.get_or_insert_with(BTreeMap::new);
                replace.extend(coordinates(options, ":deps"));
                replace.extend(coordinates(options, ":replace-deps"));
            }
            extra.extend(coordinates(options, ":extra-deps"));
            overrides.extend(coordinates(options, ":override-deps"));
        }
    }
    if let Some(replacements) = replacements {
        deps = replacements;
    }
    deps.extend(extra);
    let core = if deps.contains_key("org.clojure/clojure") {
        None
    } else {
        install_core_dependency()
    };
    if core.is_some()
        && let Some(override_core) = overrides.get("org.clojure/clojure")
    {
        deps.entry("org.clojure/clojure").or_insert(override_core);
    }
    let fallback = if deps.contains_key("org.clojure/clojure") {
        None
    } else {
        core.and_then(|version| maven_jar(repo.as_deref()?, "org.clojure/clojure", &version))
    };
    existing_jars(
        deps.into_iter()
            .filter_map(|(name, options)| {
                let options = overrides.get(name).copied().unwrap_or(options);
                let map = options.map()?;
                if let Some(local) = field(map, ":local/root").and_then(string) {
                    Some(root.join(local))
                } else {
                    maven_jar(
                        repo.as_deref()?,
                        name,
                        field(map, ":mvn/version").and_then(string)?,
                    )
                }
            })
            .chain(fallback),
    )
}

fn maven_jar(repo: &Path, name: &str, version: &str) -> Option<PathBuf> {
    let (group, artifact) = name.split_once('/')?;
    let (artifact, classifier) = artifact
        .split_once('$')
        .map_or((artifact, None), |(artifact, classifier)| {
            (artifact, Some(classifier))
        });
    if !group.split('.').all(valid_component)
        || !valid_component(artifact)
        || !valid_component(version)
        || !classifier.is_none_or(valid_component)
    {
        return None;
    }
    let jar = classifier.map_or_else(
        || format!("{artifact}-{version}.jar"),
        |classifier| format!("{artifact}-{version}-{classifier}.jar"),
    );
    Some(
        group
            .split('.')
            .fold(repo.to_path_buf(), |path, part| path.join(part))
            .join(artifact)
            .join(version)
            .join(jar),
    )
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
}

fn read_map(path: &Path) -> Option<Vec<Form>> {
    let forms = reader::read_edn(&fs::read_to_string(path).ok()?).ok()?;
    (matches!(forms.as_slice(), [entry] if entry.map().is_some())).then_some(forms)
}

fn coordinates<'a>(map: &'a [Form], name: &str) -> BTreeMap<&'a str, &'a Form> {
    field(map, name)
        .and_then(Form::map)
        .into_iter()
        .flat_map(|entries| entries.as_chunks::<2>().0)
        .filter_map(|[name, options]| Some((name.symbol()?, options)))
        .collect()
}

fn field<'a>(map: &'a [Form], name: &str) -> Option<&'a Form> {
    map.as_chunks::<2>()
        .0
        .iter()
        .find_map(|[key, value]| (key.keyword() == Some(name)).then_some(value))
}

fn string(form: &Form) -> Option<&str> {
    match &form.without_metadata().kind {
        reader::Kind::String(value) => Some(value),
        _ => None,
    }
}

fn install_core_dependency() -> Option<String> {
    // Recognize the CLI's installed EDN. Never run a tool found on PATH.
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        let Ok(cli) = directory.join("clojure").canonicalize() else {
            continue;
        };
        let Some(prefix) = cli.parent().and_then(Path::parent) else {
            continue;
        };
        for config in [prefix.join("deps.edn"), prefix.join("lib/clojure/deps.edn")] {
            if let Some(forms) = read_map(&config)
                && let Some(options) =
                    coordinates(forms[0].map()?, ":deps").get("org.clojure/clojure")
                && let Some(version) = options
                    .map()
                    .and_then(|map| field(map, ":mvn/version"))
                    .and_then(string)
            {
                return Some(version.into());
            }
        }
        break;
    }
    None
}
