//! PowerShell tokenizer, parser and AST.
//!
//! Upstream: `src/System.Management.Automation/engine/parser` (`tokenizer.cs`,
//! `Parser.cs`, `ast.cs`). Nushell analogue: `nu-parser`.
//!
//! # Status
//!
//! This is the walking skeleton from DESIGN.md §8: decimal `Int32` literals,
//! single-quoted strings, `+ - *`, unary minus, parentheses, statement
//! separators, `#` comments and `exit`. Everything else is reported as
//! [`ParseError::NotImplemented`], naming the missing feature.
//!
//! The rule that matters is in DESIGN.md §2: a script is parsed completely
//! before anything runs, and input outside the implemented subset is refused
//! rather than interpreted approximately.

mod ast;
mod error;
mod full_token_kind;
mod parser;
mod span;
mod token;
mod tokenizer;

pub use ast::{BinaryOp, Expr, ExprKind, ScriptBlockAst, Statement, UnaryOp};
pub use error::ParseError;
pub use full_token_kind::{FullTokenKind, TOKEN_KINDS, TokenFlags, TokenKindInfo};
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
