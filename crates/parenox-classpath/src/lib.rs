//! Read local JVM classpath metadata without loading classes or downloading dependencies.
mod classfile;
mod jvm;
mod maven;
mod tree;

pub use classfile::{JavaMember, JavaMemberKind};
pub use jvm::{JavaClassSource, JavaClasspath};
pub use maven::manifest_jars;
pub use tree::{JarNamespace, jar_namespaces, jar_source, namespace_resource};

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

pub struct JarSource {
    pub text: String,
    pub portable: bool,
}

pub struct NamespaceSource<'a> {
    pub archive: &'a Path,
    pub resource: String,
    pub source: JarSource,
}

/// Resolve directories and archives in the order written by Clojure CLI.
pub fn explicit_entries(root: &Path, manifest: &Path) -> io::Result<Vec<PathBuf>> {
    let classpath = fs::read_to_string(manifest)?;
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for entry in env::split_paths(classpath.trim_end_matches(['\r', '\n'])) {
        if entry.as_os_str().is_empty() {
            continue;
        }
        let path = root.join(entry);
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !(metadata.is_dir()
            || metadata.is_file() && path.extension().is_some_and(|ext| ext == "jar"))
        {
            continue;
        }
        let resolved = path.canonicalize()?;
        if seen.insert(resolved.clone()) {
            entries.push(if metadata.is_dir() { path } else { resolved });
        }
    }
    Ok(entries)
}

fn existing_jars(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut visited = BTreeSet::new();
    paths
        .into_iter()
        .filter_map(|path| {
            (path.extension()? == "jar" && path.is_file()).then_some(())?;
            let resolved = path.canonicalize().ok()?;
            visited.insert(resolved.clone()).then_some(resolved)
        })
        .collect()
}

/// Find the JVM source resource in classpath order. Clojure tries .clj
/// before .cljc. Compiled classes and data files are outside this index.
pub fn namespace_source<'a>(jars: &'a [PathBuf], namespace: &str) -> Option<NamespaceSource<'a>> {
    let clj = namespace_resource(namespace, false)?;
    let cljc = namespace_resource(namespace, true)?;
    let mut portable = None;
    for jar in jars {
        let Some(mut archive) = File::open(jar)
            .ok()
            .and_then(|file| ZipArchive::new(file).ok())
        else {
            continue;
        };
        if let Ok(file) = archive.by_name(&clj) {
            return read_source(file, false).map(|source| NamespaceSource {
                archive: jar,
                resource: clj,
                source,
            });
        }
        if portable.is_none()
            && let Ok(file) = archive.by_name(&cljc)
        {
            portable = Some(read_source(file, true).map(|source| NamespaceSource {
                archive: jar,
                resource: cljc.clone(),
                source,
            }));
        }
    }
    portable.flatten()
}

fn read_source(file: zip::read::ZipFile<'_, File>, portable: bool) -> Option<JarSource> {
    Some(JarSource {
        text: read_source_text(file)?,
        portable,
    })
}

fn read_source_text(file: zip::read::ZipFile<'_, File>) -> Option<String> {
    if file.size() > MAX_SOURCE_BYTES || !file.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= MAX_SOURCE_BYTES).then_some(text)
}
