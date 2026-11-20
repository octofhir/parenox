use parenox_analysis::reader::{self, Conditionals};
use parenox_analysis::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use serde_json::{Value, json};

use crate::text::{Lines, Position};

use super::{Command, REFACTOR_KIND, replace_node};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Collection {
    List,
    Map,
    Vector,
    Set,
}

impl Collection {
    const ALL: [Self; 4] = [Self::Vector, Self::Set, Self::List, Self::Map];

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|collection| collection.name() == name)
    }

    fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Map => "map",
            Self::Vector => "vector",
            Self::Set => "set",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::List => Self::Map,
            Self::Map => Self::Vector,
            Self::Vector => Self::Set,
            Self::Set => Self::List,
        }
    }

    fn delimiters(self) -> (&'static str, char) {
        match self {
            Self::List => ("(", ')'),
            Self::Map => ("{", '}'),
            Self::Vector => ("[", ']'),
            Self::Set => ("#{", '}'),
        }
    }
}

struct Selected {
    node: SyntaxNode,
    kind: Collection,
    body_start: usize,
}

impl Selected {
    fn at(token: &SyntaxToken) -> Option<Self> {
        use SyntaxKind::*;
        let (kind, expected, closing) = match token.kind() {
            OpenParen => (Collection::List, List, CloseParen),
            OpenBrace => (Collection::Map, Map, CloseBrace),
            OpenBracket => (Collection::Vector, Vector, CloseBracket),
            OpenSet => (Collection::Set, Set, CloseBrace),
            _ => return None,
        };
        let node = token.parent()?;
        if node.kind() != expected
            || node.last_child_or_token()?.kind() != closing
            || node
                .parent()
                .is_some_and(|parent| parent.kind() == NamespacedMap)
            || node
                .descendants_with_tokens()
                .any(|part| matches!(part.kind(), Error | ErrorToken))
        {
            return None;
        }
        Some(Self {
            node,
            kind,
            body_start: usize::from(token.text_range().end()),
        })
    }

    fn replacement(&self, source: &str, to: Collection) -> Option<String> {
        if self.kind == to {
            return None;
        }
        let body = &source[self.body_start..usize::from(self.node.text_range().end()) - 1];
        let (opening, closing) = to.delimiters();
        let replacement = format!("{opening}{body}{closing}");
        if matches!(to, Collection::Map | Collection::Set) {
            if self
                .node
                .descendants()
                .any(|node| node.kind() == SyntaxKind::Conditional)
            {
                for policy in [
                    Conditionals::Select(":clj"),
                    Conditionals::Select(":cljs"),
                    Conditionals::SelectFeatures(&[":cljrs", ":rust"]),
                ] {
                    reader::read_with(&replacement, policy).ok()?;
                }
            } else {
                reader::read(&replacement).ok()?;
            }
        }
        Some(replacement)
    }
}

pub(super) fn edit(
    lines: &Lines<'_>,
    token: &SyntaxToken,
    to: Option<Collection>,
) -> Option<Vec<Value>> {
    let selection = Selected::at(token)?;
    let new_text =
        selection.replacement(lines.source(), to.unwrap_or_else(|| selection.kind.next()))?;
    replace_node(lines, &selection.node, new_text)
}

pub(super) fn actions(
    lines: &Lines<'_>,
    token: &SyntaxToken,
    uri: &str,
    position: Position,
) -> Vec<Value> {
    let Some(selection) = Selected::at(token) else {
        return Vec::new();
    };
    Collection::ALL
        .into_iter()
        .filter(|kind| selection.replacement(lines.source(), *kind).is_some())
        .map(|kind| {
            let kind = kind.name();
            json!({
                "title":format!("Change coll to {kind}"), "kind":REFACTOR_KIND,
                "command":{
                    "title":"Change coll", "command":Command::ChangeCollection.name(),
                    "arguments":[uri, position.line, position.character, kind]
                }
            })
        })
        .collect()
}
