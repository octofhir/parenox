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

    fn item_kind(self) -> Option<Self> {
        Some(match self {
            Self::OpenParen => Self::List,
            Self::OpenBracket => Self::Vector,
            Self::OpenBrace => Self::Map,
            Self::OpenSet => Self::Set,
            Self::OpenAnonymousFunction => Self::AnonymousFunction,
            Self::QuoteMark => Self::Quote,
            Self::SyntaxQuoteMark => Self::SyntaxQuote,
            Self::UnquoteMark => Self::Unquote,
            Self::UnquoteSplicingMark => Self::UnquoteSplicing,
            Self::MetadataMark => Self::Metadata,
            Self::ConditionalMark => Self::Conditional,
            Self::DerefMark => Self::Deref,
            Self::VarQuoteMark => Self::VarQuote,
            Self::NamespacedMapMark => Self::NamespacedMap,
            Self::Symbol
            | Self::Keyword
            | Self::String
            | Self::Character
            | Self::Number
            | Self::Boolean
            | Self::Nil
            | Self::Regex => self,
            _ => return None,
        })
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

/// Builds a lossless tree from the reader's tokens and delimiter boundaries.
pub(crate) struct Builder<'a> {
    source: &'a str,
    green: GreenNodeBuilder<'static>,
    stack: Vec<OpenNode>,
    covered: usize,
}

struct OpenNode {
    checkpoint: Checkpoint,
    offset: usize,
    kind: Option<SyntaxKind>,
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
            kind: None,
        });
    }

    pub fn token(&mut self, kind: SyntaxKind, range: Range<usize>) {
        debug_assert!(kind.is_token());
        debug_assert_eq!(self.covered, range.start);
        if let Some(kind) = kind.item_kind()
            && let Some(node) = self.stack.last_mut()
            && node.offset == range.start
        {
            node.kind = Some(kind);
        }
        self.covered = range.end;
        self.green.token(kind.into(), &self.source[range]);
    }

    /// A dispatch token can start different kinds of forms.
    pub fn mark_dispatch(&mut self, kind: SyntaxKind) {
        self.stack.last_mut().unwrap().kind = Some(kind);
    }

    pub fn finish_item(&mut self, valid: bool, offset: usize) {
        let node = self.stack.pop().unwrap();
        let kind = if valid {
            node.kind.expect("a successful form has a syntax kind")
        } else {
            SyntaxKind::Error
        };
        if kind.is_token() {
            debug_assert_eq!(self.covered, offset);
        } else {
            self.close_node(node, kind, offset);
        }
    }

    pub fn finish(&mut self, kind: SyntaxKind, offset: usize) {
        let node = self.stack.pop().unwrap();
        self.close_node(node, kind, offset);
    }

    fn close_node(&mut self, node: OpenNode, kind: SyntaxKind, offset: usize) {
        debug_assert!(!kind.is_token());
        if self.covered < offset {
            // Retain a partially consumed token when reading fails.
            self.token(SyntaxKind::ErrorToken, self.covered..offset);
        }
        self.green.start_node_at(node.checkpoint, kind.into());
        self.green.finish_node();
    }

    pub fn build(mut self) -> rowan::GreenNode {
        debug_assert!(self.stack.is_empty());
        debug_assert_eq!(self.covered, self.source.len());
        self.green.finish_node();
        self.green.finish()
    }
}
