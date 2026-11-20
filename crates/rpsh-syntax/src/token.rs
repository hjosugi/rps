use crate::Span;

/// The kinds of token the skeleton recognizes.
///
/// Upstream's `TokenKind` has 157 members (`inventory/token-kinds.tsv`); they
/// are added here as the tokenizer grows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// A decimal literal that fits `System.Int32`.
    Int32(i32),
    /// A single-quoted string, with `''` already unescaped.
    StringLiteral(String),
    Plus,
    Minus,
    Multiply,
    LParen,
    RParen,
    Semi,
    NewLine,
    /// The `exit` keyword.
    Exit,
    EndOfInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}
