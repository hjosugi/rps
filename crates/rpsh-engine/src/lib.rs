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

use rpsh_runtime::{PsString, RuntimeError, Value, ops};
use rpsh_syntax::{
    BinaryOp, CommandArgument, Expr, ExprKind, NumberLiteral, Parameter, ParseError,
    PipelineCommand, ScriptBlockAst, Statement, UnaryOp, parse,
};

/// Runs scripts.
#[derive(Debug, Default)]
pub struct Engine {
    variables: HashMap<String, Value>,
    functions: HashMap<String, FunctionDef>,
    last_success: bool,
    /// While running a pipeline stage, output is collected here instead of
    /// being written to the host.
    capture: Option<Vec<Value>>,
}

type BoundArguments = (Vec<Value>, Vec<(String, Value)>);

#[derive(Debug, Clone)]
struct FunctionDef {
    parameters: Vec<Parameter>,
    body: Vec<Statement>,
    filter: bool,
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
                let old = self.variables.get(key).cloned().unwrap_or(Value::Null);
                let updated = ops::add(&old, &value)?;
                self.variables.insert(key.to_owned(), updated);
                self.last_success = true;
                Ok(Flow::Normal)
            }
            Statement::CompoundAssign {
                name, op, value, ..
            } => {
                let value = evaluate(value, &mut self.variables, self.last_success)?;
                let key = strip_scope(name);
                let old = self.variables.get(key).cloned().unwrap_or(Value::Null);
                let updated = match op {
                    BinaryOp::Subtract => ops::subtract(&old, &value)?,
                    BinaryOp::Multiply => ops::multiply(&old, &value)?,
                    BinaryOp::Divide => ops::divide(&old, &value)?,
                    BinaryOp::Remainder => ops::remainder(&old, &value)?,
                    _ => {
                        return Err(RuntimeError::NotImplemented {
                            feature: "this compound assignment operator",
                        }
                        .into());
                    }
                };
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
            Statement::MemberAssign {
                target,
                name,
                value,
                ..
            } => {
                let value = evaluate(value, &mut self.variables, self.last_success)?;
                let key = assignment_key(target)?;
                let mut entries = match self.variables.get(key) {
                    Some(Value::Object(entries)) => entries.as_ref().clone(),
                    _ => {
                        return Err(RuntimeError::NotImplemented {
                            feature: "member assignment on a value that is not an object",
                        }
                        .into());
                    }
                };
                if let Some(entry) = entries
                    .iter_mut()
                    .find(|(entry, _)| entry.eq_ignore_ascii_case(name))
                {
                    entry.1 = value;
                } else {
                    entries.push((name.clone(), value));
                }
                self.variables
                    .insert(key.to_owned(), Value::object(entries));
                self.last_success = true;
                Ok(Flow::Normal)
            }
            Statement::IndexAssign {
                target,
                index,
                value,
                ..
            } => {
                let value = evaluate(value, &mut self.variables, self.last_success)?;
                let index = evaluate(index, &mut self.variables, self.last_success)?;
                let key = assignment_key(target)?;
                let mut entries = match self.variables.get(key) {
                    Some(Value::Object(entries)) => entries.as_ref().clone(),
                    _ => {
                        return Err(RuntimeError::NotImplemented {
                            feature: "index assignment on a value that is not an object",
                        }
                        .into());
                    }
                };
                let index = index.to_string();
                if let Some(entry) = entries
                    .iter_mut()
                    .find(|(entry, _)| entry.eq_ignore_ascii_case(&index))
                {
                    entry.1 = value;
                } else {
                    entries.push((index, value));
                }
                self.variables
                    .insert(key.to_owned(), Value::object(entries));
                self.last_success = true;
                Ok(Flow::Normal)
            }
            Statement::Function {
                name,
                parameters,
                body,
                filter,
                ..
            } => {
                self.functions.insert(
                    name.to_ascii_lowercase(),
                    FunctionDef {
                        parameters: parameters.clone(),
                        body: body.clone(),
                        filter: *filter,
                    },
                );
                Ok(Flow::Normal)
            }
            Statement::Command {
                name, arguments, ..
            } => self.call_function(name, arguments, out),
            Statement::Pipeline { commands, .. } => self.exec_pipeline(commands, out),
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
    fn call_function(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let function = self.function(name)?;
        let (positional, named) = self.evaluate_arguments(arguments)?;
        let saved = std::mem::take(&mut self.variables);
        let mut local = saved.clone();
        let bound = self.bind_parameters(&function, &positional, &named, &mut local)?;
        local.insert(
            "args".to_owned(),
            Value::array(positional[bound..].to_vec()),
        );
        if function.filter {
            local.entry("_".to_owned()).or_insert(Value::Null);
        }
        self.variables = local;
        let flow = self.exec_block(&function.body, out);
        self.variables = saved;
        Ok(match flow? {
            Flow::Return(value) => {
                if let Some(value) = value {
                    self.write_value(value, out)?;
                }
                Flow::Normal
            }
            Flow::Break(_) | Flow::Continue(_) => Flow::Normal,
            flow => flow,
        })
    }

    fn exec_pipeline(
        &mut self,
        commands: &[PipelineCommand],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let mut input = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            let last = index + 1 == commands.len();
            let outputs = match command {
                PipelineCommand::Expression(expr) => {
                    let value = evaluate(expr, &mut self.variables, self.last_success)?;
                    flatten_value(value)
                }
                PipelineCommand::Command { name, arguments } => {
                    self.run_command(name, arguments, input.clone())?
                }
            };
            if last {
                for value in outputs {
                    self.write_value(value, out)?;
                }
                return Ok(Flow::Normal);
            }
            input = outputs;
        }
        Ok(Flow::Normal)
    }

    /// Runs a command for a pipeline stage and collects its output.
    fn run_command(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        input: Vec<Value>,
    ) -> Result<Vec<Value>, EngineError> {
        let function = self.function(name)?;
        let (positional, named) = self.evaluate_arguments(arguments)?;
        let mut outputs = Vec::new();
        if function.filter {
            for item in input {
                let saved = std::mem::take(&mut self.variables);
                let mut local = saved.clone();
                let bound = self.bind_parameters(&function, &positional, &named, &mut local)?;
                local.insert(
                    "args".to_owned(),
                    Value::array(positional[bound..].to_vec()),
                );
                local.insert("_".to_owned(), item);
                self.variables = local;
                self.capture = Some(Vec::new());
                let flow = self.exec_block(&function.body, &mut capture_sink());
                let captured = self.capture.take().unwrap_or_default();
                self.variables = saved;
                outputs.extend(captured);
                match flow? {
                    Flow::Return(Some(value)) => outputs.extend(flatten_value(value)),
                    Flow::Return(None) | Flow::Normal => {}
                    Flow::Break(_) | Flow::Continue(_) => {}
                    _ => {
                        return Err(RuntimeError::NotImplemented {
                            feature: "exit or throw inside a pipeline filter",
                        }
                        .into());
                    }
                }
            }
        } else {
            let saved = std::mem::take(&mut self.variables);
            let mut local = saved.clone();
            let bound = self.bind_parameters(&function, &positional, &named, &mut local)?;
            local.insert(
                "args".to_owned(),
                Value::array(positional[bound..].to_vec()),
            );
            local.insert("input".to_owned(), Value::array(input));
            self.variables = local;
            self.capture = Some(Vec::new());
            let flow = self.exec_block(&function.body, &mut capture_sink());
            let captured = self.capture.take().unwrap_or_default();
            self.variables = saved;
            outputs.extend(captured);
            match flow? {
                Flow::Return(Some(value)) => outputs.extend(flatten_value(value)),
                Flow::Return(None) | Flow::Normal => {}
                Flow::Break(_) | Flow::Continue(_) => {}
                _ => {
                    return Err(RuntimeError::NotImplemented {
                        feature: "exit or throw inside a pipeline function",
                    }
                    .into());
                }
            }
        }
        Ok(outputs)
    }

    fn function(&self, name: &str) -> Result<FunctionDef, EngineError> {
        self.functions
            .get(&name.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| {
                RuntimeError::NotImplemented {
                    feature: "command invocation other than a defined function",
                }
                .into()
            })
    }

    fn evaluate_arguments(
        &mut self,
        arguments: &[CommandArgument],
    ) -> Result<BoundArguments, EngineError> {
        let mut positional = Vec::new();
        let mut named = Vec::new();
        for argument in arguments {
            match argument {
                CommandArgument::Value(expr) => {
                    positional.push(evaluate(expr, &mut self.variables, self.last_success)?);
                }
                CommandArgument::Bareword(word) => {
                    positional.push(Value::String(word.clone().into()));
                }
                CommandArgument::Parameter { name, value } => {
                    let value = match value {
                        Some(expr) => evaluate(expr, &mut self.variables, self.last_success)?,
                        None => Value::Bool(true),
                    };
                    named.push((name.clone(), value));
                }
            }
        }
        Ok((positional, named))
    }

    /// Binds declared parameters into `local`; returns how many positional
    /// arguments it consumed.
    fn bind_parameters(
        &mut self,
        function: &FunctionDef,
        positional: &[Value],
        named: &[(String, Value)],
        local: &mut HashMap<String, Value>,
    ) -> Result<usize, EngineError> {
        let mut bound = 0;
        for parameter in &function.parameters {
            let value = if let Some((_, value)) = named
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&parameter.name))
            {
                value.clone()
            } else if let Some(value) = positional.get(bound) {
                bound += 1;
                value.clone()
            } else if let Some(default) = &parameter.default {
                evaluate(default, local, self.last_success)?
            } else {
                Value::Null
            };
            let value = match &parameter.type_name {
                Some(type_name) => cast_to(type_name, value)?,
                None => value,
            };
            local.insert(parameter.name.clone(), value);
        }
        Ok(bound)
    }

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

    fn write_value(&mut self, value: Value, out: &mut dyn Write) -> Result<(), EngineError> {
        // The host's default object formatting (Format-Table) is M6; until
        // then an object on the output stream is refused, never printed as
        // `@{...}` where pwsh would draw a table.
        let has_object = match &value {
            Value::Object(_) => true,
            Value::Array(items) => items.iter().any(|item| matches!(item, Value::Object(_))),
            _ => false,
        };
        if has_object {
            return Err(RuntimeError::NotImplemented {
                feature: "default object formatting",
            }
            .into());
        }
        if let Some(capture) = &mut self.capture {
            match value {
                Value::Array(items) => capture.extend(items.iter().cloned()),
                Value::Null | Value::AutomationNull => {}
                value => capture.push(value),
            }
            return Ok(());
        }
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
            cast_to(type_name, value)
        }
        ExprKind::Member { target, name } => {
            member(&evaluate(target, variables, last_success)?, name)
        }
        ExprKind::InvokeMember {
            target,
            name,
            arguments,
        } => {
            let target = evaluate(target, variables, last_success)?;
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(evaluate(argument, variables, last_success)?);
            }
            invoke_member(&target, name, &values)
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
                BinaryOp::As => as_operator(&left, &right),
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
    let old = lookup(name, variables, last_success).unwrap_or(Value::Null);
    if matches!(old, Value::String(_)) {
        return Err(RuntimeError::NotImplemented {
            feature: "incrementing a string",
        });
    }
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
        (Value::Object(entries), Value::String(key)) => {
            let key = key.to_string_lossy();
            Ok(entries
                .iter()
                .find(|(entry, _)| entry.eq_ignore_ascii_case(&key))
                .map_or(Value::Null, |(_, value)| value.clone()))
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

fn flatten_value(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.as_ref().clone(),
        Value::Null | Value::AutomationNull => Vec::new(),
        value => vec![value],
    }
}

fn capture_sink() -> io::Sink {
    io::sink()
}

fn assignment_key(target: &Expr) -> Result<&str, RuntimeError> {
    match &target.kind {
        ExprKind::Variable(name) => Ok(strip_scope(name)),
        _ => Err(RuntimeError::NotImplemented {
            feature: "assignment through a non-variable target",
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

/// `[type]value`, the subset of `LanguagePrimitives.ConvertTo` the skeleton
/// needs. Failing conversions are refused rather than approximated.
fn cast_to(type_name: &str, value: Value) -> Result<Value, RuntimeError> {
    match type_name.to_ascii_lowercase().as_str() {
        "pscustomobject"
        | "system.management.automation.pscustomobject"
        | "ordered"
        | "hashtable"
        | "system.collections.hashtable"
        | "ordereddictionary"
        | "system.collections.specialized.ordereddictionary" => Ok(value),
        "int" | "int32" | "system.int32" => Ok(Value::Int32(convert_int32(&value)?)),
        "long" | "int64" | "system.int64" => Ok(Value::Int64(convert_int64(&value)?)),
        "double" | "system.double" => Ok(Value::Double(convert_double(&value)?)),
        "string" | "system.string" => Ok(Value::String(value.to_string().into())),
        "bool" | "boolean" | "system.boolean" => Ok(Value::Bool(convert_bool(&value))),
        "char" | "system.char" => convert_char(&value),
        _ => Err(RuntimeError::NotImplemented {
            feature: "type casts other than to numbers, bool, char, string and pscustomobject",
        }),
    }
}

fn convert_int64(value: &Value) -> Result<i64, RuntimeError> {
    match value {
        Value::Null | Value::AutomationNull => Ok(0),
        Value::Bool(value) => Ok(i64::from(*value)),
        Value::Char(unit) => Ok(i64::from(*unit)),
        Value::Int32(value) => Ok(i64::from(*value)),
        Value::Int64(value) => Ok(*value),
        Value::UInt32(value) => Ok(i64::from(*value)),
        Value::Int16(value) => Ok(i64::from(*value)),
        Value::UInt16(value) => Ok(i64::from(*value)),
        Value::Byte(value) => Ok(i64::from(*value)),
        Value::SByte(value) => Ok(i64::from(*value)),
        Value::UInt64(value) => i64::try_from(*value).map_err(|_| RuntimeError::NotImplemented {
            feature: "integer conversion overflow",
        }),
        Value::Double(value) => {
            let rounded = value.round_ties_even();
            if rounded >= i64::MIN as f64 && rounded <= i64::MAX as f64 {
                Ok(rounded as i64)
            } else {
                Err(RuntimeError::NotImplemented {
                    feature: "integer conversion overflow",
                })
            }
        }
        Value::String(text) => parse_integer_text(&text.to_string_lossy())
            .ok_or(RuntimeError::NotImplemented {
                feature: "cannot convert a string to an integer",
            })
            .and_then(|value| {
                i64::try_from(value).map_err(|_| RuntimeError::NotImplemented {
                    feature: "integer conversion overflow",
                })
            }),
        _ => Err(RuntimeError::NotImplemented {
            feature: "conversion to an integer",
        }),
    }
}

fn convert_int32(value: &Value) -> Result<i32, RuntimeError> {
    i32::try_from(convert_int64(value)?).map_err(|_| RuntimeError::NotImplemented {
        feature: "integer conversion overflow",
    })
}

fn convert_double(value: &Value) -> Result<f64, RuntimeError> {
    match value {
        Value::Null | Value::AutomationNull => Ok(0.0),
        Value::Bool(value) => Ok(if *value { 1.0 } else { 0.0 }),
        Value::Char(unit) => Ok(f64::from(*unit)),
        Value::Int32(value) => Ok(f64::from(*value)),
        Value::Int64(value) => Ok(*value as f64),
        Value::UInt32(value) => Ok(f64::from(*value)),
        Value::UInt64(value) => Ok(*value as f64),
        Value::Int16(value) => Ok(f64::from(*value)),
        Value::UInt16(value) => Ok(f64::from(*value)),
        Value::Byte(value) => Ok(f64::from(*value)),
        Value::SByte(value) => Ok(f64::from(*value)),
        Value::Double(value) => Ok(*value),
        Value::String(text) => {
            text.to_string_lossy()
                .trim()
                .parse()
                .map_err(|_| RuntimeError::NotImplemented {
                    feature: "cannot convert a string to a double",
                })
        }
        _ => Err(RuntimeError::NotImplemented {
            feature: "conversion to a double",
        }),
    }
}

/// `[bool]`: an empty string or zero is false, a one-element array uses that
/// element, and any longer array is true.
fn convert_bool(value: &Value) -> bool {
    match value {
        Value::Null | Value::AutomationNull => false,
        Value::Bool(value) => *value,
        Value::Char(unit) => *unit != 0,
        Value::Int32(value) => *value != 0,
        Value::Int64(value) => *value != 0,
        Value::UInt32(value) => *value != 0,
        Value::UInt64(value) => *value != 0,
        Value::Int16(value) => *value != 0,
        Value::UInt16(value) => *value != 0,
        Value::Byte(value) => *value != 0,
        Value::SByte(value) => *value != 0,
        Value::Double(value) => *value != 0.0,
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => match items.len() {
            0 => false,
            1 => convert_bool(&items[0]),
            _ => true,
        },
        Value::Decimal(_)
        | Value::BigInteger(_)
        | Value::ScriptBlock(_)
        | Value::Object(_)
        | Value::Type(_) => true,
        _ => true,
    }
}

fn convert_char(value: &Value) -> Result<Value, RuntimeError> {
    match value {
        Value::Char(unit) => Ok(Value::Char(*unit)),
        Value::Int32(value) => {
            u16::try_from(*value)
                .map(Value::Char)
                .map_err(|_| RuntimeError::NotImplemented {
                    feature: "integer out of the char range",
                })
        }
        Value::String(text) if text.len() == 1 => Ok(Value::Char(text.unit(0).unwrap_or_default())),
        _ => Err(RuntimeError::NotImplemented {
            feature: "conversion to a char",
        }),
    }
}

fn parse_integer_text(text: &str) -> Option<i128> {
    let text = text.trim();
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let parsed = if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        i128::from_str_radix(hex, 16).ok()?
    } else if let Some(binary) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
        i128::from_str_radix(binary, 2).ok()?
    } else {
        body.parse::<i128>().ok()?
    };
    Some(if negative { -parsed } else { parsed })
}

/// `-as` converts when it can and yields `$null` when it cannot; it never
/// raises a conversion error.
fn as_operator(value: &Value, type_value: &Value) -> Result<Value, RuntimeError> {
    let Value::Type(name) = type_value else {
        return Err(RuntimeError::NotImplemented {
            feature: "-as with a non-type right operand",
        });
    };
    let name = name.to_string_lossy();
    Ok(cast_to(&name, value.clone()).unwrap_or(Value::Null))
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

fn invoke_member(target: &Value, name: &str, arguments: &[Value]) -> Result<Value, RuntimeError> {
    if name.eq_ignore_ascii_case("GetType") && arguments.is_empty() {
        return Ok(Value::Type(target.type_name().to_owned().into()));
    }
    match target {
        Value::String(text) => string_method(text, name, arguments),
        Value::Object(entries) => object_method(entries, name, arguments),
        _ => Err(RuntimeError::NotImplemented {
            feature: "method invocation on this type",
        }),
    }
}

fn string_method(text: &PsString, name: &str, arguments: &[Value]) -> Result<Value, RuntimeError> {
    let units: Vec<u16> = text.units().collect();
    match name.to_ascii_lowercase().as_str() {
        "substring" => {
            let start = int_argument(arguments.first())?;
            let start = usize::try_from(start).map_err(|_| RuntimeError::NotImplemented {
                feature: "Substring out of range",
            })?;
            let length = match arguments.get(1) {
                Some(value) => usize::try_from(int_argument(Some(value))?).map_err(|_| {
                    RuntimeError::NotImplemented {
                        feature: "Substring out of range",
                    }
                })?,
                None => units.len().saturating_sub(start),
            };
            let end = start
                .checked_add(length)
                .filter(|end| *end <= units.len())
                .ok_or(RuntimeError::NotImplemented {
                    feature: "Substring out of range",
                })?;
            Ok(Value::String(PsString::from_units(
                units[start..end].to_vec(),
            )))
        }
        "indexof" => {
            let needle = needle_units(arguments.first())?;
            Ok(Value::Int32(
                find_units(&units, &needle).map_or(-1, |at| at as i32),
            ))
        }
        "contains" => {
            let needle = needle_units(arguments.first())?;
            Ok(Value::Bool(find_units(&units, &needle).is_some()))
        }
        "toupper" | "tolower" => {
            let ascii = text.as_ascii().ok_or(RuntimeError::NotImplemented {
                feature: "culture-aware case conversion (ICU)",
            })?;
            let result = if name.eq_ignore_ascii_case("toupper") {
                ascii.to_ascii_uppercase()
            } else {
                ascii.to_ascii_lowercase()
            };
            Ok(Value::String(result.into()))
        }
        "replace" => {
            let old = needle_units(arguments.first())?;
            let new = needle_units(arguments.get(1))?;
            Ok(Value::String(replace_units(&units, &old, &new)))
        }
        _ => Err(RuntimeError::NotImplemented {
            feature: "this string method",
        }),
    }
}

fn object_method(
    entries: &[(String, Value)],
    name: &str,
    arguments: &[Value],
) -> Result<Value, RuntimeError> {
    match name.to_ascii_lowercase().as_str() {
        "containskey" => {
            let Some(key) = arguments.first() else {
                return Err(RuntimeError::NotImplemented {
                    feature: "ContainsKey without an argument",
                });
            };
            let key = key.to_string();
            Ok(Value::Bool(
                entries
                    .iter()
                    .any(|(entry, _)| entry.eq_ignore_ascii_case(&key)),
            ))
        }
        _ => Err(RuntimeError::NotImplemented {
            feature: "this object method",
        }),
    }
}

fn int_argument(value: Option<&Value>) -> Result<i32, RuntimeError> {
    let value = value.ok_or(RuntimeError::NotImplemented {
        feature: "method argument missing",
    })?;
    convert_int32(value)
}

fn needle_units(value: Option<&Value>) -> Result<Vec<u16>, RuntimeError> {
    match value {
        Some(Value::Char(unit)) => Ok(vec![*unit]),
        Some(Value::String(text)) => Ok(text.units().collect()),
        _ => Err(RuntimeError::NotImplemented {
            feature: "string method argument",
        }),
    }
}

fn find_units(haystack: &[u16], needle: &[u16]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn replace_units(haystack: &[u16], needle: &[u16], replacement: &[u16]) -> PsString {
    if needle.is_empty() {
        return PsString::from_units(haystack.to_vec());
    }
    let mut result = Vec::new();
    let mut at = 0;
    while at < haystack.len() {
        if haystack[at..].starts_with(needle) {
            result.extend_from_slice(replacement);
            at += needle.len();
        } else {
            result.push(haystack[at]);
            at += 1;
        }
    }
    PsString::from_units(result)
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
        Ok(Value::Null)
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
            if name.eq_ignore_ascii_case("Count") {
                return Ok(Value::Int32(entries.len() as i32));
            }
            if name.eq_ignore_ascii_case("Keys") {
                return Ok(Value::array(
                    entries
                        .iter()
                        .map(|(key, _)| Value::String(key.clone().into()))
                        .collect(),
                ));
            }
            if name.eq_ignore_ascii_case("Values") {
                return Ok(Value::array(
                    entries.iter().map(|(_, value)| value.clone()).collect(),
                ));
            }
            Ok(entries
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map_or(Value::Null, |(_, value)| value.clone()))
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
    fn hashtables_index_assign_and_call_methods() {
        let (result, out) =
            run("$h = @{ a = 1 }; $h['a']; $h.b = 2; $h.ContainsKey('B'); $h.Keys -join ','");
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "1\nTrue\na,b\n");
    }

    #[test]
    fn as_operator_converts_or_yields_null() {
        let (result, out) = run("'5' -as [int]; 'x' -as [int]; 1 -as [string]");
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "5\n1\n");
    }

    #[test]
    fn compound_assignment_and_increment_work() {
        let (result, out) = run("$x = 5; $x -= 2; $x\n\
             $y = 5; $y *= 2; $y\n\
             $z = 5; $z /= 2; $z\n\
             $w = 5; $w %= 2; $w\n\
             $s = 'ab'; $s += 'cd'; $s\n\
             $a = 1, 2; $a += 3; $a\n\
             $n = $null; $n++; $n");
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "3\n10\n2.5\n1\nabcd\n1\n2\n3\n1\n");
    }

    #[test]
    fn casts_convert_the_common_types() {
        let (result, out) = run(
            "[int]2.5; [int]3.5; [int]'0x10'; [bool]''; [bool]'false'; [bool]@(0); [string]12; [char]65",
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(out, "2\n4\n16\nFalse\nTrue\nFalse\n12\nA\n");
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
