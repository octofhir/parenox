//! Complete symbols at the cursor using the lossless reader and project index.
mod dependencies;
mod import;
mod ns_form;
mod require;

use std::ops::Range;

pub use dependencies::DependencyCompletions;
use parenox_analysis::reader;
use parenox_analysis::syntax::SyntaxKind;
use parenox_analysis::{ArgumentKeyword, Session};
use parenox_classpath::JavaMemberKind;
use serde_json::{Value, json};

use super::text::Lines;

struct Core {
    name: &'static str,
    detail: &'static str,
    template: Option<&'static str>,
}

const CORE: &[Core] = &[
    Core {
        name: "ns",
        detail: "(ns name & clauses)",
        template: Some("${1:name}$0"),
    },
    Core {
        name: "def",
        detail: "(def name init)",
        template: Some("${1:name} $0"),
    },
    Core {
        name: "defn",
        detail: "(defn name [params*] body)",
        template: Some("${1:name} [${2:args}] $0"),
    },
    Core {
        name: "defn-",
        detail: "(defn- name [params*] body) — private",
        template: Some("${1:name} [${2:args}] $0"),
    },
    Core {
        name: "defmacro",
        detail: "(defmacro name [params*] body)",
        template: Some("${1:name} [${2:args}] $0"),
    },
    Core {
        name: "fn",
        detail: "(fn [params*] body)",
        template: Some("[${1:args}] $0"),
    },
    Core {
        name: "let",
        detail: "(let [bindings*] body)",
        template: Some("[${1:name} ${2:value}] $0"),
    },
    Core {
        name: "loop",
        detail: "(loop [bindings*] body)",
        template: Some("[${1:name} ${2:value}] $0"),
    },
    Core {
        name: "if",
        detail: "(if test then else)",
        template: Some("${1:test} ${2:then} ${3:else}$0"),
    },
    Core {
        name: "when",
        detail: "(when test body)",
        template: Some("${1:test} $0"),
    },
    Core {
        name: "cond",
        detail: "(cond & test-expr-pairs)",
        template: None,
    },
    Core {
        name: "case",
        detail: "(case expr & clauses)",
        template: None,
    },
    Core {
        name: "try",
        detail: "(try body* catch* finally?)",
        template: None,
    },
    Core {
        name: "->",
        detail: "(-> x & forms) — thread as first argument",
        template: None,
    },
    Core {
        name: "->>",
        detail: "(->> x & forms) — thread as last argument",
        template: None,
    },
    Core {
        name: "map",
        detail: "(map f) or (map f coll & colls)",
        template: None,
    },
    Core {
        name: "mapv",
        detail: "(mapv f coll & colls)",
        template: None,
    },
    Core {
        name: "filter",
        detail: "(filter pred) or (filter pred coll)",
        template: None,
    },
    Core {
        name: "reduce",
        detail: "(reduce f coll) or (reduce f init coll)",
        template: None,
    },
    Core {
        name: "assoc",
        detail: "(assoc m key val & kvs)",
        template: None,
    },
    Core {
        name: "get",
        detail: "(get m key) or (get m key not-found)",
        template: None,
    },
];

struct Site {
    range: Range<usize>,
    kind: SyntaxKind,
    empty_call: Option<bool>, // Some(true) if the head is followed by a closing paren.
    require: Option<require::Target>,
    import: Option<import::Target>,
}

impl Site {
    fn at(source: &str, offset: usize) -> Option<Self> {
        use SyntaxKind::*;
        let node = reader::parse(source).syntax();
        let token = node
            .token_at_offset(u32::try_from(offset).ok()?.into())
            .find(|token| {
                matches!(token.kind(), Symbol | Keyword)
                    && usize::from(token.text_range().start()) < offset
            })?;
        let kind = token.kind();
        let parent = token.parent()?;
        if parent.ancestors().any(|node| {
            node.kind() == Discard
                || (kind == Symbol && matches!(node.kind(), Quote | SyntaxQuote | VarQuote))
        }) {
            return None;
        }
        let empty_call = if matches!(parent.kind(), List | Error) {
            let mut forms = parent
                .children_with_tokens()
                .filter(|node| !node.kind().is_trivia());
            if forms.next().is_some_and(|node| node.kind() == OpenParen)
                && forms.next().and_then(|node| node.into_token()).as_ref() == Some(&token)
            {
                match forms.next().map(|node| node.kind()) {
                    None => Some(false),
                    Some(CloseParen) => Some(true),
                    _ => None,
                }
            } else {
                None
            }
        } else {
            None
        };
        let bytes = token.text_range();
        Some(Self {
            range: usize::from(bytes.start())..usize::from(bytes.end()),
            kind,
            empty_call,
            require: require::target(&token),
            import: import::target(&token),
        })
    }
}

pub fn complete(
    session: &Session,
    path: &str,
    uri: &str,
    lines: &Lines<'_>,
    offset: usize,
    snippets: bool,
    dependencies: &DependencyCompletions,
) -> Option<Value> {
    const LIMIT: usize = 60;
    let source = lines.source();
    let site = Site::at(source, offset)?;
    let prefix = &source[site.range.start..offset];
    let range =
        json!({"start": lines.position(site.range.start)?, "end": lines.position(site.range.end)?});
    let mut items: Vec<_> = if let Some(target) = site.import {
        import::complete(target, prefix, &range, LIMIT + 1, dependencies)
    } else if let Some(target) = site.require {
        require::complete(
            session,
            path,
            target,
            prefix,
            &range,
            LIMIT + 1,
            dependencies,
        )
    } else if site.kind == SyntaxKind::Keyword {
        if let Some(values) =
            session.complete_argument_keyword(path, site.range.clone(), offset, LIMIT + 1)
        {
            values
                .into_iter()
                .map(|suggestion| match suggestion {
                    ArgumentKeyword::Value(value) => {
                        json!({"label":value,"kind":20,"detail":"Allowed argument value",
                               "textEdit":{"range":range,"newText":value}})
                    }
                    ArgumentKeyword::Field {
                        keyword,
                        required,
                        value_type,
                    } => {
                        let role = if required {
                            "Required key"
                        } else {
                            "Optional key"
                        };
                        let detail = value_type
                            .map_or_else(|| role.to_owned(), |ty| format!("{role} · {ty}"));
                        json!({"label":keyword,"kind":5,"detail":detail,
                               "textEdit":{"range":range,"newText":keyword}})
                    }
                })
                .collect()
        } else {
            if !prefix.starts_with("::") && !prefix.contains('/') {
                return None;
            }
            session.complete_spec_key(path, prefix, LIMIT + 1).into_iter().map(|key| {
                json!({"label":key,"kind":14,"detail":"Project spec","textEdit":{"range":range,"newText":key}})
            }).collect()
        }
    } else if let Some((qualifier, member)) = prefix.split_once('/')
        && qualifier != "clojure.core"
    {
        let require_alias = session.completion_require_alias(path, qualifier, offset);
        let java = if require_alias.is_none() {
            session
                .completion_java_class(path, qualifier)
                .and_then(|class| dependencies.matching_java(class, member, LIMIT + 1))
        } else {
            None
        };
        if let Some(members) = java {
            let dot = if member.starts_with('.') { "." } else { "" };
            members
                .into_iter()
                .map(|member| {
                    let label = format!("{qualifier}/{dot}{}", member.name);
                    let kind = match member.kind {
                        JavaMemberKind::Method => 2,
                        JavaMemberKind::Field => 5,
                        JavaMemberKind::Constant => 21,
                    };
                    json!({"label":label,"kind":kind,"detail":member.detail(),
                    "textEdit":{"range":range,"newText":label}})
                })
                .collect()
        } else if let Some(namespace) =
            session.completion_archive_namespace(path, qualifier, offset)
        {
            dependencies
                .matching(namespace, member, LIMIT + 1)
                .into_iter()
                .map(|export| {
                    let label = format!("{qualifier}/{}", export.name);
                    json!({"label":label,"kind":if export.callable {3} else {6},"detail":namespace,
                    "textEdit":{"range":range,"newText":label}})
                })
                .collect()
        } else {
            let namespace = session.completion_namespace_at(path, qualifier, offset);
            session
                .complete_qualified_var_at(path, prefix, offset, LIMIT + 1)
                .into_iter()
                .map(|name| {
                    let mut item =
                        json!({"label":name,"kind":6,"textEdit":{"range":range,"newText":name}});
                    if let Some(namespace) = namespace {
                        item["data"] =
                            json!({"parenoxSource":{"uri":uri,"label":name,"namespace":namespace}});
                        if require_alias.is_some() {
                            item["data"]["parenoxSource"]["offset"] = json!(offset);
                        }
                    }
                    item
                })
                .collect()
        }
    } else {
        let qualified = prefix.strip_prefix("clojure.core/");
        let locals = if qualified.is_none() {
            session.complete_local(path, site.range.start, prefix, LIMIT + 1)
        } else {
            Vec::new()
        };
        let core = CORE.iter().filter_map(|entry| {
            if !entry.name.starts_with(qualified.unwrap_or(prefix))
                || (qualified.is_none()
                    && (locals.contains(&entry.name)
                        || !session.core_completion_visible(path, entry.name, offset)))
            {
                return None;
            }
            let label = qualified.map_or_else(
                || entry.name.to_owned(),
                |_| format!("clojure.core/{}", entry.name),
            );
            let mut text = label.clone();
            let mut format = 1;
            if snippets && let (Some(template), Some(closed)) = (entry.template, site.empty_call) {
                text.push(' ');
                text.push_str(template);
                if !closed {
                    text.push(')');
                }
                format = 2;
            }
            Some(
                json!({"label":label,"kind":if format == 2 {15} else {3},"detail":entry.detail,
                "insertTextFormat":format,"textEdit":{"range":range,"newText":text}}),
            )
        });
        locals
            .iter()
            .map(|name| json!({"label":name,"kind":6,"textEdit":{"range":range,"newText":name}}))
            .chain(core)
            .take(LIMIT + 1)
            .collect()
    };
    let incomplete = items.len() > LIMIT;
    items.truncate(LIMIT);
    Some(json!({"isIncomplete":incomplete,"items":items}))
}

pub fn resolve_uri(item: &Value) -> Option<&str> {
    item.pointer("/data/parenoxSource/uri")?.as_str()
}

pub fn resolve(session: &Session, path: &str, mut item: Value) -> Value {
    if item
        .get("documentation")
        .is_some_and(|documentation| !documentation.is_null())
    {
        return item;
    }
    if let (Some(label), Some(data)) = (item["label"].as_str(), item.pointer("/data/parenoxSource"))
        && data["label"].as_str() == Some(label)
        && let Some(namespace) = data["namespace"].as_str()
        && let Some(offset) = data
            .get("offset")
            .map_or(Some(0), |offset| usize::try_from(offset.as_u64()?).ok())
        && let Some(documentation) =
            session.completion_documentation_at(path, label, namespace, offset)
    {
        item["documentation"] = json!({"kind":"plaintext","value":documentation});
    }
    item
}
