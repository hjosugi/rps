//! The AST the skeleton produces.
//!
//! Upstream has 74 public `*Ast` classes (`inventory/ast-nodes.tsv`). The names here
//! follow them so that the public `System.Management.Automation.Language` API
//! can later be layered on top without renaming.

use crate::{NumberLiteral, Span};

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
    /// `$name = expression`. Upstream: `AssignmentStatementAst`.
    Assign {
        name: String,
        value: Expr,
        span: Span,
    },
    /// `$name += expression`.
    AppendAssign {
        name: String,
        value: Expr,
        span: Span,
    },
    /// `$name ??= expression`.
    CoalesceAssign {
        name: String,
        value: Expr,
        span: Span,
    },
    /// `$name -= expression`, `*=`, `/=`, `%=`.
    CompoundAssign {
        name: String,
        op: BinaryOp,
        value: Expr,
        span: Span,
    },
    /// `$object.property = expression`.
    MemberAssign {
        target: Expr,
        name: String,
        value: Expr,
        span: Span,
    },
    /// `$object[index] = expression`.
    IndexAssign {
        target: Expr,
        index: Expr,
        value: Expr,
        span: Span,
    },
    /// `function Name(...) { ... }`; `filter` sets `filter`.
    Function {
        name: String,
        parameters: Vec<Parameter>,
        body: Vec<Statement>,
        filter: bool,
        span: Span,
    },
    /// A command invocation, `Name -a 1 x`.
    Command {
        name: String,
        arguments: Vec<CommandArgument>,
        span: Span,
    },
    /// A pipeline, `expr | command | ...`.
    Pipeline {
        commands: Vec<PipelineCommand>,
        span: Span,
    },
    /// `if` / `elseif` / `else`. Upstream: `IfStatementAst`.
    If {
        condition: Expr,
        then: Vec<Statement>,
        elseif: Vec<(Expr, Vec<Statement>)>,
        otherwise: Vec<Statement>,
        span: Span,
    },
    /// `while (condition) { ... }`. Upstream: `WhileStatementAst`.
    While {
        label: Option<String>,
        condition: Expr,
        body: Vec<Statement>,
        span: Span,
    },
    /// `for (init; condition; step) { ... }`. Upstream: `ForStatementAst`.
    For {
        label: Option<String>,
        init: Option<Box<Statement>>,
        condition: Option<Expr>,
        step: Option<Box<Statement>>,
        body: Vec<Statement>,
        span: Span,
    },
    /// `foreach ($name in collection) { ... }`. Upstream: `ForEachStatementAst`.
    Foreach {
        label: Option<String>,
        name: String,
        collection: Expr,
        body: Vec<Statement>,
        span: Span,
    },
    /// `do { ... } while|until (condition)`. Upstream: `DoWhileStatementAst` /
    /// `DoUntilStatementAst`.
    Do {
        label: Option<String>,
        body: Vec<Statement>,
        condition: Expr,
        until: bool,
        span: Span,
    },
    /// `try { ... } catch { ... } finally { ... }`. Upstream: `TryStatementAst`.
    Try {
        body: Vec<Statement>,
        catches: Vec<CatchClause>,
        finally: Option<Vec<Statement>>,
        span: Span,
    },
    /// `throw expression`. Upstream: `ThrowStatementAst`.
    Throw { value: Expr, span: Span },
    /// `return` with an optional value. Upstream: `ReturnStatementAst`.
    Return { value: Option<Expr>, span: Span },
    /// `break [label]`. Upstream: `BreakStatementAst`.
    Break { label: Option<String>, span: Span },
    /// `continue [label]`. Upstream: `ContinueStatementAst`.
    Continue { label: Option<String>, span: Span },
    /// `exit` with an optional exit code. Upstream: `ExitStatementAst`.
    Exit { code: Option<Expr>, span: Span },
}

/// One stage of a pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineCommand {
    /// A `CommandExpressionAst` stage, such as `1..3`.
    Expression(Expr),
    /// A command stage, such as `Sum`.
    Command {
        name: String,
        arguments: Vec<CommandArgument>,
    },
}

/// A declared parameter. Upstream: `ParameterAst`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub type_name: Option<String>,
    pub default: Option<Expr>,
}

/// One argument of a command invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandArgument {
    /// An expression, `Add (1 + 2)` or `Add $x`.
    Value(Expr),
    /// `-Name value`, or `-Switch` with no value.
    Parameter { name: String, value: Option<Expr> },
    /// A bare word such as `Add hello`.
    Bareword(String),
}

/// One `catch` clause of a `try` statement. Upstream: `CatchClauseAst`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatchClause {
    /// The exception types the clause catches. Empty means every exception.
    pub types: Vec<String>,
    pub body: Vec<Statement>,
    pub span: Span,
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
    /// Upstream: `ConstantExpressionAst` holding another numeric type.
    Number(NumberLiteral),
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
    /// Upstream: `VariableExpressionAst`.
    Variable(String),
    /// Upstream: `ExpandableStringExpressionAst`.
    ExpandableString(Vec<StringPart>),
    /// Upstream: `SubExpressionAst`.
    Subexpression(Box<Expr>),
    /// Upstream: `TypeExpressionAst` used as a value.
    TypeLiteral(String),
    /// Upstream: `ArrayLiteralAst`.
    Array(Vec<Expr>),
    /// Upstream: `ScriptBlockExpressionAst`.
    ScriptBlock(Vec<Statement>),
    /// Upstream: `IndexExpressionAst`.
    Index { target: Box<Expr>, index: Box<Expr> },
    /// Upstream: `TernaryExpressionAst`.
    Ternary {
        condition: Box<Expr>,
        when_true: Box<Expr>,
        when_false: Box<Expr>,
    },
    /// Upstream: `BinaryExpressionAst` with `??`.
    Coalesce { left: Box<Expr>, right: Box<Expr> },
    /// Upstream: `HashtableAst` (and the value of `[pscustomobject]`).
    Object(Vec<(String, Expr)>),
    /// Upstream: `ConvertExpressionAst` for the supported `[pscustomobject]`.
    Cast { type_name: String, value: Box<Expr> },
    /// Upstream: `MemberExpressionAst`.
    Member { target: Box<Expr>, name: String },
    /// Upstream: `InvokeMemberExpressionAst`.
    InvokeMember {
        target: Box<Expr>,
        name: String,
        arguments: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringPart {
    Literal(String),
    Variable(String),
    Subexpression(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
    Join,
    Not,
    Bnot,
    /// `++$x`. Upstream: a `UnaryExpressionAst` whose token is `++`.
    Increment,
    /// `--$x`.
    Decrement,
    /// `$x++`.
    PostIncrement,
    /// `$x--`.
    PostDecrement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Range,
    Join,
    Eq,
    Ne,
    Ge,
    Gt,
    Lt,
    Le,
    CEq,
    CNe,
    CGe,
    CGt,
    CLt,
    CLe,
    Like,
    NotLike,
    CLike,
    CNotLike,
    Contains,
    NotContains,
    CContains,
    CNotContains,
    In,
    NotIn,
    CIn,
    CNotIn,
    Shl,
    Shr,
    Band,
    Bor,
    Bxor,
    And,
    Or,
    Xor,
    Is,
    IsNot,
    As,
    Match,
    NotMatch,
    CMatch,
    CNotMatch,
    Replace,
    CReplace,
    Split,
    CSplit,
}
