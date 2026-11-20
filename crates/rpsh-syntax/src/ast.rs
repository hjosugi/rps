//! The AST the skeleton produces.
//!
//! Upstream has 74 public `*Ast` classes (`inventory/ast-nodes.tsv`). The names here
//! follow them so that the public `System.Management.Automation.Language` API
//! can later be layered on top without renaming.

use crate::Span;

/// A parsed script. Upstream: `ScriptBlockAst`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptBlockAst {
    pub statements: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    /// An expression whose value goes to the output stream.
    /// Upstream: a `PipelineAst` holding one `CommandExpressionAst`.
    Expression(Expr),
    /// `exit` with an optional exit code. Upstream: `ExitStatementAst`.
    Exit { code: Option<Expr>, span: Span },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    /// Upstream: `ConstantExpressionAst` holding an `int`.
    Int32(i32),
    /// Upstream: `StringConstantExpressionAst` (single-quoted).
    String(String),
    /// Upstream: `UnaryExpressionAst`.
    Unary { op: UnaryOp, operand: Box<Expr> },
    /// Upstream: `BinaryExpressionAst`.
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Upstream: `ParenExpressionAst`.
    Paren(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
}
