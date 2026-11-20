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
mod parser;
mod span;
mod token;
mod tokenizer;

pub use ast::{BinaryOp, Expr, ExprKind, ScriptBlockAst, Statement, UnaryOp};
pub use error::ParseError;
pub use parser::parse;
pub use span::Span;
pub use token::{Token, TokenKind};
pub use tokenizer::tokenize;
