//! Read Java class and member metadata from classpath directories, JARs and the local JDK.
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use super::classfile::{Class, JavaMember, is_public_class, read_class};
use super::read_source_text;

const MAX_CLASS_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HIERARCHY_DEPTH: usize = 32;
const MAX_HIERARCHY_CLASSES: usize = 128;
const MAX_OPEN_ARCHIVES: usize = 8;

#[derive(Default)]
pub struct JavaClasspath {
    entries: Vec<PathBuf>,
    jdk: OnceCell<Vec<PathBuf>>,
    packages: RefCell<BTreeMap<String, Vec<String>>>,
}

pub struct JavaClassSource {
    archive: PathBuf,
    resource: String,
}

impl JavaClassSource {
    pub fn archive(&self) -> &Path {
        &self.archive
    }

    pub fn resource(&self) -> &str {
        &self.resource
    }

    pub fn text(&self) -> Option<String> {
        let mut archive = ZipArchive::new(File::open(&self.archive).ok()?).ok()?;
        read_source_text(archive.by_name(&self.resource).ok()?)
    }
}

impl JavaClasspath {
    pub fn new(entries: Vec<PathBuf>) -> Self {
        Self {
            entries,
            ..Self::default()
        }
    }

    pub fn static_members(&self, class: &str) -> Option<Vec<JavaMember>> {
        self.members(class, false)
    }

    pub fn instance_methods(&self, class: &str) -> Option<Vec<JavaMember>> {
        self.members(class, true)
    }

    /// Locate a public top-level class in the source ZIP of its bytecode's JDK.
    pub fn jdk_class_source(&self, class: &str) -> Option<JavaClassSource> {
        let source = self.jdk_source_candidate(&class.replace('.', "/"))?;
        source.text()?;
        Some(source)
    }

    pub fn jdk_source_archive(&self) -> Option<PathBuf> {
        jdk_source_archive(self.jdk.get_or_init(jdk_archives).first()?)
    }

    /// Open the exact resource of a class supplied by the selected JDK.
    pub fn jdk_source_contents(&self, resource: &str) -> Option<String> {
        let (_, path) = resource.split_once('/')?;
        let source = self.jdk_source_candidate(path.strip_suffix(".java")?)?;
        if source.resource != resource {
            return None;
        }
        source.text()
    }

    fn jdk_source_candidate(&self, name: &str) -> Option<JavaClassSource> {
        if name.rsplit('/').next()?.contains('$') {
            return None;
        }
        let (class, Some(module)) = self.class(name, &mut ClassArchives::default())? else {
            return None;
        };
        if !class.public {
            return None;
        }
        let archive = jdk_source_archive(module)?;
        let resource = format!("{}/{name}.java", module.file_stem()?.to_str()?);
        Some(JavaClassSource { archive, resource })
    }

    fn members(&self, class: &str, instance: bool) -> Option<Vec<JavaMember>> {
        let class = class.replace('.', "/");
        let mut pending = VecDeque::from([(class, 0)]);
        let mut visited = BTreeSet::new();
        let mut members = BTreeMap::new();
        let mut archives = ClassArchives::default();
        while let Some((name, depth)) = pending.pop_front() {
            if visited.len() == MAX_HIERARCHY_CLASSES {
                break;
            }
            if depth == MAX_HIERARCHY_DEPTH || !visited.insert(name.clone()) {
                continue;
            }
            let metadata = self.class(&name, &mut archives).map(|(class, _)| class);
            if depth == 0 && !metadata.as_ref().is_some_and(|metadata| metadata.public) {
                return None;
            }
            let Some(metadata) = metadata else { continue };
            let declared = if instance {
                metadata.instance_methods
            } else {
                metadata.static_members
            };
            for member in declared {
                // Java overloads differ by their parameter types. An inherited
                // field or a method with the same parameters is shadowed.
                let parameters = member.parameter_types().map(str::to_owned);
                members
                    .entry((member.name.clone(), parameters))
                    .or_insert(Some(member));
            }
            if let Some(parent) = metadata.superclass {
                pending.push_front((parent, depth + 1));
            }
            if instance {
                // A compiler bridge also overrides the erased ancestor signature.
                // Keep it out of completions while suppressing that inherited method.
                for bridge in metadata.bridges {
                    let parameters = bridge.parameter_types().map(str::to_owned);
                    members.entry((bridge.name, parameters)).or_insert(None);
                }
                pending.extend(
                    metadata
                        .interfaces
                        .into_iter()
                        .map(|name| (name, depth + 1)),
                );
            }
        }
        Some(members.into_values().flatten().collect())
    }

    /// Return short names of public top-level classes in one Java package.
    pub fn package_classes(&self, package: &str, prefix: &str, limit: usize) -> Vec<String> {
        if !package.split('.').all(name_component) {
            return Vec::new();
        }
        let mut packages = self.packages.borrow_mut();
        packages
            .entry(package.to_owned())
            .or_insert_with(|| self.read_package(&package.replace('.', "/")))
            .iter()
            .filter(|name| name.starts_with(prefix))
            .take(limit)
            .cloned()
            .collect()
    }

    fn read_package(&self, package: &str) -> Vec<String> {
        let jdk = self.jdk.get_or_init(jdk_archives);
        let locations = if package == "java" || package.starts_with("java/") {
            jdk.iter().chain(&self.entries)
        } else {
            self.entries.iter().chain(jdk)
        };
        let mut classes = BTreeMap::new();
        for location in locations {
            if location.is_file() {
                archive_package(location, package, &mut classes);
            } else {
                directory_package(location, package, &mut classes);
            }
        }
        classes
            .into_iter()
            .filter_map(|(name, public)| public.then_some(name))
            .collect()
    }

    fn class(&self, name: &str, archives: &mut ClassArchives) -> Option<(Class, Option<&Path>)> {
        if !name.split('/').all(name_component) {
            return None;
        }
        let entry = format!("{name}.class");
        let jdk = |archives: &mut ClassArchives| {
            self.jdk
                .get_or_init(jdk_archives)
                .iter()
                .find_map(|archive| {
                    Some((archives.read(archive, &entry)?, Some(archive.as_path())))
                })
        };
        let classpath = |archives: &mut ClassArchives| {
            let bytes = self.entries.iter().find_map(|location| {
                if location.is_file() {
                    return archives.read(location, &entry);
                }
                let path = location.join(&entry);
                if !path.is_file() {
                    return None;
                }
                limited_bytes(File::open(path).ok()?)
            })?;
            Some((bytes, None))
        };
        let (bytes, jdk_archive) = if name.starts_with("java/") {
            jdk(archives).or_else(|| classpath(archives))
        } else {
            classpath(archives).or_else(|| jdk(archives))
        }?;
        let class = read_class(&bytes)?;
        (class.name == name).then_some((class, jdk_archive))
    }
}

fn name_component(part: &str) -> bool {
    !part.is_empty()
        && part
            .chars()
            .all(|ch| ch == '_' || ch == '$' || ch.is_alphanumeric())
}

fn class_stem(file: &str) -> Option<&str> {
    let stem = file.strip_suffix(".class")?;
    (name_component(stem) && !stem.contains('$')).then_some(stem)
}

fn directory_package(root: &Path, package: &str, classes: &mut BTreeMap<String, bool>) {
    let Ok(entries) = fs::read_dir(root.join(package)) else {
        return;
    };
    for file in entries.filter_map(Result::ok) {
        let basename = file.file_name();
        let Some(name) = basename.to_str().and_then(class_stem) else {
            continue;
        };
        if !classes.contains_key(name)
            && file.file_type().is_ok_and(|kind| kind.is_file())
            && let Some(bytes) = File::open(file.path()).ok().and_then(limited_bytes)
        {
            classes.insert(
                name.to_owned(),
                is_public_class(&bytes, &format!("{package}/{name}")),
            );
        }
    }
}

fn archive_package(root: &Path, package: &str, classes: &mut BTreeMap<String, bool>) {
    let Some(mut archive) = File::open(root)
        .ok()
        .and_then(|file| ZipArchive::new(file).ok())
    else {
        return;
    };
    let prefix = if is_jmod(root) {
        format!("classes/{package}/")
    } else {
        format!("{package}/")
    };
    let candidates: Vec<_> = archive
        .file_names()
        .filter_map(|file| {
            let name = class_stem(file.strip_prefix(&prefix)?)?;
            (!classes.contains_key(name)).then(|| (name.to_owned(), file.to_owned()))
        })
        .collect();
    for (name, file) in candidates {
        if let Some(bytes) = archive_bytes(&mut archive, &file) {
            let public = is_public_class(&bytes, &format!("{package}/{name}"));
            classes.insert(name, public);
        }
    }
}

fn is_jmod(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "jmod")
}

fn jdk_source_archive(module: &Path) -> Option<PathBuf> {
    if !is_jmod(module) {
        return None;
    }
    std::path::absolute(module.parent()?.parent()?.join("lib/src.zip")).ok()
}

fn limited_bytes(reader: impl Read) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_CLASS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_CLASS_BYTES).then_some(bytes)
}

/// Ancestors often live in the same JAR or JDK module. Reuse its directory
/// for the duration of one lookup and bound the number of open files.
#[derive(Default)]
struct ClassArchives {
    open: VecDeque<(PathBuf, ZipArchive<File>)>,
}

impl ClassArchives {
    fn read(&mut self, path: &Path, entry: &str) -> Option<Vec<u8>> {
        let position = self.open.iter().position(|(archive, _)| archive == path);
        let (path, mut archive) = if let Some(index) = position {
            self.open.remove(index)?
        } else {
            let archive = ZipArchive::new(File::open(path).ok()?).ok()?;
            (path.to_owned(), archive)
        };
        let bytes = if is_jmod(&path) {
            archive_bytes(&mut archive, &format!("classes/{entry}"))
        } else {
            archive_bytes(&mut archive, entry)
        };
        self.open.push_front((path, archive));
        self.open.truncate(MAX_OPEN_ARCHIVES);
        bytes
    }
}

fn archive_bytes(archive: &mut ZipArchive<File>, entry: &str) -> Option<Vec<u8>> {
    let file = archive.by_name(entry).ok()?;
    if !file.is_file() || file.size() > MAX_CLASS_BYTES {
        return None;
    }
    limited_bytes(file)
}

fn jdk_archives() -> Vec<PathBuf> {
    let java = format!("java{}", env::consts::EXE_SUFFIX);
    let home = env::var_os("JAVA_HOME").map(PathBuf::from);
    let path = env::var_os("PATH");
    let java_home = path.iter().flat_map(env::split_paths).filter_map(|bin| {
        let executable = bin.join(&java).canonicalize().ok()?;
        Some(executable.parent()?.parent()?.to_path_buf())
    });
    home.into_iter()
        .chain(java_home)
        .find_map(|home| {
            let mut modules: Vec<_> = fs::read_dir(home.join("jmods"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "jmod")
                        && path.is_file()
                })
                .collect();
            modules.sort_unstable();
            if !modules.is_empty() {
                return Some(modules);
            }
            ["lib/rt.jar", "jre/lib/rt.jar"]
                .iter()
                .map(|path| home.join(path))
                .find(|path| path.is_file())
                .map(|path| vec![path])
        })
        .unwrap_or_default()
}
