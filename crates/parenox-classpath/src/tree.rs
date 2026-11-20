//! Bounded discovery of explicit JVM namespace declarations in a selected JAR.
use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use parenox_frontend::reader::{self, Conditionals, Kind};
use zip::ZipArchive;

use super::{JarSource, MAX_SOURCE_BYTES, read_source};

const MAX_ENTRIES: usize = 4096;
const MAX_NAMESPACES: usize = 512;
const MAX_SCAN_BYTES: u64 = 32 * 1024 * 1024;

pub struct JarNamespace {
    pub name: String,
    pub resource: String,
}

pub fn namespace_resource(namespace: &str, portable: bool) -> Option<String> {
    if namespace
        .split('.')
        .any(|part| part.is_empty() || part.contains(['/', '\\']))
    {
        return None;
    }
    let resource = namespace.replace('-', "_").replace('.', "/");
    let suffix = if portable { "cljc" } else { "clj" };
    Some(format!("{resource}.{suffix}"))
}

pub fn jar_source(jar: &Path, resource: &str) -> Option<JarSource> {
    if resource
        .split('/')
        .any(|part| matches!(part, "" | "." | "..") || part.contains(['\\', '\0']))
    {
        return None;
    }
    let portable = match resource.rsplit_once('.')?.1 {
        "clj" => false,
        "cljc" => true,
        _ => return None,
    };
    let mut archive = ZipArchive::new(File::open(jar).ok()?).ok()?;
    read_source(archive.by_name(resource).ok()?, portable)
}

pub fn jar_namespaces(jar: &Path) -> Option<Vec<JarNamespace>> {
    let mut archive = ZipArchive::new(File::open(jar).ok()?).ok()?;
    let mut namespaces = BTreeMap::<String, String>::new();
    let mut budget = MAX_SCAN_BYTES;
    for index in 0..archive.len().min(MAX_ENTRIES) {
        let Ok(file) = archive.by_index(index) else {
            continue;
        };
        let Some((_, extension)) = file.name().rsplit_once('.') else {
            continue;
        };
        let portable = match extension {
            "clj" => false,
            "cljc" => true,
            _ => continue,
        };
        let size = file.size();
        if size > MAX_SOURCE_BYTES {
            continue;
        }
        if size > budget {
            break;
        }
        budget -= size;
        let resource = file.name().to_owned();
        let Some(source) = read_source(file, portable) else {
            continue;
        };
        let Some(name) = declared_namespace(&source) else {
            continue;
        };
        if namespace_resource(&name, portable).as_deref() != Some(&resource) {
            continue;
        }
        if namespaces.len() == MAX_NAMESPACES && !namespaces.contains_key(&name) {
            continue;
        }
        if portable {
            namespaces.entry(name).or_insert(resource);
        } else {
            namespaces.insert(name, resource);
        }
    }
    Some(
        namespaces
            .into_iter()
            .map(|(name, resource)| JarNamespace { name, resource })
            .collect(),
    )
}

fn declared_namespace(source: &JarSource) -> Option<String> {
    let conditionals = if source.portable {
        Conditionals::Select(":clj")
    } else {
        Conditionals::Reject
    };
    let (read, _) = reader::read_for_check(&source.text, conditionals, None);
    let mut declared = read.forms.iter().filter_map(|form| {
        let Kind::List(items) = &form.without_metadata().kind else {
            return None;
        };
        let [head, namespace, ..] = items.as_slice() else {
            return None;
        };
        matches!(head.symbol(), Some("ns" | "clojure.core/ns"))
            .then(|| namespace.symbol())
            .flatten()
    });
    let name = declared.next()?;
    declared.next().is_none().then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    struct Archive(PathBuf);

    impl Drop for Archive {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn discovers_only_matching_jvm_source_declarations_and_prefers_clj() {
        let jar = Archive(
            std::env::temp_dir().join(format!("parenox-jar-tree-{}.jar", std::process::id())),
        );
        let mut zip = zip::ZipWriter::new(File::create(&jar.0).unwrap());
        for (path, source) in [
            ("sample/tool.cljc", "(ns sample.tool)"),
            ("sample/tool.clj", "(ns sample.tool) (defn hello [] :clj)"),
            (
                "sample/portable.cljc",
                "#?(:clj (ns sample.portable) :cljs (ns sample.web))",
            ),
            ("sample/mismatch.clj", "(ns sample.another)"),
            ("sample/client.cljs", "(ns sample.client)"),
            ("sample/Binary.class", "binary"),
        ] {
            zip.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(source.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        let namespaces = jar_namespaces(&jar.0).unwrap();
        assert_eq!(
            namespaces
                .iter()
                .map(|node| (node.name.as_str(), node.resource.as_str()))
                .collect::<Vec<_>>(),
            [
                ("sample.portable", "sample/portable.cljc"),
                ("sample.tool", "sample/tool.clj")
            ]
        );
        let tool = jar_source(&jar.0, "sample/tool.clj").unwrap();
        assert!(tool.text.contains("hello"));
        assert!(!tool.portable);
        assert!(jar_source(&jar.0, "../escape.clj").is_none());
        assert!(jar_source(&jar.0, "sample/Binary.class").is_none());
    }
}
