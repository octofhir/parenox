//! Complete symbols at the cursor using the lossless reader and project index.
use std::ops::Range;

use parenox_analysis::Session;
use parenox_analysis::reader;
use parenox_analysis::syntax::SyntaxKind;
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
        })
    }
}

pub fn complete(
    session: &Session,
    path: &str,
    uri: &str,
    source: &str,
    lines: &Lines<'_>,
    offset: usize,
    snippets: bool,
) -> Option<Value> {
    const LIMIT: usize = 60;
    let site = Site::at(source, offset)?;
    let prefix = &source[site.range.start..offset];
    let range =
        json!({"start": lines.position(site.range.start)?, "end": lines.position(site.range.end)?});
    let mut items: Vec<_> = if site.kind == SyntaxKind::Keyword {
        if !prefix.starts_with("::") && !prefix.contains('/') {
            return None;
        }
        session.complete_spec_key(path, prefix, LIMIT + 1).into_iter().map(|key| {
            json!({"label":key,"kind":14,"detail":"Project spec","textEdit":{"range":range,"newText":key}})
        }).collect()
    } else if prefix.contains('/') && !prefix.starts_with("clojure.core/") {
        let namespace = prefix
            .split_once('/')
            .and_then(|(qualifier, _)| session.completion_namespace(path, qualifier));
        session
            .complete_qualified_var(path, prefix, LIMIT + 1)
            .into_iter()
            .map(|name| {
                let mut item =
                    json!({"label":name,"kind":6,"textEdit":{"range":range,"newText":name}});
                if let Some(namespace) = namespace {
                    item["data"] =
                        json!({"parenoxSource":{"uri":uri,"label":name,"namespace":namespace}});
                }
                item
            })
            .collect()
    } else {
        let qualified = prefix.strip_prefix("clojure.core/");
        CORE.iter()
            .filter_map(|entry| {
                if !entry.name.starts_with(qualified.unwrap_or(prefix))
                    || (qualified.is_none()
                        && !session.core_completion_visible(path, entry.name, offset))
                {
                    return None;
                }
                let label = qualified.map_or_else(
                    || entry.name.to_owned(),
                    |_| format!("clojure.core/{}", entry.name),
                );
                let mut text = label.clone();
                let mut format = 1;
                if snippets
                    && let (Some(template), Some(closed)) = (entry.template, site.empty_call)
                {
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
            })
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
        && let Some(documentation) = session.completion_documentation(path, label, namespace)
    {
        item["documentation"] = json!({"kind":"plaintext","value":documentation});
    }
    item
}
