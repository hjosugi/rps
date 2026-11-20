//! PowerShell execution engine.
//!
//! Upstream: `src/System.Management.Automation/engine` — the compiler
//! (`parser/Compiler.cs`), the interpreter, session state, command discovery,
//! parameter binding and the pipeline. Nushell analogue: `nu-engine`.
//!
//! # Status
//!
//! A tree-walking evaluator for the walking skeleton. There is no session
//! state yet, so constructing an [`Engine`] costs nothing; DESIGN.md §4
//! requires that this stays true as state is added, by building each table
//! on first use.

use std::fmt;
use std::io::{self, Write};

use rpsh_runtime::{RuntimeError, Value, ops};
use rpsh_syntax::{BinaryOp, Expr, ExprKind, ParseError, Statement, UnaryOp, parse};

/// Runs scripts.
#[derive(Debug, Default)]
pub struct Engine {
    _private: (),
}

#[derive(Debug)]
pub enum EngineError {
    /// The script was refused before anything ran.
    Parse(ParseError),
    /// The script stopped part-way. Output written so far stays written.
    Runtime(RuntimeError),
    /// Writing to the output stream failed.
    Io(io::Error),
}

impl EngineError {
    /// The label of the missing feature, when that is why the script failed.
    pub fn missing_feature(&self) -> Option<&'static str> {
        match self {
            Self::Parse(error) => Some(error.feature()),
            Self::Runtime(error) => Some(error.feature()),
            Self::Io(_) => None,
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::Runtime(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<ParseError> for EngineError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}

impl From<RuntimeError> for EngineError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<io::Error> for EngineError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `source` in full, then runs it, writing what reaches the end of
    /// the pipeline to `out`. Returns the process exit code: the argument of
    /// `exit`, or 0 when the script runs to its end.
    pub fn run_script(&mut self, source: &str, out: &mut dyn Write) -> Result<i32, EngineError> {
        let script = parse(source)?;
        for statement in &script.statements {
            match statement {
                Statement::Expression(expr) => {
                    let value = evaluate(expr)?;
                    writeln!(out, "{value}")?;
                }
                Statement::Exit { code: None, .. } => return Ok(0),
                Statement::Exit {
                    code: Some(code), ..
                } => {
                    return match evaluate(code)? {
                        Value::Int32(code) => Ok(code),
                        _ => Err(RuntimeError::NotImplemented {
                            feature: "exit with a value other than Int32",
                        }
                        .into()),
                    };
                }
            }
        }
        Ok(0)
    }
}

fn evaluate(expr: &Expr) -> Result<Value, RuntimeError> {
    match &expr.kind {
        ExprKind::Int32(value) => Ok(Value::Int32(*value)),
        ExprKind::String(value) => Ok(Value::String(value.clone())),
        ExprKind::Paren(inner) => evaluate(inner),
        ExprKind::Unary {
            op: UnaryOp::Negate,
            operand,
        } => ops::negate(&evaluate(operand)?),
        ExprKind::Binary { op, left, right } => {
            let left = evaluate(left)?;
            let right = evaluate(right)?;
            match op {
                BinaryOp::Add => ops::add(&left, &right),
                BinaryOp::Subtract => ops::subtract(&left, &right),
                BinaryOp::Multiply => ops::multiply(&left, &right),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> (Result<i32, EngineError>, String) {
        let mut out = Vec::new();
        let result = Engine::new().run_script(source, &mut out);
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn writes_each_statement_value_on_its_own_line() {
        let (result, out) = run("1 + 1\n'a' + 'b'; (1 + 2) * 3; -5; ''");
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "2\nab\n9\n-5\n\n");
    }

    #[test]
    fn exit_stops_the_script_and_sets_the_exit_code() {
        let (result, out) = run("1; exit 1 + 2; 2");
        assert_eq!(result.unwrap(), 3);
        assert_eq!(out, "1\n");

        let (result, out) = run("exit");
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "");
    }

    #[test]
    fn an_unparsable_script_runs_nothing() {
        let (result, out) = run("1\n2\n$x = 3");
        assert_eq!(result.unwrap_err().missing_feature(), Some("variables"));
        assert_eq!(out, "");
    }

    #[test]
    fn a_runtime_refusal_keeps_earlier_output() {
        let (result, out) = run("1; 2147483647 + 1; 3");
        assert_eq!(
            result.unwrap_err().missing_feature(),
            Some("Int32 overflow (promotion to Double)")
        );
        assert_eq!(out, "1\n");
    }
}
