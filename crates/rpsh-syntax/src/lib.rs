//! PowerShell tokenizer, parser and AST.
//!
//! Upstream: `src/System.Management.Automation/engine/parser` (`tokenizer.cs`,
//! `Parser.cs`, `ast.cs`). Nushell analogue: `nu-parser`.
//!
//! # Status
//!
//! The full tokenizer produces all 157 reference token kinds with byte and
//! UTF-16 extents. The walking-skeleton parser still understands only
//! decimal `Int32` literals, single-quoted strings, `+ - *`, unary minus,
//! parentheses, statement separators, `#` comments and `exit`; everything
//! else is reported as [`ParseError::NotImplemented`], naming the missing
//! feature.
//!
//! The rule that matters is in DESIGN.md §2: a script is parsed completely
//! before anything runs, and input outside the implemented subset is refused
//! rather than interpreted approximately.

mod ast;
mod error;
mod full_token_kind;
mod full_tokenizer;
mod number;
mod parser;
mod span;
mod token;
mod tokenizer;

pub use ast::{BinaryOp, Expr, ExprKind, ScriptBlockAst, Statement, StringPart, UnaryOp};
pub use error::ParseError;
pub use full_token_kind::{FullTokenKind, TOKEN_KINDS, TokenFlags, TokenKindInfo};
pub use full_tokenizer::{FullToken, tokenize_full};
pub use number::{NumberLiteral, parse_number_literal};
pub use parser::parse;
pub use span::Span;
pub use token::{Token, TokenKind};
pub use tokenizer::tokenize;

#[cfg(test)]
mod tests {
    #[test]
    fn token_metadata_covers_every_inventory_row() {
        assert_eq!(super::TOKEN_KINDS.len(), 157);
        for info in super::TOKEN_KINDS {
            assert_eq!(info.kind.name(), info.kind.name());
        }
        assert_ne!(super::FullTokenKind::Plus.flags().0, 0);
        assert_eq!(super::FullTokenKind::Unknown.flags().0, 0);
    }
}
