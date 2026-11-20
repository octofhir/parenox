//! Cache editor queries backed by local dependency artifacts.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use parenox_analysis::{
    ArchiveDeclarations, Session, SourceExport, VarDocumentation, namespace_documentation,
    namespace_exports,
};
use parenox_classpath::{JavaClassSource, JavaClasspath, JavaMember, namespace_source};
use serde_json::{Value, json};

use crate::{
    project_tree::{self, jar_resource_uri},
    text::{LineIndex, Position, Range, decode_uri_path},
};

type Documentation = BTreeMap<(String, String), Option<Rc<VarDocumentation>>>;

struct ArchiveDefinitionSource {
    uri: String,
    text: String,
    lines: LineIndex,
    declarations: ArchiveDeclarations,
}

struct JavaDefinitionSource {
    uri: String,
    source: JavaClassSource,
}

#[derive(Default)]
pub struct DependencyCompletions {
    jars: Vec<PathBuf>,
    namespaces: RefCell<BTreeMap<String, Vec<SourceExport>>>,
    documentation: RefCell<Documentation>,
    definitions: RefCell<BTreeMap<String, Option<ArchiveDefinitionSource>>>,
    java: JavaClasspath,
    java_definitions: RefCell<BTreeMap<String, Option<JavaDefinitionSource>>>,
    static_members: RefCell<JavaMembers>,
    instance_methods: RefCell<JavaMembers>,
}

type JavaMembers = BTreeMap<String, Option<Vec<JavaMember>>>;

impl DependencyCompletions {
    pub fn new(classpath: Vec<PathBuf>) -> Self {
        Self {
            jars: classpath
                .iter()
                .filter(|path| {
                    path.extension().is_some_and(|extension| extension == "jar") && path.is_file()
                })
                .cloned()
                .collect(),
            java: JavaClasspath::new(classpath),
            ..Self::default()
        }
    }

    pub fn jars(&self) -> &[PathBuf] {
        &self.jars
    }

    pub(crate) fn documentation(
        &self,
        namespace: &str,
        name: &str,
    ) -> Option<Rc<VarDocumentation>> {
        if self.jars.is_empty() {
            return None;
        }
        self.documentation
            .borrow_mut()
            .entry((namespace.into(), name.into()))
            .or_insert_with(|| {
                let source = namespace_source(&self.jars, namespace)?.source;
                namespace_documentation(&source.text, namespace, name, source.portable).map(Rc::new)
            })
            .clone()
    }

    /// Resolve JVM sources selected by the project's classpath or local JDK.
    pub fn definition(&self, session: &Session, path: &str, offset: usize) -> Option<Value> {
        self.clojure_definition(session, path, offset)
            .or_else(|| self.java_definition(session, path, offset))
    }

    pub fn dependency_contents(&self, uri: &str) -> Option<String> {
        if !uri.ends_with(".java") {
            return project_tree::dependency_contents(uri, &self.jars);
        }
        let definitions = self.java_definitions.borrow();
        if let Some(definition) = definitions
            .values()
            .flatten()
            .find(|source| source.uri == uri)
        {
            return definition.source.text();
        }
        let (_, encoded) = uri.rsplit_once("!/")?;
        let resource = decode_uri_path(encoded)?;
        let archive = self.java.jdk_source_archive()?;
        if jar_resource_uri(&archive, &resource) != uri {
            return None;
        }
        self.java.jdk_source_contents(&resource)
    }

    fn java_definition(&self, session: &Session, path: &str, offset: usize) -> Option<Value> {
        let class = session.definition_java_class(path, offset)?;
        let mut definitions = self.java_definitions.borrow_mut();
        let source = definitions
            .entry(class.to_owned())
            .or_insert_with(|| {
                let source = self.java.jdk_class_source(class)?;
                let uri = jar_resource_uri(source.archive(), source.resource());
                Some(JavaDefinitionSource { uri, source })
            })
            .as_ref()?;
        let start = Position {
            line: 0,
            character: 0,
        };
        let range = Range { start, end: start };
        Some(json!({"uri":source.uri,"range":range}))
    }

    fn clojure_definition(&self, session: &Session, path: &str, offset: usize) -> Option<Value> {
        if self.jars.is_empty() {
            return None;
        }
        let (namespace, name) = match session.hover_archive_var(path, offset) {
            Some((namespace, name)) => (namespace, Some(name)),
            None => (session.archive_namespace_at(path, offset)?, None),
        };
        let mut definitions = self.definitions.borrow_mut();
        let source = definitions
            .entry(namespace.to_owned())
            .or_insert_with(|| {
                let located = namespace_source(&self.jars, namespace)?;
                let uri = jar_resource_uri(located.archive, &located.resource);
                let source = located.source;
                let declarations =
                    ArchiveDeclarations::read(&source.text, namespace, source.portable)?;
                Some(ArchiveDefinitionSource {
                    uri,
                    lines: LineIndex::new(&source.text),
                    text: source.text,
                    declarations,
                })
            })
            .as_ref()?;
        let span = source.declarations.definition(name)?;
        let range = source.lines.range(&source.text, span)?;
        Some(json!({"uri":source.uri,"range":range}))
    }

    pub(super) fn matching(
        &self,
        namespace: &str,
        prefix: &str,
        limit: usize,
    ) -> Vec<SourceExport> {
        if self.jars.is_empty() {
            return Vec::new();
        }
        let mut namespaces = self.namespaces.borrow_mut();
        let exports = namespaces.entry(namespace.to_owned()).or_insert_with(|| {
            namespace_source(&self.jars, namespace)
                .and_then(|located| {
                    namespace_exports(&located.source.text, namespace, located.source.portable)
                })
                .unwrap_or_default()
        });
        exports
            .iter()
            .filter(|export| export.name.starts_with(prefix))
            .take(limit)
            .cloned()
            .collect()
    }

    pub(super) fn matching_java(
        &self,
        class: &str,
        prefix: &str,
        limit: usize,
    ) -> Option<Vec<JavaMember>> {
        let instance = prefix.strip_prefix('.');
        let mut classes = if instance.is_some() {
            self.instance_methods.borrow_mut()
        } else {
            self.static_members.borrow_mut()
        };
        let members = classes.entry(class.to_owned()).or_insert_with(|| {
            if instance.is_some() {
                self.java.instance_methods(class)
            } else {
                self.java.static_members(class)
            }
        });
        let prefix = instance.unwrap_or(prefix);
        Some(
            members
                .as_ref()?
                .iter()
                .filter(|member| member.name.starts_with(prefix))
                .take(limit)
                .cloned()
                .collect(),
        )
    }

    pub(super) fn matching_java_classes(
        &self,
        package: &str,
        prefix: &str,
        limit: usize,
    ) -> Vec<String> {
        self.java.package_classes(package, prefix, limit)
    }
}
