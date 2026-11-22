//! Lossless Clojure syntax trees, captured by the same reader used for semantic forms.
use std::ops::Range;

use rowan::{Checkpoint, GreenNodeBuilder};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    Root,
    List,
    Vector,
    Map,
    Set,
    Quote,
    Discard,
    Metadata,
    Tagged,
    Conditional,
    Error,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    OpenSet,
    QuoteMark,
    MetadataMark,
    DispatchMark,
    ConditionalMark,
    DiscardMark,
    Whitespace,
    Comma,
    Comment,
    Symbol,
    Keyword,
    String,
    Character,
    Number,
    Boolean,
    Nil,
    AnonymousFunction,
    OpenAnonymousFunction,
    Regex,
    Deref,
    VarQuote,
    DerefMark,
    VarQuoteMark,
    SymbolicNumber,
    SyntaxQuote,
    Unquote,
    UnquoteSplicing,
    SyntaxQuoteMark,
    UnquoteMark,
    UnquoteSplicingMark,
    NamespacedMap,
    NamespacedMapMark,
    ErrorToken,
}

impl SyntaxKind {
    pub fn is_token(self) -> bool {
        !matches!(
            self,
            Self::Root
                | Self::List
                | Self::AnonymousFunction
                | Self::Vector
                | Self::Map
                | Self::Set
                | Self::Quote
                | Self::Deref
                | Self::VarQuote
                | Self::Discard
                | Self::Metadata
                | Self::Tagged
                | Self::Conditional
                | Self::SymbolicNumber
                | Self::SyntaxQuote
                | Self::Unquote
                | Self::UnquoteSplicing
                | Self::NamespacedMap
                | Self::Error
        )
    }

    pub fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Comma | Self::Comment)
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        Self(kind as u16)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ClojureLanguage {}

impl rowan::Language for ClojureLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        assert!(raw.0 <= SyntaxKind::ErrorToken as u16);
        // SAFETY: repr(u16), contiguous discriminants from Root=0 through ErrorToken.
        unsafe { std::mem::transmute::<u16, SyntaxKind>(raw.0) }
    }

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }
}

pub type SyntaxNode = rowan::SyntaxNode<ClojureLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<ClojureLanguage>;
pub type SyntaxElement = rowan::SyntaxElement<ClojureLanguage>;

/// Delays choosing each node's kind until the reader has recognized its semantic form.
pub(crate) struct Builder<'a> {
    source: &'a str,
    green: GreenNodeBuilder<'static>,
    stack: Vec<OpenNode>,
    covered: usize,
}

struct OpenNode {
    checkpoint: Checkpoint,
    offset: usize,
    prefix_kind: Option<SyntaxKind>,
}

impl<'a> Builder<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut green = GreenNodeBuilder::new();
        green.start_node(SyntaxKind::Root.into());
        Self {
            source,
            green,
            stack: Vec::new(),
            covered: 0,
        }
    }

    pub fn start(&mut self, offset: usize) {
        debug_assert_eq!(self.covered, offset);
        self.stack.push(OpenNode {
            checkpoint: self.green.checkpoint(),
            offset,
            prefix_kind: None,
        });
    }

    pub fn token(&mut self, kind: SyntaxKind, range: Range<usize>) {
        debug_assert!(kind.is_token());
        debug_assert_eq!(self.covered, range.start);
        // These prefixes can produce a different semantic form. The token
        // emitted by the reader retains the original syntax classification.
        let prefix_kind = match kind {
            SyntaxKind::ConditionalMark => Some(SyntaxKind::Conditional),
            SyntaxKind::DerefMark => Some(SyntaxKind::Deref),
            SyntaxKind::VarQuoteMark => Some(SyntaxKind::VarQuote),
            SyntaxKind::NamespacedMapMark => Some(SyntaxKind::NamespacedMap),
            _ => None,
        };
        if let Some(prefix_kind) = prefix_kind {
            let node = self.stack.last_mut().unwrap();
            debug_assert_eq!(node.offset, range.start);
            node.prefix_kind = Some(prefix_kind);
        }
        self.covered = range.end;
        self.green.token(kind.into(), &self.source[range]);
    }

    pub fn finish(&mut self, kind: SyntaxKind, offset: usize) {
        let node = self.stack.pop().unwrap();
        let kind = if kind == SyntaxKind::Error {
            kind
        } else {
            node.prefix_kind.unwrap_or(kind)
        };
        if kind.is_token() {
            self.token(kind, node.offset..offset);
        } else {
            if self.covered < offset {
                // Retain a partially consumed token when reading fails.
                self.token(SyntaxKind::ErrorToken, self.covered..offset);
            }
            self.green.start_node_at(node.checkpoint, kind.into());
            self.green.finish_node();
        }
    }

    pub fn build(mut self) -> rowan::GreenNode {
        debug_assert!(self.stack.is_empty());
        debug_assert_eq!(self.covered, self.source.len());
        self.green.finish_node();
        self.green.finish()
    }
}
