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

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};

use rpsh_runtime::{RuntimeError, Value, ops};
use rpsh_syntax::{
    BinaryOp, Expr, ExprKind, NumberLiteral, ParseError, ScriptBlockAst, Statement, UnaryOp, parse,
};

/// Runs scripts.
#[derive(Debug, Default)]
pub struct Engine {
    variables: HashMap<String, Value>,
    last_success: bool,
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
        self.run_ast(&script, out)
    }

    /// Runs an already parsed script. For a caller that keeps the AST, this
    /// separates parsing from execution, as upstream's `ScriptBlock` does.
    pub fn run_ast(
        &mut self,
        script: &ScriptBlockAst,
        out: &mut dyn Write,
    ) -> Result<i32, EngineError> {
        for statement in &script.statements {
            match statement {
                Statement::Expression(expr) => {
                    let value = evaluate(expr, &self.variables, self.last_success)?;
                    match value {
                        Value::Array(items) => {
                            for item in items {
                                if !matches!(item, Value::Null) {
                                    writeln!(out, "{item}")?;
                                }
                            }
                        }
                        Value::Null => {}
                        value => writeln!(out, "{value}")?,
                    }
                    self.last_success = true;
                }
                Statement::CoalesceAssign { name, value, .. } => {
                    let key = strip_scope(name);
                    if matches!(self.variables.get(key), None | Some(Value::Null)) {
                        let value = evaluate(value, &self.variables, self.last_success)?;
                        self.variables.insert(key.to_owned(), value);
                    }
                    self.last_success = true;
                }
                Statement::AppendAssign { name, value, .. } => {
                    let value = evaluate(value, &self.variables, self.last_success)?;
                    let key = strip_scope(name);
                    let updated = append(self.variables.get(key), value)?;
                    self.variables.insert(key.to_owned(), updated);
                    self.last_success = true;
                }
                Statement::Assign { name, value, .. } => {
                    let value = evaluate(value, &self.variables, self.last_success)?;
                    let key = strip_scope(name);
                    self.variables.insert(key.to_owned(), value);
                    self.last_success = true;
                }
                Statement::Exit { code: None, .. } => return Ok(0),
                Statement::Exit {
                    code: Some(code), ..
                } => {
                    return match evaluate(code, &self.variables, self.last_success)? {
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

fn evaluate(
    expr: &Expr,
    variables: &HashMap<String, Value>,
    last_success: bool,
) -> Result<Value, RuntimeError> {
    match &expr.kind {
        ExprKind::Int32(value) => Ok(Value::Int32(*value)),
        ExprKind::Number(value) => Ok(number_value(value)),
        ExprKind::String(value) => Ok(Value::String(value.clone())),
        ExprKind::Paren(inner) => evaluate(inner, variables, last_success),
        ExprKind::Variable(name) => {
            lookup(name, variables, last_success).ok_or(RuntimeError::NotImplemented {
                feature: "undefined variable",
            })
        }
        ExprKind::ExpandableString(parts) => {
            let mut text = String::new();
            for part in parts {
                match part {
                    rpsh_syntax::StringPart::Literal(value) => text.push_str(value),
                    rpsh_syntax::StringPart::Variable(name) => {
                        if let Some(value) = lookup(name, variables, last_success) {
                            use std::fmt::Write as _;
                            let _ = write!(text, "{value}");
                        }
                    }
                    rpsh_syntax::StringPart::Subexpression(expr) => {
                        use std::fmt::Write as _;
                        let _ = write!(text, "{}", evaluate(expr, variables, last_success)?);
                    }
                }
            }
            Ok(Value::String(text))
        }
        ExprKind::Subexpression(expr) => evaluate(expr, variables, last_success),
        ExprKind::Ternary {
            condition,
            when_true,
            when_false,
        } => {
            if ops::is_true(&evaluate(condition, variables, last_success)?) {
                evaluate(when_true, variables, last_success)
            } else {
                evaluate(when_false, variables, last_success)
            }
        }
        ExprKind::TypeLiteral(name) => Ok(Value::Type(type_full_name(name).to_owned())),
        ExprKind::Coalesce { left, right } => {
            let value = evaluate(left, variables, last_success)?;
            if matches!(value, Value::Null) {
                evaluate(right, variables, last_success)
            } else {
                Ok(value)
            }
        }
        ExprKind::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(evaluate(item, variables, last_success)?);
            }
            Ok(Value::Array(values))
        }
        ExprKind::Index { target, index } => {
            let target = evaluate(target, variables, last_success)?;
            let index = evaluate(index, variables, last_success)?;
            index_value(&target, &index)
        }
        ExprKind::Object(entries) => {
            let mut object = Vec::with_capacity(entries.len());
            for (name, value) in entries {
                object.push((name.clone(), evaluate(value, variables, last_success)?));
            }
            Ok(Value::Object(object))
        }
        ExprKind::Cast { type_name, value } => {
            let value = evaluate(value, variables, last_success)?;
            if type_name.eq_ignore_ascii_case("pscustomobject") {
                Ok(value)
            } else {
                Err(RuntimeError::NotImplemented {
                    feature: "type casts other than [pscustomobject]",
                })
            }
        }
        ExprKind::Member { target, name } => {
            member(&evaluate(target, variables, last_success)?, name)
        }
        ExprKind::InvokeMember { target, name } => {
            if name == "GetType" {
                Ok(Value::Type(
                    evaluate(target, variables, last_success)?
                        .type_name()
                        .to_owned(),
                ))
            } else {
                Err(RuntimeError::NotImplemented {
                    feature: "method invocation other than GetType()",
                })
            }
        }
        ExprKind::Unary {
            op: UnaryOp::Negate,
            operand,
        } => ops::negate(&evaluate(operand, variables, last_success)?),
        ExprKind::Unary {
            op: UnaryOp::Join,
            operand,
        } => ops::unary_join(&evaluate(operand, variables, last_success)?),
        ExprKind::Unary {
            op: UnaryOp::Not,
            operand,
        } => Ok(ops::not(&evaluate(operand, variables, last_success)?)),
        ExprKind::Unary {
            op: UnaryOp::Bnot,
            operand,
        } => ops::bnot(&evaluate(operand, variables, last_success)?),
        ExprKind::Binary { op, left, right } => {
            let left = evaluate(left, variables, last_success)?;
            let right = evaluate(right, variables, last_success)?;
            match op {
                BinaryOp::Add => ops::add(&left, &right),
                BinaryOp::Subtract => ops::subtract(&left, &right),
                BinaryOp::Multiply => ops::multiply(&left, &right),
                BinaryOp::Divide => ops::divide(&left, &right),
                BinaryOp::Remainder => ops::remainder(&left, &right),
                BinaryOp::Range => ops::range(&left, &right),
                BinaryOp::Join => ops::join(&left, &right),
                BinaryOp::Eq => ops::compare(&left, &right, ops::Comparison::Eq, false),
                BinaryOp::Ne => ops::compare(&left, &right, ops::Comparison::Ne, false),
                BinaryOp::Ge => ops::compare(&left, &right, ops::Comparison::Ge, false),
                BinaryOp::Gt => ops::compare(&left, &right, ops::Comparison::Gt, false),
                BinaryOp::Lt => ops::compare(&left, &right, ops::Comparison::Lt, false),
                BinaryOp::Le => ops::compare(&left, &right, ops::Comparison::Le, false),
                BinaryOp::CEq => ops::compare(&left, &right, ops::Comparison::Eq, true),
                BinaryOp::CNe => ops::compare(&left, &right, ops::Comparison::Ne, true),
                BinaryOp::CGe => ops::compare(&left, &right, ops::Comparison::Ge, true),
                BinaryOp::CGt => ops::compare(&left, &right, ops::Comparison::Gt, true),
                BinaryOp::CLt => ops::compare(&left, &right, ops::Comparison::Lt, true),
                BinaryOp::CLe => ops::compare(&left, &right, ops::Comparison::Le, true),
                BinaryOp::Like => ops::compare(&left, &right, ops::Comparison::Like, false),
                BinaryOp::NotLike => ops::compare(&left, &right, ops::Comparison::NotLike, false),
                BinaryOp::CLike => ops::compare(&left, &right, ops::Comparison::Like, true),
                BinaryOp::CNotLike => ops::compare(&left, &right, ops::Comparison::NotLike, true),
                BinaryOp::Contains => ops::contains(&left, &right, false),
                BinaryOp::NotContains => ops::contains(&left, &right, false)
                    .map(|value| Value::Bool(!matches!(value, Value::Bool(true)))),
                BinaryOp::CContains => ops::contains(&left, &right, true),
                BinaryOp::CNotContains => ops::contains(&left, &right, true)
                    .map(|value| Value::Bool(!matches!(value, Value::Bool(true)))),
                BinaryOp::In => ops::contains(&right, &left, false),
                BinaryOp::NotIn => ops::contains(&right, &left, false)
                    .map(|value| Value::Bool(!matches!(value, Value::Bool(true)))),
                BinaryOp::CIn => ops::contains(&right, &left, true),
                BinaryOp::CNotIn => ops::contains(&right, &left, true)
                    .map(|value| Value::Bool(!matches!(value, Value::Bool(true)))),
                BinaryOp::Shl => ops::shift(&left, &right, true),
                BinaryOp::Shr => ops::shift(&left, &right, false),
                BinaryOp::Band => ops::bitwise(&left, &right, "band"),
                BinaryOp::Bor => ops::bitwise(&left, &right, "bor"),
                BinaryOp::Bxor => ops::bitwise(&left, &right, "bxor"),
                BinaryOp::And => ops::logical(&left, &right, "and"),
                BinaryOp::Or => ops::logical(&left, &right, "or"),
                BinaryOp::Xor => ops::logical(&left, &right, "xor"),
                BinaryOp::Is => type_is(&left, &right, false),
                BinaryOp::IsNot => type_is(&left, &right, true),
                BinaryOp::As => type_is(&left, &right, false).map(|value| {
                    if matches!(value, Value::Bool(true)) {
                        left
                    } else {
                        Value::Null
                    }
                }),
            }
        }
    }
}

fn index_value(target: &Value, index: &Value) -> Result<Value, RuntimeError> {
    let Value::Array(items) = target else {
        return Err(RuntimeError::NotImplemented {
            feature: "indexing other than arrays",
        });
    };
    match index {
        Value::Int32(at) => {
            let at = if *at < 0 {
                items.len() as i32 + *at
            } else {
                *at
            };
            usize::try_from(at)
                .ok()
                .and_then(|at| items.get(at).cloned())
                .ok_or(RuntimeError::NotImplemented {
                    feature: "index out of range",
                })
        }
        Value::Array(indices) => {
            let mut values = Vec::new();
            for index in indices {
                values.push(index_value(target, index)?);
            }
            Ok(Value::Array(values))
        }
        _ => Err(RuntimeError::NotImplemented {
            feature: "index of this type",
        }),
    }
}

fn append(old: Option<&Value>, value: Value) -> Result<Value, RuntimeError> {
    match (old, value) {
        (None, value) => Ok(Value::Array(vec![value])),
        (Some(Value::Array(items)), Value::Array(mut more)) => {
            let mut items = items.clone();
            items.append(&mut more);
            Ok(Value::Array(items))
        }
        (Some(Value::Array(items)), value) => {
            let mut items = items.clone();
            items.push(value);
            Ok(Value::Array(items))
        }
        (Some(Value::Int32(a)), Value::Int32(b)) => a.checked_add(b).map_or(
            Err(RuntimeError::NotImplemented {
                feature: "Int32 overflow (promotion to Double)",
            }),
            |value| Ok(Value::Int32(value)),
        ),
        (Some(Value::String(a)), Value::String(b)) => Ok(Value::String(format!("{a}{b}"))),
        _ => Err(RuntimeError::NotImplemented {
            feature: "compound assignment on these values",
        }),
    }
}

fn strip_scope(name: &str) -> &str {
    for scope in ["global:", "script:", "local:", "private:", "using:"] {
        if let Some(rest) = name.strip_prefix(scope) {
            return rest;
        }
    }
    name
}

fn lookup(name: &str, variables: &HashMap<String, Value>, last_success: bool) -> Option<Value> {
    match name {
        "null" => return Some(Value::Null),
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        "?" => return Some(Value::Bool(last_success)),
        "_" => return Some(Value::Null),
        _ => {}
    }
    if let Some(rest) = name.strip_prefix("env:") {
        if let Some(value) = variables.get(name) {
            return Some(value.clone());
        }
        return std::env::var(rest).ok().map(Value::String);
    }
    let key = strip_scope(name);
    variables.get(key).cloned()
}

fn type_full_name(name: &str) -> &'static str {
    match name.to_ascii_lowercase().as_str() {
        "int" | "int32" => "System.Int32",
        "long" | "int64" => "System.Int64",
        "uint" | "uint32" => "System.UInt32",
        "ulong" | "uint64" => "System.UInt64",
        "short" | "int16" => "System.Int16",
        "ushort" | "uint16" => "System.UInt16",
        "byte" => "System.Byte",
        "sbyte" => "System.SByte",
        "string" => "System.String",
        "bool" | "boolean" => "System.Boolean",
        "double" => "System.Double",
        "decimal" => "System.Decimal",
        "biginteger" => "System.Numerics.BigInteger",
        _ => "System.Object",
    }
}

fn type_is(left: &Value, right: &Value, negate: bool) -> Result<Value, RuntimeError> {
    let Value::Type(full) = right else {
        return Err(RuntimeError::NotImplemented {
            feature: "-is / -as with a non-type right operand",
        });
    };
    let same = left.type_name() == full;
    Ok(Value::Bool(if negate { !same } else { same }))
}

fn number_value(value: &NumberLiteral) -> Value {
    match value {
        NumberLiteral::Int32(value) => Value::Int32(*value),
        NumberLiteral::Int64(value) => Value::Int64(*value),
        NumberLiteral::UInt32(value) => Value::UInt32(*value),
        NumberLiteral::UInt64(value) => Value::UInt64(*value),
        NumberLiteral::Int16(value) => Value::Int16(*value),
        NumberLiteral::UInt16(value) => Value::UInt16(*value),
        NumberLiteral::Byte(value) => Value::Byte(*value),
        NumberLiteral::SByte(value) => Value::SByte(*value),
        NumberLiteral::Decimal(value) => Value::Decimal(value.clone()),
        NumberLiteral::Double(value) => Value::Double(*value),
        NumberLiteral::BigInteger(value) => Value::BigInteger(value.clone()),
    }
}

fn member(value: &Value, name: &str) -> Result<Value, RuntimeError> {
    match (value, name) {
        (_, "GetType") => Ok(Value::Type(value.type_name().to_owned())),
        (Value::Type(full), "FullName") => Ok(Value::String(full.clone())),
        (Value::Type(full), "Name") => Ok(Value::String(
            full.rsplit('.').next().unwrap_or(full).to_owned(),
        )),
        (Value::Array(items), "Count" | "Length") => Ok(Value::Int32(items.len() as i32)),
        (Value::Object(entries), name) => entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .ok_or(RuntimeError::NotImplemented {
                feature: "missing object property",
            }),
        _ => Err(RuntimeError::NotImplemented {
            feature: "member access other than GetType().FullName / .Name",
        }),
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
    fn runs_an_ast_that_was_parsed_once() {
        let ast = parse("1 + 2 * 3").unwrap();
        let mut out = Vec::new();
        Engine::new().run_ast(&ast, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "7\n");
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
        let (result, out) = run("1\n2\nif (1) { 3 }");
        assert_eq!(
            result.unwrap_err().missing_feature(),
            Some("statement keywords (if, foreach, function, ...)")
        );
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
