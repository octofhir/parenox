//! Lazy Clojure project tree over indexed workspace source.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use parenox_analysis::SourceSymbolKind::{self, *};
use parenox_analysis::{Session, SourceSymbol};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::symbols::Candidate;
use crate::text::{Lines, decode_uri_path, file_path, file_uri};

const PROJECT: u8 = 1;
const SOURCE_PATH: u8 = 2;
const LIBRARY: u8 = 3;
const JAR: u8 = 4;
const NAMESPACE: u8 = 5;
const CLASS: u8 = 6;
const FUNCTION: u8 = 7;
const VARIABLE: u8 = 8;
const INTERFACE: u8 = 9;
const EXTERNAL_ID: &str = "external-dependencies";

#[derive(Deserialize)]
pub struct Leaf {
    name: String,
    #[serde(rename = "type")]
    kind: u8,
    #[serde(rename = "final")]
    terminal: bool,
    id: Option<String>,
    uri: Option<String>,
}

impl Leaf {
    pub fn needs_index(&self) -> bool {
        !self.terminal
            && (self.kind == SOURCE_PATH
                || (self.kind == NAMESPACE
                    && self
                        .uri
                        .as_deref()
                        .is_some_and(|uri| !uri.starts_with("jar:"))))
    }
}

pub fn nodes<'a>(
    project_root: &Path,
    source_roots: &[PathBuf],
    jar_paths: &[PathBuf],
    candidates: impl Iterator<Item = Candidate<'a>>,
    query: Option<Leaf>,
) -> Option<Value> {
    let roots: BTreeMap<_, _> = source_roots
        .iter()
        .map(|path| (relative_name(project_root, path), path))
        .collect();
    let Some(query) = query else {
        let mut nodes: Vec<_> = roots
            .keys()
            .map(|name| json!({"name": name, "type": SOURCE_PATH, "final": false}))
            .collect();
        nodes.push(json!({
            "name": "External dependencies", "type": LIBRARY, "id": EXTERNAL_ID, "final": false,
        }));
        return Some(json!({
            "name": project_root.file_name().unwrap_or_default().to_string_lossy(),
            "type": PROJECT, "nodes": nodes,
        }));
    };
    if query.terminal {
        return None;
    }
    match query.kind {
        SOURCE_PATH => {
            let root = roots.get(&query.name)?;
            let namespaces: BTreeSet<_> = candidates
                .filter(|candidate| {
                    candidate.symbol.kind == Namespace
                        && Path::new(candidate.path).starts_with(root)
                })
                .map(|candidate| (candidate.namespace, document_uri(&candidate)))
                .collect();
            let nodes: Vec<_> = namespaces
                .into_iter()
                .map(|(name, uri)| json!({"name": name, "type": NAMESPACE, "uri": uri, "final": false}))
                .collect();
            Some(json!({"name": query.name, "type": SOURCE_PATH, "nodes": nodes}))
        }
        NAMESPACE => {
            let uri = query.uri.as_deref()?;
            if uri.starts_with("jar:") {
                return archived_namespace(&query.name, uri, jar_paths);
            }
            let path = file_path(uri)?;
            let snapshot: Vec<_> = candidates
                .filter(|candidate| {
                    candidate.namespace == query.name
                        && (Path::new(candidate.path) == path || candidate.editor_uri == Some(uri))
                        && source_roots
                            .iter()
                            .any(|root| Path::new(candidate.path).starts_with(root))
                })
                .collect();
            let (source, uri) = snapshot
                .iter()
                .find(|entry| entry.symbol.kind == Namespace)
                .map(|entry| (entry.source, document_uri(entry)))?;
            let symbols = snapshot.into_iter().map(|entry| entry.symbol).collect();
            Some(namespace_branch(&query.name, &uri, source, symbols))
        }
        LIBRARY if query.id.as_deref() == Some(EXTERNAL_ID) => {
            let mut archives: Vec<_> = jar_paths
                .iter()
                .filter_map(|path| Some((jar_uri(path), path.file_name()?.to_string_lossy())))
                .collect();
            archives.sort_unstable();
            let nodes: Vec<_> = archives.into_iter().map(|(uri, name)| {
                json!({"name": name, "type": JAR, "uri": uri, "detail": uri, "final": false})
            }).collect();
            Some(json!({"name": query.name, "type": LIBRARY, "id": EXTERNAL_ID, "nodes": nodes}))
        }
        JAR => {
            let uri = query.uri.as_deref()?;
            let jar = selected_jar(jar_paths, uri)?;
            let namespaces = parenox_classpath::jar_namespaces(jar)?;
            let nodes: Vec<_> = namespaces
                .into_iter()
                .map(|item| {
                    json!({"name": item.name, "type": NAMESPACE,
                       "uri": resource_uri(uri, &item.resource), "final": false})
                })
                .collect();
            Some(
                json!({"name": jar.file_name()?.to_string_lossy(), "type": JAR,
                        "uri": uri, "detail": uri, "nodes": nodes}),
            )
        }
        _ => None,
    }
}

fn selected_jar<'a>(jars: &'a [PathBuf], uri: &str) -> Option<&'a Path> {
    jars.iter()
        .find(|path| jar_uri(path) == uri)
        .map(PathBuf::as_path)
}

fn jar_uri(path: &Path) -> String {
    format!("jar:{}", file_uri(&path.to_string_lossy()))
}

fn resource_uri(archive: &str, resource: &str) -> String {
    let encoded = file_uri(&format!("/{resource}"));
    format!(
        "{archive}!/{}",
        encoded.strip_prefix("file:///").unwrap_or(resource)
    )
}

pub(crate) fn jar_resource_uri(archive: &Path, resource: &str) -> String {
    resource_uri(&jar_uri(archive), resource)
}

fn selected_resource<'a>(jars: &'a [PathBuf], uri: &str) -> Option<(&'a Path, String)> {
    let (archive, encoded) = uri.rsplit_once("!/")?;
    let jar = selected_jar(jars, archive)?;
    let resource = decode_uri_path(encoded)?;
    (resource_uri(archive, &resource) == uri).then_some((jar, resource))
}

/// Read Clojure source from a URI under a JAR selected by this project.
pub fn dependency_contents(uri: &str, jars: &[PathBuf]) -> Option<String> {
    let (jar, resource) = selected_resource(jars, uri)?;
    Some(parenox_classpath::jar_source(jar, &resource)?.text)
}

fn archived_namespace(name: &str, uri: &str, jars: &[PathBuf]) -> Option<Value> {
    let (jar, resource) = selected_resource(jars, uri)?;
    let source = parenox_classpath::jar_source(jar, &resource)?;
    if parenox_classpath::namespace_resource(name, source.portable).as_deref() != Some(&resource) {
        return None;
    }
    let mut session = Session::default();
    session.check(&resource, &source.text);
    let (namespace, symbols) = session.document_symbols(&resource)?;
    if namespace != name || !symbols.iter().any(|symbol| symbol.kind == Namespace) {
        return None;
    }
    Some(namespace_branch(name, uri, &source.text, symbols))
}

fn namespace_branch(
    name: &str,
    uri: &str,
    source: &str,
    mut symbols: Vec<SourceSymbol<'_>>,
) -> Value {
    symbols.sort_unstable_by_key(|symbol| symbol.selection.start.offset);
    let lines = Lines::new(source);
    let mut protocol: Option<(usize, Value)> = None;
    let mut vars = Vec::new();
    let mut specs = Vec::new();
    for symbol in symbols {
        let inside_protocol = protocol
            .as_ref()
            .is_some_and(|(end, _)| symbol.range.start.offset < *end);
        if !inside_protocol && let Some((_, node)) = protocol.take() {
            vars.push(node);
        }
        let Some(kind) = symbol_kind(symbol.kind, inside_protocol) else {
            continue;
        };
        let name = if symbol.kind == Spec {
            spec_name(symbol.name)
        } else {
            symbol.name
        };
        let Some(range) = lines.range(symbol.selection) else {
            continue;
        };
        let node = json!({"name": name, "type": kind, "uri": uri, "range": range, "final": true});
        match symbol.kind {
            Protocol => protocol = Some((symbol.range.end.offset, node)),
            Spec => specs.push(node),
            _ => vars.push(node),
        }
    }
    vars.extend(protocol.map(|(_, node)| node));
    vars.extend(specs);
    json!({"name": name, "type": NAMESPACE, "uri": uri, "nodes": vars})
}

fn document_uri(candidate: &Candidate<'_>) -> String {
    candidate
        .editor_uri
        .map_or_else(|| file_uri(candidate.path), str::to_owned)
}

fn symbol_kind(kind: SourceSymbolKind, inside_protocol: bool) -> Option<u8> {
    Some(match kind {
        Function | Macro | Test | Spec => FUNCTION,
        Method if inside_protocol => FUNCTION,
        Type => CLASS,
        Protocol => INTERFACE,
        Variable => VARIABLE,
        Namespace | Method => return None,
    })
}

fn spec_name(spelling: &str) -> &str {
    let spelling = spelling.trim_start_matches(':');
    spelling.split_once('/').map_or(spelling, |(_, name)| name)
}

fn relative_name(root: &Path, path: &Path) -> String {
    let mut root = root.components();
    let mut rest = path.components();
    while let (Some(from), Some(to)) = (root.clone().next(), rest.clone().next()) {
        if from != to {
            if matches!(from, Component::Prefix(_) | Component::RootDir)
                || matches!(to, Component::Prefix(_) | Component::RootDir)
            {
                return path.to_string_lossy().into_owned();
            }
            break;
        }
        root.next();
        rest.next();
    }
    let mut relative = PathBuf::new();
    for _ in root {
        relative.push("..");
    }
    relative.extend(rest);
    relative.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_resources_resolve_only_under_selected_jar_uris() {
        let dir = std::env::temp_dir();
        let jars = [dir.join("project dependencies.jar")];
        let selected = jar_uri(&jars[0]);
        let uri = resource_uri(&selected, "project/tool.clj");
        assert_eq!(
            selected_resource(&jars, &uri),
            Some((jars[0].as_path(), "project/tool.clj".to_owned()))
        );
        let forged = resource_uri(&jar_uri(&dir.join("unrelated.jar")), "project/tool.clj");
        assert!(selected_resource(&jars, &forged).is_none());
        assert!(dependency_contents(&forged, &jars).is_none());
        assert!(selected_resource(&jars, &format!("{selected}!/project%2Ftool.clj")).is_none());
    }
}
