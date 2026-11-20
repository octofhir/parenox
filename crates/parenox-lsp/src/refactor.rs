//! Small source edits for editor refactoring commands.
mod collections;
mod conditionals;
mod keywords;
mod lookups;
mod threading;
mod unwind;

pub use collections::Collection;

use parenox_analysis::reader::{self, Form, Kind, Span};
use parenox_analysis::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use parenox_analysis::{PrivacyDeclaration, Session};
use serde_json::{Value, json};

use crate::text::{Lines, Range};
use threading::Direction;

pub const CYCLE_PRIVACY_TITLE: &str = "Cycle privacy";
pub const REFACTOR_KIND: &str = "refactor";
pub const REWRITE_KIND: &str = "refactor.rewrite";
pub const EXTRACT_KIND: &str = "refactor.extract";

#[derive(Clone, Copy)]
pub enum Command {
    CyclePrivacy,
    CycleKeyword,
    ThreadFirst,
    ThreadFirstAll,
    ThreadLast,
    ThreadLastAll,
    UnwindThread,
    UnwindAll,
    CycleCollection,
    ChangeCollection,
    IfToCond,
    CondToIf,
    GetInAll,
    GetInLess,
    GetInNone,
}

impl Command {
    pub const ALL: [Self; 15] = [
        Self::CyclePrivacy,
        Self::CycleKeyword,
        Self::ThreadFirst,
        Self::ThreadFirstAll,
        Self::ThreadLast,
        Self::ThreadLastAll,
        Self::UnwindThread,
        Self::UnwindAll,
        Self::CycleCollection,
        Self::ChangeCollection,
        Self::IfToCond,
        Self::CondToIf,
        Self::GetInAll,
        Self::GetInLess,
        Self::GetInNone,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::CyclePrivacy => "cycle-privacy",
            Self::CycleKeyword => "cycle-keyword-auto-resolve",
            Self::ThreadFirst => "thread-first",
            Self::ThreadFirstAll => "thread-first-all",
            Self::ThreadLast => "thread-last",
            Self::ThreadLastAll => "thread-last-all",
            Self::UnwindThread => "unwind-thread",
            Self::UnwindAll => "unwind-all",
            Self::CycleCollection => "cycle-coll",
            Self::ChangeCollection => "change-coll",
            Self::IfToCond => "if->cond-refactor",
            Self::CondToIf => "cond->if-refactor",
            Self::GetInAll => "get-in-all",
            Self::GetInLess => "get-in-less",
            Self::GetInNone => "get-in-none",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.name() == name)
    }

    pub fn missing_edit(self) -> (i32, &'static str) {
        match self {
            Self::CyclePrivacy => (-32600, "No editable var declaration at this position"),
            Self::IfToCond => (-32602, "Not an if expression"),
            Self::CondToIf => (-32602, "Not a cond"),
            _ => (-32600, "Nothing to change."),
        }
    }

    pub fn edits(
        self,
        session: &Session,
        path: &str,
        lines: &Lines<'_>,
        offset: usize,
        collection: Option<Collection>,
    ) -> Option<Vec<Value>> {
        match self {
            Self::CyclePrivacy => cycle_privacy(session, path, lines, offset),
            Self::ThreadFirst if lines.source().as_bytes().get(offset) != Some(&b'(') => None,
            _ => {
                let token = token_at(lines, offset, matches!(self, Self::ThreadLast))?;
                match self {
                    Self::CycleCollection => collections::edit(lines, &token, None),
                    Self::ChangeCollection => collections::edit(lines, &token, Some(collection?)),
                    Self::CycleKeyword => Some(vec![
                        keywords::cycle(session, path, &token, offset)?.edit(lines)?,
                    ]),
                    Self::GetInAll | Self::GetInLess | Self::GetInNone => {
                        self.lookup_edit(session, path, lines, &token)
                    }
                    Self::IfToCond | Self::CondToIf => {
                        self.conditional_edit(session, path, lines, &token)
                    }
                    _ => self.threading_edit(session, path, lines, &token),
                }
            }
        }
    }

    fn threading_edit(
        self,
        session: &Session,
        path: &str,
        lines: &Lines<'_>,
        token: &SyntaxToken,
    ) -> Option<Vec<Value>> {
        match self {
            Self::CyclePrivacy
            | Self::CycleKeyword
            | Self::CycleCollection
            | Self::ChangeCollection
            | Self::IfToCond
            | Self::CondToIf
            | Self::GetInAll
            | Self::GetInLess
            | Self::GetInNone => None,
            Self::ThreadFirst => {
                threading::thread(session, path, lines, token, Direction::First, false)
            }
            Self::ThreadFirstAll => {
                threading::thread(session, path, lines, token, Direction::First, true)
            }
            Self::ThreadLast => {
                threading::thread(session, path, lines, token, Direction::Last, false)
            }
            Self::ThreadLastAll => {
                threading::thread(session, path, lines, token, Direction::Last, true)
            }
            Self::UnwindThread => unwind::unwind(session, path, lines, token, false),
            Self::UnwindAll => unwind::unwind(session, path, lines, token, true),
        }
    }

    fn conditional_edit(
        self,
        session: &Session,
        path: &str,
        lines: &Lines<'_>,
        token: &SyntaxToken,
    ) -> Option<Vec<Value>> {
        match self {
            Self::IfToCond => conditionals::if_to_cond(session, path, lines, token),
            Self::CondToIf => conditionals::cond_to_if(session, path, lines, token),
            _ => None,
        }
    }

    fn lookup_edit(
        self,
        session: &Session,
        path: &str,
        lines: &Lines<'_>,
        token: &SyntaxToken,
    ) -> Option<Vec<Value>> {
        match self {
            Self::GetInAll => lookups::get_in_all(session, path, lines, token),
            Self::GetInLess | Self::GetInNone => {
                lookups::unwind_get_in(session, path, lines, token, matches!(self, Self::GetInNone))
            }
            _ => None,
        }
    }
}

pub fn action_allowed(kind: &str, only: Option<&[String]>) -> bool {
    only.is_none_or(|kinds| {
        kinds.iter().any(|prefix| {
            prefix.is_empty()
                || kind == prefix
                || kind
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
    })
}

pub fn command_actions(
    session: &Session,
    path: &str,
    lines: &Lines<'_>,
    offset: usize,
    uri: &str,
    only: Option<&[String]>,
) -> Vec<Value> {
    let (Some(token), Some(position)) = (token_at(lines, offset, false), lines.position(offset))
    else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    if action_allowed(REWRITE_KIND, only) {
        let titles = [
            (Command::ThreadFirstAll, "Thread first all"),
            (Command::ThreadLastAll, "Thread last all"),
            (Command::UnwindThread, "Unwind thread once"),
            (Command::UnwindAll, "Unwind whole thread"),
        ];
        actions.extend(titles.into_iter().filter_map(|(command, title)| {
            command.threading_edit(session, path, lines, &token)?;
            Some(json!({
                "title":title,"kind":REWRITE_KIND,"command":{
                    "title":title,"command":command.name(),"arguments":[uri,position.line,position.character]
                }
            }))
        }));
        if let Some(cycle) = keywords::cycle(session, path, &token, offset) {
            let title = cycle.title();
            actions.push(json!({
                "title":title,"kind":REWRITE_KIND,"command":{
                    "title":title,"command":Command::CycleKeyword.name(),"arguments":[uri,position.line,position.character]
                }
            }));
        }
        let titles = [
            (Command::GetInAll, "Move all expressions to get/get-in"),
            (Command::GetInLess, "Remove one element from get/get-in"),
            (Command::GetInNone, "Unwind whole get/get-in"),
        ];
        actions.extend(titles.into_iter().filter_map(|(command, title)| {
            command.lookup_edit(session, path, lines, &token)?;
            Some(json!({
                "title":title,"kind":REWRITE_KIND,"command":{
                    "title":title,"command":command.name(),"arguments":[uri,position.line,position.character]
                }
            }))
        }));
    }
    if action_allowed(REFACTOR_KIND, only) {
        actions.extend(collections::actions(lines, &token, uri, position));
    }
    if action_allowed(EXTRACT_KIND, only) {
        let titles = [
            (Command::IfToCond, "Change nested if to cond"),
            (Command::CondToIf, "Change cond to nested if"),
        ];
        actions.extend(titles.into_iter().filter_map(|(command, title)| {
            command.conditional_edit(session, path, lines, &token)?;
            Some(json!({
                "title":title,"kind":EXTRACT_KIND,"command":{
                    "title":title,"command":command.name(),"arguments":[uri,position.line,position.character]
                }
            }))
        }));
    }
    actions
}

fn replace_node(lines: &Lines<'_>, node: &SyntaxNode, new_text: String) -> Option<Vec<Value>> {
    let range = Range {
        start: lines.position(usize::from(node.text_range().start()))?,
        end: lines.position(usize::from(node.text_range().end()))?,
    };
    Some(vec![json!({"range":range,"newText":new_text})])
}

fn push_trivia(text: &mut String, gap: &str) -> bool {
    if gap.trim_matches(threading::SPACE).is_empty() {
        return false;
    }
    if gap.starts_with("#_") && !text.ends_with(threading::SPACE) {
        text.push(' ');
    }
    text.push_str(gap);
    true
}

fn token_at(lines: &Lines<'_>, offset: usize, previous_closer: bool) -> Option<SyntaxToken> {
    let syntax = reader::parse(lines.source()).syntax();
    let tokens = syntax.token_at_offset(u32::try_from(offset).ok()?.into());
    if !previous_closer {
        return tokens.right_biased();
    }
    let right = tokens.clone().right_biased()?;
    if right.kind() == SyntaxKind::CloseParen {
        // At consecutive closers, the call immediately to the left has ended.
        return tokens
            .left_biased()
            .filter(|left| left.kind() == SyntaxKind::CloseParen && *left != right);
    }
    Some(right)
}

pub fn cycle_privacy(
    session: &Session,
    path: &str,
    lines: &Lines<'_>,
    offset: usize,
) -> Option<Vec<Value>> {
    let PrivacyDeclaration {
        operator,
        name,
        attribute,
        core_name,
    } = session.privacy_declaration_at(path, offset)?;
    if let Some(attribute) = attribute {
        return Some(vec![replace_flag(lines, attribute, !truthy(attribute))?]);
    }

    let metadata = name.metadata(":private");
    let mut edits = Vec::new();
    if core_name == "defn-" || (core_name == "defn" && metadata.is_none()) {
        let head = operator.without_metadata();
        let new_head = if core_name == "defn-" {
            "defn"
        } else {
            "defn-"
        };
        let original = head.symbol()?;
        let replacement = if let Some((qualifier, _)) = original.split_once('/') {
            format!("{qualifier}/{new_head}")
        } else if session.core_completion_visible(path, new_head, head.span.start.offset) {
            new_head.to_owned()
        } else {
            format!("clojure.core/{new_head}")
        };
        edits.push(replace(lines, head.span, &replacement)?);
        if core_name == "defn" || !metadata.is_some_and(truthy) {
            return Some(edits);
        }
    }
    let edit = if let Some(flag) = metadata {
        match shorthand(lines, name, flag) {
            Some(edit) => edit,
            None => replace_flag(lines, flag, !truthy(flag))?,
        }
    } else {
        let original = text(lines, name.span)?;
        replace(lines, name.span, &format!("^:private {original}"))?
    };
    edits.push(edit);
    Some(edits)
}

fn truthy(form: &Form) -> bool {
    !matches!(form.without_metadata().kind, Kind::Bool(false) | Kind::Nil)
}

fn replace_flag(lines: &Lines<'_>, flag: &Form, private: bool) -> Option<Value> {
    let token = flag.without_metadata();
    if !matches!(token.kind, Kind::Bool(_) | Kind::Nil) {
        return None;
    }
    replace(lines, token.span, if private { "true" } else { "false" })
}

fn shorthand(lines: &Lines<'_>, mut name: &Form, flag: &Form) -> Option<Value> {
    if text(lines, flag.span)? != ":private" {
        return None;
    }
    while let Kind::Metadata(metadata, inner) = &name.kind {
        if metadata.span == flag.span {
            let source = lines.source();
            let prefix = source.get(name.span.start.offset..flag.span.start.offset)?;
            let clean_marker = prefix.trim_end().eq("^") || prefix.trim_end().eq("#^");
            if !clean_marker || inner.metadata(":private").is_some_and(truthy) {
                return replace(lines, flag.span, "{:private false}");
            }
            let remaining = source.get(flag.span.end.offset..name.span.end.offset)?;
            return replace(lines, name.span, remaining.trim_start_matches([' ', '\t']));
        }
        name = inner;
    }
    None
}

fn text<'a>(lines: &Lines<'a>, span: Span) -> Option<&'a str> {
    lines.source().get(span.start.offset..span.end.offset)
}

fn replace(lines: &Lines<'_>, span: Span, new_text: &str) -> Option<Value> {
    Some(json!({"range":lines.range(span)?,"newText":new_text}))
}
