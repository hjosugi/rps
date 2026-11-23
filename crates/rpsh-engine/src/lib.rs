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
        match self.exec_block(&script.statements, out)? {
            Flow::Normal => Ok(0),
            Flow::Exit(code) => Ok(code),
            Flow::Return(value) => {
                if let Some(value) = value {
                    self.write_value(value, out)?;
                }
                Ok(0)
            }
            Flow::Throw(_) => Err(RuntimeError::NotImplemented {
                feature: "uncaught throw",
            }
            .into()),
            // `break` and `continue` outside a loop end the script quietly, as
            // upstream's top-level script block does.
            Flow::Break(_) | Flow::Continue(_) => Ok(0),
        }
    }

    fn exec_block(
        &mut self,
        statements: &[Statement],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        for statement in statements {
            let flow = self.exec_statement(statement, out)?;
            if !matches!(flow, Flow::Normal) {
                return Ok(flow);
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_statement(
        &mut self,
        statement: &Statement,
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        match statement {
            Statement::Expression(expr) => {
                let value = evaluate(expr, &mut self.variables, self.last_success)?;
                if !matches!(
                    expr.kind,
                    ExprKind::Unary {
                        op: UnaryOp::Increment
                            | UnaryOp::Decrement
                            | UnaryOp::PostIncrement
                            | UnaryOp::PostDecrement,
                        ..
                    }
                ) {
                    self.write_value(value, out)?;
                }
                self.last_success = true;
                Ok(Flow::Normal)
            }
            Statement::CoalesceAssign { name, value, .. } => {
                let key = strip_scope(name);
                if matches!(self.variables.get(key), None | Some(Value::Null)) {
                    let value = evaluate(value, &mut self.variables, self.last_success)?;
                    self.variables.insert(key.to_owned(), value);
                }
                self.last_success = true;
                Ok(Flow::Normal)
            }
            Statement::AppendAssign { name, value, .. } => {
                let value = evaluate(value, &mut self.variables, self.last_success)?;
                let key = strip_scope(name);
                let updated = append(self.variables.get(key), value)?;
                self.variables.insert(key.to_owned(), updated);
                self.last_success = true;
                Ok(Flow::Normal)
            }
            Statement::Assign { name, value, .. } => {
                let value = evaluate(value, &mut self.variables, self.last_success)?;
                let key = strip_scope(name);
                self.variables.insert(key.to_owned(), value);
                self.last_success = true;
                Ok(Flow::Normal)
            }
            Statement::If {
                condition,
                then,
                elseif,
                otherwise,
                ..
            } => {
                let value = evaluate(condition, &mut self.variables, self.last_success)?;
                if ops::is_true(&value) {
                    self.last_success = true;
                    return self.exec_block(then, out);
                }
                for (condition, body) in elseif {
                    let value = evaluate(condition, &mut self.variables, self.last_success)?;
                    if ops::is_true(&value) {
                        self.last_success = true;
                        return self.exec_block(body, out);
                    }
                }
                self.last_success = true;
                self.exec_block(otherwise, out)
            }
            Statement::While {
                label,
                condition,
                body,
                ..
            } => self.exec_loop(label, Some(condition), None, body, out),
            Statement::For {
                label,
                init,
                condition,
                step,
                body,
                ..
            } => {
                if let Some(init) = init {
                    let flow = self.exec_statement(init, out)?;
                    if !matches!(flow, Flow::Normal) {
                        return Ok(flow);
                    }
                }
                self.exec_loop(label, condition.as_ref(), step.as_deref(), body, out)
            }
            Statement::Foreach {
                label,
                name,
                collection,
                body,
                ..
            } => {
                let value = evaluate(collection, &mut self.variables, self.last_success)?;
                let items = match value {
                    Value::Array(items) => (*items).clone(),
                    Value::Null => Vec::new(),
                    value => vec![value],
                };
                let key = strip_scope(name).to_owned();
                for item in items {
                    self.variables.insert(key.clone(), item);
                    match self.exec_block(body, out)? {
                        Flow::Normal => {}
                        Flow::Break(None) => break,
                        Flow::Break(Some(target)) if label.as_deref() == Some(target.as_str()) => {
                            break;
                        }
                        Flow::Continue(None) => {}
                        Flow::Continue(Some(target))
                            if label.as_deref() == Some(target.as_str()) => {}
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }
            Statement::Do {
                label,
                body,
                condition,
                until,
                ..
            } => {
                loop {
                    match self.exec_block(body, out)? {
                        Flow::Normal => {}
                        Flow::Break(None) => break,
                        Flow::Break(Some(target)) if label.as_deref() == Some(target.as_str()) => {
                            break;
                        }
                        Flow::Continue(None) => {}
                        Flow::Continue(Some(target))
                            if label.as_deref() == Some(target.as_str()) => {}
                        other => return Ok(other),
                    }
                    let value = evaluate(condition, &mut self.variables, self.last_success)?;
                    let truth = ops::is_true(&value);
                    if (*until && truth) || (!*until && !truth) {
                        break;
                    }
                }
                Ok(Flow::Normal)
            }
            Statement::Try {
                body,
                catches,
                finally,
                ..
            } => {
                let mut flow = match self.exec_block(body, out)? {
                    Flow::Throw(value) => {
                        let mut handled = Flow::Throw(value.clone());
                        for clause in catches {
                            if clause.types.is_empty()
                                || clause.types.iter().any(|kind| catch_matches(kind))
                            {
                                self.variables.insert("_".to_owned(), value);
                                handled = self.exec_block(&clause.body, out)?;
                                break;
                            }
                        }
                        handled
                    }
                    flow => flow,
                };
                if let Some(finally) = finally {
                    let finally = self.exec_block(finally, out)?;
                    if !matches!(finally, Flow::Normal) {
                        flow = finally;
                    }
                }
                Ok(flow)
            }
            Statement::Throw { value, .. } => {
                let value = evaluate(value, &mut self.variables, self.last_success)?;
                Ok(Flow::Throw(value))
            }
            Statement::Return { value, .. } => {
                let value = match value {
                    Some(value) => Some(evaluate(value, &mut self.variables, self.last_success)?),
                    None => None,
                };
                Ok(Flow::Return(value))
            }
            Statement::Break { label, .. } => Ok(Flow::Break(label.clone())),
            Statement::Continue { label, .. } => Ok(Flow::Continue(label.clone())),
            Statement::Exit { code: None, .. } => Ok(Flow::Exit(0)),
            Statement::Exit {
                code: Some(code), ..
            } => match evaluate(code, &mut self.variables, self.last_success)? {
                Value::Int32(code) => Ok(Flow::Exit(code)),
                _ => Err(RuntimeError::NotImplemented {
                    feature: "exit with a value other than Int32",
                }
                .into()),
            },
        }
    }

    /// Runs a loop body with the `break`/`continue` handling every loop shares.
    /// `step` is `None` for `while`, and the `for` iterator otherwise.
    fn exec_loop(
        &mut self,
        label: &Option<String>,
        condition: Option<&Expr>,
        step: Option<&Statement>,
        body: &[Statement],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        loop {
            if let Some(condition) = condition {
                let value = evaluate(condition, &mut self.variables, self.last_success)?;
                if !ops::is_true(&value) {
                    break;
                }
            }
            match self.exec_block(body, out)? {
                Flow::Normal => {}
                Flow::Break(None) => break,
                Flow::Break(Some(target)) if label.as_deref() == Some(target.as_str()) => break,
                Flow::Continue(None) => {}
                Flow::Continue(Some(target)) if label.as_deref() == Some(target.as_str()) => {}
                other => return Ok(other),
            }
            if let Some(step) = step {
                let flow = self.exec_statement(step, out)?;
                if !matches!(flow, Flow::Normal) {
                    return Ok(flow);
                }
            }
        }
        Ok(Flow::Normal)
    }

    fn write_value(&self, value: Value, out: &mut dyn Write) -> Result<(), EngineError> {
        match value {
            Value::Array(items) => {
                for item in items.iter() {
                    if !item.is_null_like() {
                        writeln!(out, "{item}")?;
                    }
                }
            }
            Value::Null | Value::AutomationNull => {}
            value => writeln!(out, "{value}")?,
        }
        Ok(())
    }
}

/// How a statement finishes. Only `Normal` continues with the next statement;
/// the rest is consumed by the nearest construct that understands it.
#[derive(Debug)]
enum Flow {
    Normal,
    Break(Option<String>),
    Continue(Option<String>),
    Return(Option<Value>),
    Throw(Value),
    Exit(i32),
}

/// A `catch` clause with one of these types takes a value thrown by `throw`.
fn catch_matches(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "system.management.automation.runtimeexception"
            | "runtimeexception"
            | "system.exception"
            | "exception"
    )
}

fn evaluate(
    expr: &Expr,
    variables: &mut HashMap<String, Value>,
    last_success: bool,
) -> Result<Value, RuntimeError> {
    match &expr.kind {
        ExprKind::Int32(value) => Ok(Value::Int32(*value)),
        ExprKind::Number(value) => Ok(number_value(value)),
        ExprKind::String(value) => Ok(Value::String(value.clone().into())),
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
            Ok(Value::String(text.into()))
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
        ExprKind::TypeLiteral(name) => Ok(Value::Type(type_full_name(name).to_owned().into())),
        ExprKind::Coalesce { left, right } => {
            let value = evaluate(left, variables, last_success)?;
            if matches!(value, Value::Null) {
                evaluate(right, variables, last_success)
            } else {
                Ok(value)
            }
        }
        ExprKind::ScriptBlock(statements) => Ok(Value::ScriptBlock(
            format!("{{ {} }}", statements.len()).into(),
        )),
        ExprKind::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(evaluate(item, variables, last_success)?);
            }
            Ok(Value::array(values))
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
            Ok(Value::object(object))
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
                        .to_owned()
                        .into(),
                ))
            } else {
                Err(RuntimeError::NotImplemented {
                    feature: "method invocation other than GetType()",
                })
            }
        }
        ExprKind::Unary {
            op:
                op @ (UnaryOp::Increment
                | UnaryOp::Decrement
                | UnaryOp::PostIncrement
                | UnaryOp::PostDecrement),
            operand,
        } => increment(operand, variables, last_success, *op),
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
                BinaryOp::Match => regex_match(&left, &right, false, false, variables),
                BinaryOp::NotMatch => regex_match(&left, &right, false, true, variables),
                BinaryOp::CMatch => regex_match(&left, &right, true, false, variables),
                BinaryOp::CNotMatch => regex_match(&left, &right, true, true, variables),
                BinaryOp::Replace => regex_replace(&left, &right, false),
                BinaryOp::CReplace => regex_replace(&left, &right, true),
                BinaryOp::Split => regex_split(&left, &right, false),
                BinaryOp::CSplit => regex_split(&left, &right, true),
            }
        }
    }
}

/// `++$x` / `$x++` / `--$x` / `$x--`: a postfix form yields the value before
/// the update, a prefix form the value after it.
fn increment(
    operand: &Expr,
    variables: &mut HashMap<String, Value>,
    last_success: bool,
    op: UnaryOp,
) -> Result<Value, RuntimeError> {
    let ExprKind::Variable(name) = &operand.kind else {
        return Err(RuntimeError::NotImplemented {
            feature: "incrementing anything but a variable",
        });
    };
    let old = lookup(name, variables, last_success).ok_or(RuntimeError::NotImplemented {
        feature: "undefined variable",
    })?;
    let one = Value::Int32(1);
    let updated = match op {
        UnaryOp::Increment | UnaryOp::PostIncrement => ops::add(&old, &one)?,
        UnaryOp::Decrement | UnaryOp::PostDecrement => ops::subtract(&old, &one)?,
        _ => unreachable!("increment is called only for increment operators"),
    };
    variables.insert(strip_scope(name).to_owned(), updated.clone());
    Ok(match op {
        UnaryOp::PostIncrement | UnaryOp::PostDecrement => old,
        _ => updated,
    })
}

fn index_value(target: &Value, index: &Value) -> Result<Value, RuntimeError> {
    match (target, index) {
        (Value::Array(items), Value::Int32(at)) => {
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
        (Value::Array(items), Value::Array(indices)) => {
            let mut values = Vec::new();
            for index in indices.iter() {
                values.push(index_value(&Value::Array(items.clone()), index)?);
            }
            Ok(Value::array(values))
        }
        (Value::String(text), Value::Int32(at)) => {
            let at = if *at < 0 {
                text.len() as i32 + *at
            } else {
                *at
            };
            usize::try_from(at)
                .ok()
                .and_then(|at| text.unit(at))
                .map(Value::Char)
                .ok_or(RuntimeError::NotImplemented {
                    feature: "index out of range",
                })
        }
        (Value::Object(entries), Value::Int32(index)) => {
            let key = index.to_string();
            entries
                .iter()
                .find(|(name, _)| name == &key)
                .map(|(_, value)| value.clone())
                .ok_or(RuntimeError::NotImplemented {
                    feature: "missing object key",
                })
        }
        _ => Err(RuntimeError::NotImplemented {
            feature: "indexing other than arrays",
        }),
    }
}

fn append(old: Option<&Value>, value: Value) -> Result<Value, RuntimeError> {
    match (old, value) {
        (None, value) => Ok(Value::array(vec![value])),
        (Some(Value::Array(items)), Value::Array(more)) => {
            let mut items = items.as_ref().clone();
            items.extend(more.iter().cloned());
            Ok(Value::array(items))
        }
        (Some(Value::Array(items)), value) => {
            let mut items = items.as_ref().clone();
            items.push(value);
            Ok(Value::array(items))
        }
        (Some(Value::Int32(a)), Value::Int32(b)) => a.checked_add(b).map_or(
            Err(RuntimeError::NotImplemented {
                feature: "Int32 overflow (promotion to Double)",
            }),
            |value| Ok(Value::Int32(value)),
        ),
        (Some(Value::String(a)), Value::String(b)) => Ok(Value::String(a.concat(&b))),
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
        "_" => return variables.get("_").cloned().or(Some(Value::Null)),
        _ => {}
    }
    if let Some(rest) = name.strip_prefix("env:") {
        if let Some(value) = variables.get(name) {
            return Some(value.clone());
        }
        return std::env::var(rest)
            .ok()
            .map(|value| Value::String(value.into()));
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
    let same = full.as_ascii() == Some(left.type_name());
    Ok(Value::Bool(if negate { !same } else { same }))
}

fn regex_match(
    left: &Value,
    right: &Value,
    case_sensitive: bool,
    negate: bool,
    variables: &mut HashMap<String, Value>,
) -> Result<Value, RuntimeError> {
    let regex = rpsh_runtime::regex::Regex::new(&right.to_string(), case_sensitive)
        .map_err(|feature| RuntimeError::NotImplemented { feature })?;
    let text = left.to_string();
    let matches = regex.find(&text);
    let mut object = Vec::new();
    if let Some(found) = &matches {
        let chars: Vec<char> = text.chars().collect();
        for (index, capture) in found.groups.iter().enumerate() {
            if let Some((start, end)) = capture {
                let value: String = chars[*start..*end].iter().collect();
                object.push((index.to_string(), Value::String(value.into())));
            }
        }
    }
    variables.insert(
        "Matches".to_owned(),
        if object.is_empty() {
            Value::Null
        } else {
            Value::object(object)
        },
    );
    let matched = matches.is_some();
    Ok(Value::Bool(if negate { !matched } else { matched }))
}

fn regex_replace(left: &Value, right: &Value, case_sensitive: bool) -> Result<Value, RuntimeError> {
    let (pattern, replacement) = match right {
        Value::Array(items) => (
            items.first().map_or_else(String::new, ToString::to_string),
            items.get(1).map_or_else(String::new, ToString::to_string),
        ),
        other => (other.to_string(), String::new()),
    };
    let regex = rpsh_runtime::regex::Regex::new(&pattern, case_sensitive)
        .map_err(|feature| RuntimeError::NotImplemented { feature })?;
    Ok(Value::String(
        regex.replace_all(&left.to_string(), &replacement).into(),
    ))
}

fn regex_split(left: &Value, right: &Value, case_sensitive: bool) -> Result<Value, RuntimeError> {
    let regex = rpsh_runtime::regex::Regex::new(&right.to_string(), case_sensitive)
        .map_err(|feature| RuntimeError::NotImplemented { feature })?;
    Ok(Value::array(
        regex
            .split(&left.to_string())
            .into_iter()
            .map(|text| Value::String(text.into()))
            .collect(),
    ))
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
        NumberLiteral::Decimal(value) => Value::Decimal(value.clone().into()),
        NumberLiteral::Double(value) => Value::Double(*value),
        NumberLiteral::BigInteger(value) => Value::BigInteger(value.clone().into()),
    }
}

/// The type names of a PowerShell object, most specific first. Upstream's
/// `PSObject.TypeNames`. A `[pscustomobject]` is a `PSCustomObject` backed by
/// `System.Object`.
fn pstypenames() -> Value {
    Value::array(vec![
        Value::String("System.Management.Automation.PSCustomObject".into()),
        Value::String("System.Object".into()),
    ])
}

/// The `PSObject` view of an ordered object: `$o.psobject`.
fn psobject_view(value: &Value, entries: &[(String, Value)]) -> Value {
    let properties = entries
        .iter()
        .map(|(name, property)| {
            Value::object(vec![
                ("Name".to_owned(), Value::String(name.clone().into())),
                ("Value".to_owned(), property.clone()),
            ])
        })
        .collect();
    Value::object(vec![
        ("TypeNames".to_owned(), pstypenames()),
        ("BaseObject".to_owned(), value.clone()),
        ("Properties".to_owned(), Value::array(properties)),
        ("Members".to_owned(), Value::array(Vec::new())),
        ("Methods".to_owned(), Value::array(Vec::new())),
        ("ExtendedMembers".to_owned(), Value::array(Vec::new())),
        ("AdaptedMembers".to_owned(), Value::array(Vec::new())),
    ])
}

/// Member enumeration: `$array.Name` yields `Name` from every element.
fn member_enumeration(items: &[Value], name: &str) -> Result<Value, RuntimeError> {
    let mut values = Vec::new();
    let mut found = false;
    for item in items {
        match member(item, name) {
            Ok(Value::Array(more)) => {
                values.extend(more.iter().cloned());
                found = true;
            }
            Ok(value) => {
                values.push(value);
                found = true;
            }
            Err(_) => {}
        }
    }
    if found {
        Ok(Value::array(values))
    } else {
        Err(RuntimeError::NotImplemented {
            feature: "missing object property",
        })
    }
}

fn member(value: &Value, name: &str) -> Result<Value, RuntimeError> {
    if name.eq_ignore_ascii_case("GetType") {
        return Ok(Value::Type(value.type_name().to_owned().into()));
    }
    match value {
        Value::Type(full) if name.eq_ignore_ascii_case("FullName") => {
            Ok(Value::String(full.clone()))
        }
        Value::Type(full) if name.eq_ignore_ascii_case("Name") => {
            let text = full.to_string_lossy();
            Ok(Value::String(
                text.rsplit('.').next().unwrap_or(&text).to_owned().into(),
            ))
        }
        Value::String(text) if name.eq_ignore_ascii_case("Length") => {
            Ok(Value::Int32(text.len() as i32))
        }
        Value::Array(items) if name.eq_ignore_ascii_case("Count") => {
            Ok(Value::Int32(items.len() as i32))
        }
        Value::Array(items) if name.eq_ignore_ascii_case("Length") => {
            Ok(Value::Int32(items.len() as i32))
        }
        Value::Array(items) => member_enumeration(items.as_slice(), name),
        Value::Object(entries) => {
            if name.eq_ignore_ascii_case("psobject") || name.eq_ignore_ascii_case("pscustomobject")
            {
                return Ok(psobject_view(value, entries.as_slice()));
            }
            if name.eq_ignore_ascii_case("pstypenames") {
                return Ok(pstypenames());
            }
            entries
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
                .ok_or(RuntimeError::NotImplemented {
                    feature: "missing object property",
                })
        }
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
        let (result, out) = run("1\n2\nswitch (1) { 1 { 3 } }");
        assert_eq!(
            result.unwrap_err().missing_feature(),
            Some("statement keywords (if, foreach, function, ...)")
        );
        assert_eq!(out, "");
    }

    #[test]
    fn if_selects_the_first_true_branch() {
        let (result, out) =
            run("$n = 7; if ($n -gt 10) { 'big' } elseif ($n -gt 5) { 'medium' } else { 'small' }");
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "medium\n");

        let (_, out) = run("if (@()) { 'yes' } else { 'no' }; if ('0') { 'yes' } else { 'no' }");
        assert_eq!(out, "no\nyes\n");
    }

    #[test]
    fn loops_support_break_and_continue() {
        let (result, out) = run(
            "$n = 0; while ($true) { $n++; if ($n -ge 3) { break } }; $n\n\
             foreach ($x in 'a', 'b', 'c') { if ($x -eq 'b') { continue }; $x }\n\
             for ($i = 0; $i -lt 3; $i++) { $i }\n\
             do { $n-- } until ($n -le 0)\n$n",
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "3\na\nc\n0\n1\n2\n0\n");
    }

    #[test]
    fn a_label_is_carried_out_of_the_inner_loop() {
        let (result, out) = run(
            ":outer foreach ($a in 1..3) { foreach ($b in 1..3) { if ($b -eq 2) { continue outer }; \"$a$b\" } }",
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "11\n21\n31\n");
    }

    #[test]
    fn a_runtime_refusal_keeps_earlier_output() {
        let (result, out) = run("1; 1 + 'a'; 3");
        assert_eq!(
            result.unwrap_err().missing_feature(),
            Some("operators on operand types other than numbers and strings")
        );
        assert_eq!(out, "1\n");
    }
}
