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
use std::path::Path;
use std::process::Stdio;

use rpsh_runtime::{PsString, RuntimeError, Value, ops};
use rpsh_syntax::{
    BinaryOp, CommandArgument, Expr, ExprKind, NumberLiteral, Parameter, ParseError,
    PipelineCommand, ScriptBlockAst, Statement, SwitchClause, UnaryOp, parse,
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
    /// The active `trap` block of the function being run.
    trap: Option<Vec<Statement>>,
    /// Objects written to the host, formatted as one table at the end.
    host_objects: Vec<Value>,
    /// The generator Get-Random draws from; -SetSeed replaces it.
    random: Option<RandomGenerator>,
    /// `$global:`/`$script:` variables, kept across function calls.
    globals: HashMap<String, Value>,
}

/// .NET's seeded `System.Random` (the subtractive generator).
#[derive(Debug, Clone)]
struct RandomGenerator {
    seed_array: [i32; 56],
    inext: i32,
    inextp: i32,
}

impl RandomGenerator {
    fn new(seed: i32) -> Self {
        const MBIG: i32 = i32::MAX;
        const MSEED: i32 = 161803398;
        let mut seed_array = [0i32; 56];
        let subtraction = if seed == i32::MIN {
            i32::MAX
        } else {
            seed.abs()
        };
        let mut mj = MSEED.wrapping_sub(subtraction);
        seed_array[55] = mj;
        let mut mk = 1;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            seed_array[ii as usize] = mk;
            mk = mj.wrapping_sub(mk);
            if mk < 0 {
                mk = mk.wrapping_add(MBIG);
            }
            mj = seed_array[ii as usize];
        }
        for _ in 1..5 {
            for i in 1..56 {
                seed_array[i] = seed_array[i].wrapping_sub(seed_array[1 + (i + 30) % 55]);
                if seed_array[i] < 0 {
                    seed_array[i] = seed_array[i].wrapping_add(MBIG);
                }
            }
        }
        RandomGenerator {
            seed_array,
            inext: 0,
            inextp: 21,
        }
    }

    fn next_value(&mut self) -> i32 {
        loop {
            let mut bytes = [0u8; 4];
            for byte in &mut bytes {
                *byte = self.sample() as u8;
            }
            let value = i32::from_le_bytes(bytes);
            if value != i32::MAX {
                return if value < 0 {
                    value.wrapping_add(i32::MAX)
                } else {
                    value
                };
            }
        }
    }

    fn next_double(&mut self) -> f64 {
        f64::from(self.next_value()) / f64::from(i32::MAX)
    }

    fn next_max(&mut self, maximum: i32) -> i32 {
        (self.next_double() * f64::from(maximum)) as i32
    }

    fn next_range(&mut self, minimum: i32, maximum: i32) -> i32 {
        let range = i64::from(maximum) - i64::from(minimum);
        (self.next_double() * range as f64) as i32 + minimum
    }

    fn sample(&mut self) -> i32 {
        const MBIG: i32 = i32::MAX;
        let mut inext = self.inext;
        let mut inextp = self.inextp;
        inext += 1;
        if inext >= 56 {
            inext = 1;
        }
        inextp += 1;
        if inextp >= 56 {
            inextp = 1;
        }
        let mut value =
            self.seed_array[inext as usize].wrapping_sub(self.seed_array[inextp as usize]);
        if value == MBIG {
            value -= 1;
        }
        if value < 0 {
            value = value.wrapping_add(MBIG);
        }
        self.seed_array[inext as usize] = value;
        self.inext = inext;
        self.inextp = inextp;
        value
    }
}

type BoundArguments = (Vec<Value>, Vec<(String, Value)>);

#[derive(Debug, Clone)]
struct FunctionDef {
    parameters: Vec<Parameter>,
    begin: Vec<Statement>,
    process: Vec<Statement>,
    end: Vec<Statement>,
    body: Vec<Statement>,
    filter: bool,
}

impl FunctionDef {
    fn has_blocks(&self) -> bool {
        !self.begin.is_empty() || !self.process.is_empty() || !self.end.is_empty()
    }
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
        let flow = self.exec_block(&script.statements, out)?;
        self.flush_host(out)?;
        match flow {
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

    fn restore_variables(&mut self, saved: HashMap<String, Value>) {
        self.variables = saved;
        for (key, value) in &self.globals {
            self.variables.insert(key.clone(), value.clone());
        }
    }

    fn exec_block(
        &mut self,
        statements: &[Statement],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        for statement in statements {
            match self.exec_statement(statement, out) {
                Ok(Flow::Normal) => {}
                Ok(Flow::Throw(value)) if self.trap.is_some() => {
                    self.variables.insert("_".to_owned(), throw_record(&value));
                    if self.run_trap(out)? {
                        continue;
                    }
                    return Ok(Flow::Normal);
                }
                Ok(flow) => return Ok(flow),
                Err(EngineError::Runtime(error))
                    if self.trap.is_some() && catchable(&error).is_some() =>
                {
                    let record = catchable(&error).unwrap_or(Value::Null);
                    self.variables.insert("_".to_owned(), record);
                    if self.run_trap(out)? {
                        continue;
                    }
                    return Ok(Flow::Normal);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Flow::Normal)
    }

    /// Runs the active `trap`; returns true when `continue` asks the caller to
    /// resume with the next statement.
    fn run_trap(&mut self, out: &mut dyn Write) -> Result<bool, EngineError> {
        let trap = self.trap.clone().unwrap_or_default();
        match self.exec_block(&trap, out)? {
            Flow::Continue(_) => Ok(true),
            Flow::Break(_) => Ok(false),
            Flow::Normal => Ok(false),
            _ => Ok(false),
        }
    }

    fn exec_statement(
        &mut self,
        statement: &Statement,
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        match statement {
            Statement::Expression(expr) => {
                let value = self.eval_expression(expr)?;
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
                    if is_global_scope(name) {
                        self.globals.insert(key.to_owned(), value.clone());
                    }
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
                if is_global_scope(name) {
                    self.globals.insert(key.to_owned(), updated.clone());
                }
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
                if is_global_scope(name) {
                    self.globals.insert(key.to_owned(), updated.clone());
                }
                self.variables.insert(key.to_owned(), updated);
                self.last_success = true;
                Ok(Flow::Normal)
            }
            Statement::Assign { name, value, .. } => {
                let value = evaluate(value, &mut self.variables, self.last_success)?;
                let global = is_global_scope(name);
                let key = strip_scope(name).to_owned();
                if global {
                    self.globals.insert(key.clone(), value.clone());
                }
                self.variables.insert(key, value);
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
                begin,
                process,
                end,
                body,
                filter,
                ..
            } => {
                self.functions.insert(
                    name.to_ascii_lowercase(),
                    FunctionDef {
                        parameters: parameters.clone(),
                        begin: begin.clone(),
                        process: process.clone(),
                        end: end.clone(),
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
            Statement::AssignPipeline { name, commands, .. } => {
                let outputs = self.pipeline_output(commands)?;
                let value = match outputs.len() {
                    0 => Value::Null,
                    1 => outputs.into_iter().next().unwrap_or(Value::Null),
                    _ => Value::array(outputs),
                };
                let key = strip_scope(name);
                self.variables.insert(key.to_owned(), value);
                Ok(Flow::Normal)
            }
            Statement::Switch {
                condition,
                wildcard,
                regex,
                case_sensitive,
                clauses,
                ..
            } => self.exec_switch(condition, *wildcard, *regex, *case_sensitive, clauses, out),
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
                let thrown = match self.exec_block(body, out) {
                    Ok(Flow::Throw(value)) => Some((value, true)),
                    Ok(flow) => {
                        let mut result = flow;
                        if let Some(finally) = finally {
                            let finally = self.exec_block(finally, out)?;
                            if !matches!(finally, Flow::Normal) {
                                result = finally;
                            }
                        }
                        return Ok(result);
                    }
                    Err(EngineError::Runtime(error)) => match catchable(&error) {
                        Some(record) => Some((record, false)),
                        None => return Err(EngineError::Runtime(error)),
                    },
                    Err(error) => return Err(error),
                };
                let mut flow = Flow::Throw(
                    thrown
                        .as_ref()
                        .map_or(Value::Null, |(value, _)| value.clone()),
                );
                if let Some((value, from_throw)) = thrown {
                    for clause in catches {
                        if clause.types.is_empty()
                            || clause.types.iter().any(|kind| catch_matches(kind))
                        {
                            let record = if from_throw {
                                throw_record(&value)
                            } else {
                                value
                            };
                            self.variables.insert("_".to_owned(), record);
                            flow = self.exec_block(&clause.body, out)?;
                            break;
                        }
                    }
                }
                if let Some(finally) = finally {
                    let finally = self.exec_block(finally, out)?;
                    if !matches!(finally, Flow::Normal) {
                        flow = finally;
                    }
                }
                Ok(flow)
            }
            Statement::Trap { body, .. } => {
                self.trap = Some(body.clone());
                Ok(Flow::Normal)
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
        if let Some(outputs) = self.run_builtin(name, arguments, &[])? {
            for value in outputs {
                self.write_value(value, out)?;
            }
            return Ok(Flow::Normal);
        }
        if !self.functions.contains_key(&name.to_ascii_lowercase()) {
            if let Some(outputs) = self.run_native(name, arguments, &[])? {
                for value in outputs {
                    self.write_value(value, out)?;
                }
                return Ok(Flow::Normal);
            }
            return Err(command_not_found(name).into());
        }
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
        let saved_trap = self.trap.take();
        self.variables = local;
        let flow = if function.has_blocks() {
            self.exec_named_blocks(&function, None, out)
        } else {
            self.exec_block(&function.body, out)
        };
        self.restore_variables(saved);
        self.trap = saved_trap;
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

    /// Runs a function's `begin` / `process` / `end` blocks over `input`.
    /// `None` invokes `process` once with `$_` as null.
    fn exec_named_blocks(
        &mut self,
        function: &FunctionDef,
        input: Option<Vec<Value>>,
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let mut flow = self.exec_block(&function.begin, out)?;
        if matches!(flow, Flow::Normal) {
            match input {
                Some(items) if !function.process.is_empty() => {
                    for item in items {
                        self.variables.insert("_".to_owned(), item);
                        flow = self.exec_block(&function.process, out)?;
                        if !matches!(flow, Flow::Normal) {
                            break;
                        }
                    }
                }
                Some(items) => {
                    self.variables
                        .insert("input".to_owned(), Value::array(items));
                }
                None => {
                    self.variables.insert("_".to_owned(), Value::Null);
                    if !function.process.is_empty() {
                        flow = self.exec_block(&function.process, out)?;
                    }
                }
            }
        }
        if matches!(flow, Flow::Normal | Flow::Return(_)) {
            let end = self.exec_block(&function.end, out)?;
            if !matches!(end, Flow::Normal) {
                flow = end;
            }
        }
        Ok(flow)
    }

    fn exec_switch(
        &mut self,
        condition: &Expr,
        wildcard: bool,
        regex: bool,
        case_sensitive: bool,
        clauses: &[SwitchClause],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let value = evaluate(condition, &mut self.variables, self.last_success)?;
        let items = flatten_value(value);
        let previous = self.variables.get("_").cloned();
        let mut result = Flow::Normal;
        'items: for item in items {
            self.variables.insert("_".to_owned(), item.clone());
            let mut matched = false;
            for clause in clauses.iter().filter(|clause| clause.label.is_some()) {
                let label = clause.label.as_ref().unwrap();
                let is_match = if wildcard {
                    let pattern =
                        evaluate(label, &mut self.variables, self.last_success)?.to_string();
                    ops::wildcard_match(&item.to_string(), &pattern, case_sensitive)
                } else if regex {
                    let pattern = evaluate(label, &mut self.variables, self.last_success)?;
                    matches!(
                        regex_match(&item, &pattern, case_sensitive, false, &mut self.variables)?,
                        Value::Bool(true)
                    )
                } else if matches!(label.kind, ExprKind::ScriptBlock(_)) {
                    self.scriptblock_truth(label)?
                } else {
                    let expected = evaluate(label, &mut self.variables, self.last_success)?;
                    matches!(
                        ops::compare(&item, &expected, ops::Comparison::Eq, case_sensitive)?,
                        Value::Bool(true)
                    )
                };
                if !is_match {
                    continue;
                }
                matched = true;
                match self.exec_block(&clause.body, out)? {
                    Flow::Normal => {}
                    Flow::Break(None) => {
                        result = Flow::Normal;
                        break 'items;
                    }
                    Flow::Continue(None) => continue 'items,
                    other => {
                        result = other;
                        break 'items;
                    }
                }
            }
            if matched {
                continue;
            }
            for clause in clauses.iter().filter(|clause| clause.label.is_none()) {
                match self.exec_block(&clause.body, out)? {
                    Flow::Normal => {}
                    Flow::Break(None) => {
                        result = Flow::Normal;
                        break 'items;
                    }
                    Flow::Continue(None) => continue 'items,
                    other => {
                        result = other;
                        break 'items;
                    }
                }
            }
        }
        match previous {
            Some(value) => self.variables.insert("_".to_owned(), value),
            None => self.variables.remove("_"),
        };
        Ok(result)
    }

    fn scriptblock_truth(&mut self, expr: &Expr) -> Result<bool, EngineError> {
        let ExprKind::ScriptBlock(statements) = &expr.kind else {
            return Ok(ops::is_true(&evaluate(
                expr,
                &mut self.variables,
                self.last_success,
            )?));
        };
        let saved = self.capture.take();
        self.capture = Some(Vec::new());
        let flow = self.exec_block(statements, &mut capture_sink());
        let captured = self.capture.take().unwrap_or_default();
        self.capture = saved;
        flow?;
        Ok(captured.first().is_some_and(ops::is_true))
    }

    fn exec_pipeline(
        &mut self,
        commands: &[PipelineCommand],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let outputs = self.pipeline_output(commands)?;
        for value in outputs {
            self.write_value(value, out)?;
        }
        Ok(Flow::Normal)
    }

    fn pipeline_output(&mut self, commands: &[PipelineCommand]) -> Result<Vec<Value>, EngineError> {
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
                return Ok(outputs);
            }
            input = outputs;
        }
        Ok(input)
    }

    /// Runs a command for a pipeline stage and collects its output.
    fn run_command(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        input: Vec<Value>,
    ) -> Result<Vec<Value>, EngineError> {
        if let Some(outputs) = self.run_builtin(name, arguments, &input)? {
            return Ok(outputs);
        }
        if !self.functions.contains_key(&name.to_ascii_lowercase()) {
            if let Some(outputs) = self.run_native(name, arguments, &input)? {
                return Ok(outputs);
            }
            return Err(command_not_found(name).into());
        }
        let function = self.function(name)?;
        let (positional, named) = self.evaluate_arguments(arguments)?;
        let saved_trap = self.trap.take();
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
                self.restore_variables(saved);
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
            let has_blocks = function.has_blocks();
            let saved = std::mem::take(&mut self.variables);
            let mut local = saved.clone();
            let bound = self.bind_parameters(&function, &positional, &named, &mut local)?;
            local.insert(
                "args".to_owned(),
                Value::array(positional[bound..].to_vec()),
            );
            if !has_blocks {
                local.insert("input".to_owned(), Value::array(input.clone()));
            }
            self.variables = local;
            self.capture = Some(Vec::new());
            let flow = if has_blocks {
                self.exec_named_blocks(&function, Some(input), &mut capture_sink())
            } else {
                self.exec_block(&function.body, &mut capture_sink())
            };
            let captured = self.capture.take().unwrap_or_default();
            self.restore_variables(saved);
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
        self.trap = saved_trap;
        Ok(outputs)
    }

    /// Commands implemented by the engine itself. `Ok(None)` means the name
    /// is not a builtin and should be resolved as a function.
    fn run_builtin(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        input: &[Value],
    ) -> Result<Option<Vec<Value>>, EngineError> {
        match name.to_ascii_lowercase().as_str() {
            "write-output" | "echo" | "write-warning" | "write-verbose" | "write-debug"
            | "write-information" | "write-error" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let no_enumerate = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("noenumerate") && ops::is_true(value)
                });
                let mut outputs = Vec::new();
                for value in positional {
                    if no_enumerate {
                        outputs.push(value);
                    } else {
                        outputs.extend(flatten_value(value));
                    }
                }
                Ok(Some(outputs))
            }
            "foreach-object" => {
                if let Some(script) = script_block_argument(arguments) {
                    let script = script.clone();
                    let mut outputs = Vec::new();
                    for item in input {
                        let previous = self.variables.insert("_".to_owned(), item.clone());
                        let saved = self.capture.take();
                        self.capture = Some(Vec::new());
                        let flow = self.exec_block(&script, &mut capture_sink());
                        let captured = self.capture.take().unwrap_or_default();
                        self.capture = saved;
                        match previous {
                            Some(value) => self.variables.insert("_".to_owned(), value),
                            None => self.variables.remove("_"),
                        };
                        outputs.extend(captured);
                        flow?;
                    }
                    return Ok(Some(outputs));
                }
                let member_name = arguments.iter().find_map(|argument| match argument {
                    CommandArgument::Bareword(word) => Some(word.clone()),
                    _ => None,
                });
                let Some(member_name) = member_name else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "ForEach-Object without a script block or member name",
                    }
                    .into());
                };
                let mut outputs = Vec::new();
                for item in input {
                    outputs.push(member(item, &member_name)?);
                }
                Ok(Some(outputs))
            }
            "where-object" | "where" => {
                let Some(script) = script_block_argument(arguments) else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Where-Object without a script block",
                    }
                    .into());
                };
                let script = script.clone();
                let mut outputs = Vec::new();
                for item in input {
                    let previous = self.variables.insert("_".to_owned(), item.clone());
                    let saved = self.capture.take();
                    self.capture = Some(Vec::new());
                    let flow = self.exec_block(&script, &mut capture_sink());
                    let captured = self.capture.take().unwrap_or_default();
                    self.capture = saved;
                    match previous {
                        Some(value) => self.variables.insert("_".to_owned(), value),
                        None => self.variables.remove("_"),
                    };
                    flow?;
                    if captured.first().is_some_and(ops::is_true) {
                        outputs.push(item.clone());
                    }
                }
                Ok(Some(outputs))
            }
            "measure-object" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let sum = named
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("sum"));
                let mut total = Value::Int32(0);
                if sum {
                    for item in input {
                        total = ops::add(&total, item)?;
                    }
                }
                Ok(Some(vec![Value::object(vec![
                    ("Count".to_owned(), Value::Int32(input.len() as i32)),
                    ("Sum".to_owned(), total),
                ])]))
            }
            "sort-object" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let descending = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("descending") && ops::is_true(value)
                });
                let case_sensitive = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("casesensitive") && ops::is_true(value)
                });
                let mut items = input.to_vec();
                items.sort_by(|left, right| {
                    let left = left.to_string();
                    let right = right.to_string();
                    culture_rank(&left)
                        .cmp(&culture_rank(&right))
                        .then_with(|| left.to_lowercase().cmp(&right.to_lowercase()))
                        .then_with(|| {
                            if case_sensitive {
                                culture_case(&left).cmp(&culture_case(&right))
                            } else {
                                std::cmp::Ordering::Equal
                            }
                        })
                });
                if descending {
                    items.reverse();
                }
                Ok(Some(items))
            }
            "group-object" => {
                let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
                for item in input {
                    let key = item.to_string();
                    if let Some((_, members)) = groups
                        .iter_mut()
                        .find(|(name, _)| name.eq_ignore_ascii_case(&key))
                    {
                        members.push(item.clone());
                    } else {
                        groups.push((key, vec![item.clone()]));
                    }
                }
                groups.sort_by_key(|(name, _)| name.to_lowercase());
                let outputs = groups
                    .into_iter()
                    .map(|(name, group)| {
                        Value::object(vec![
                            ("Name".to_owned(), Value::String(name.into())),
                            ("Count".to_owned(), Value::Int32(group.len() as i32)),
                            ("Group".to_owned(), Value::array(group)),
                        ])
                    })
                    .collect();
                Ok(Some(outputs))
            }
            "get-culture" | "get-uiculture" => Ok(Some(vec![culture_value()])),
            "get-item" => {
                let (positional, _) = self.evaluate_arguments(arguments)?;
                let Some(path) = positional.first() else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Get-Item without a path",
                    }
                    .into());
                };
                let text = path.to_string();
                if !std::path::Path::new(&text).exists() {
                    return Err(RuntimeError::Script {
                        id: "PathNotFound,Microsoft.PowerShell.Commands.GetItemCommand",
                        category: format!(
                            "ObjectNotFound: ({text}:String) [Get-Item], ItemNotFoundException"
                        ),
                        exception_type: "System.Management.Automation.ItemNotFoundException",
                        message: format!("Cannot find path '{text}' because it does not exist."),
                    }
                    .into());
                }
                Ok(Some(vec![Value::object(vec![
                    (
                        "__TypeName".to_owned(),
                        Value::String("System.IO.FileInfo".into()),
                    ),
                    ("FullName".to_owned(), Value::String(text.into())),
                ])]))
            }
            "get-content" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let Some(path) = positional.first() else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Get-Content without a path",
                    }
                    .into());
                };
                let text = std::fs::read_to_string(path.to_string()).map_err(|_| {
                    RuntimeError::NotImplemented {
                        feature: "Get-Content of a missing file",
                    }
                })?;
                let mut lines: Vec<Value> = text
                    .split('\n')
                    .map(|line| Value::String(line.trim_end_matches('\r').into()))
                    .collect();
                if text.ends_with('\n') {
                    lines.pop();
                }
                if let Some((_, count)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("totalcount"))
                {
                    let count = convert_int32(count)?.max(0) as usize;
                    lines.truncate(count);
                }
                Ok(Some(lines))
            }
            "test-path" => {
                let (positional, _) = self.evaluate_arguments(arguments)?;
                let Some(path) = positional.first() else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Test-Path without a path",
                    }
                    .into());
                };
                Ok(Some(vec![Value::Bool(
                    std::path::Path::new(&path.to_string()).exists(),
                )]))
            }
            "join-path" => {
                let (positional, _) = self.evaluate_arguments(arguments)?;
                let parts: Vec<String> = positional.iter().map(ToString::to_string).collect();
                if parts.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Join-Path without a path",
                    }
                    .into());
                }
                Ok(Some(vec![Value::String(parts.join("/").into())]))
            }
            "split-path" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let Some(path) = positional.first() else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Split-Path without a path",
                    }
                    .into());
                };
                let path = path.to_string();
                let path = std::path::Path::new(&path);
                if named
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("leaf") && ops::is_true(value))
                {
                    let leaf = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    Ok(Some(vec![Value::String(leaf.into())]))
                } else {
                    let parent = path
                        .parent()
                        .map(|parent| parent.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    Ok(Some(vec![Value::String(parent.into())]))
                }
            }
            "write-host" => {
                let (positional, _) = self.evaluate_arguments(arguments)?;
                let mut outputs = Vec::new();
                for value in positional {
                    outputs.extend(flatten_value(value));
                }
                Ok(Some(outputs))
            }
            "set-content" => {
                let (positional, _) = self.evaluate_arguments(arguments)?;
                let Some(path) = positional.first() else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Set-Content without a path",
                    }
                    .into());
                };
                let mut text = String::new();
                for item in input {
                    text.push_str(&item.to_string());
                    text.push('\n');
                }
                std::fs::write(path.to_string(), text).map_err(|_| {
                    RuntimeError::NotImplemented {
                        feature: "Set-Content of an unwritable file",
                    }
                })?;
                Ok(Some(Vec::new()))
            }
            "get-random" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                if let Some((_, seed)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("setseed"))
                {
                    self.random = Some(RandomGenerator::new(convert_int32(seed)?));
                }
                if self.random.is_none() {
                    let seed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |duration| duration.subsec_nanos() as i32);
                    self.random = Some(RandomGenerator::new(seed));
                }
                let generator = self.random.as_mut().unwrap();
                if let Some((_, maximum)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("maximum"))
                {
                    let maximum = convert_int32(maximum)?;
                    return Ok(Some(vec![Value::Int32(generator.next_max(maximum))]));
                }
                let input = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("inputobject"))
                    .map(|(_, value)| value.clone())
                    .or_else(|| positional.first().cloned());
                if let Some(input) = input {
                    let items = flatten_value(input);
                    let count = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("count"))
                        .map_or(Ok(1), |(_, count)| convert_int32(count))?
                        .max(1) as usize;
                    let count = count.min(items.len());
                    let mut chosen: Vec<Value> = items[..count].to_vec();
                    for (index, item) in items.iter().enumerate().skip(count) {
                        if generator.next_range(0, index as i32 + 1) < count as i32 {
                            let replace = generator.next_range(0, count as i32) as usize;
                            chosen[replace] = item.clone();
                        }
                    }
                    let mut output = Vec::new();
                    for i in 0..count {
                        let j = generator.next_range(i as i32, count as i32) as usize;
                        output.push(chosen[j].clone());
                        if i != j {
                            chosen[j] = chosen[i].clone();
                        }
                    }
                    return Ok(Some(output));
                }
                Ok(Some(vec![Value::Int32(generator.next_max(i32::MAX))]))
            }
            "out-string" => {
                let text = format_out_string(input);
                Ok(Some(vec![Value::String(text.into())]))
            }
            "convertto-json" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let compress = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("compress") && ops::is_true(value)
                });
                if !compress {
                    return Err(RuntimeError::NotImplemented {
                        feature: "ConvertTo-Json without -Compress",
                    }
                    .into());
                }
                let values = if input.is_empty() {
                    positional
                } else {
                    input.to_vec()
                };
                let mut outputs = Vec::new();
                for value in values {
                    outputs.push(Value::String(json_stringify(&value)?.into()));
                }
                Ok(Some(outputs))
            }
            "convertfrom-json" => {
                let mut outputs = Vec::new();
                for item in input {
                    outputs.push(json_parse(&item.to_string())?);
                }
                Ok(Some(outputs))
            }
            "select-object" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let mut items = input.to_vec();
                if named
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("unique") && ops::is_true(value))
                {
                    let mut seen: Vec<String> = Vec::new();
                    items.retain(|item| {
                        let key = item.to_string();
                        if seen.contains(&key) {
                            false
                        } else {
                            seen.push(key);
                            true
                        }
                    });
                }
                if let Some((_, skip)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("skip"))
                {
                    let skip = convert_int32(skip)?.max(0) as usize;
                    items = items.into_iter().skip(skip).collect();
                }
                if let Some((_, first)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("first"))
                {
                    let first = convert_int32(first)?.max(0) as usize;
                    items.truncate(first);
                }
                Ok(Some(items))
            }
            _ => Ok(None),
        }
    }

    /// Runs an external program. `Ok(None)` means the executable was not
    /// found, so the caller reports a command-not-found refusal.
    fn run_native(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        input: &[Value],
    ) -> Result<Option<Vec<Value>>, EngineError> {
        let arguments = self.native_arguments(arguments)?;
        let mut command = std::process::Command::new(name);
        command
            .args(&arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in &self.variables {
            if let Some(env_name) = key.strip_prefix("env:") {
                match value {
                    Value::Null | Value::AutomationNull => {
                        command.env_remove(env_name);
                    }
                    value => {
                        command.env(env_name, value.to_string());
                    }
                }
            }
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => {
                return Err(RuntimeError::NotImplemented {
                    feature: "starting a native command",
                }
                .into());
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            let mut text = String::new();
            for (index, item) in input.iter().enumerate() {
                if index > 0 {
                    text.push('\n');
                }
                text.push_str(&item.to_string());
            }
            if !input.is_empty() {
                text.push('\n');
            }
            let _ = stdin.write_all(text.as_bytes());
        }
        let output = child
            .wait_with_output()
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "waiting for a native command",
            })?;
        let code = output.status.code().unwrap_or(1);
        self.last_success = output.status.success();
        self.variables
            .insert("LASTEXITCODE".to_owned(), Value::Int32(code));
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = Vec::new();
        let mut rest = text.as_ref();
        if rest.ends_with('\n') {
            rest = &rest[..rest.len() - 1];
        }
        if !rest.is_empty() {
            for line in rest.split('\n') {
                lines.push(Value::String(line.trim_end_matches('\r').into()));
            }
        }
        Ok(Some(lines))
    }

    fn native_arguments(
        &mut self,
        arguments: &[CommandArgument],
    ) -> Result<Vec<String>, EngineError> {
        let mut out = Vec::new();
        for argument in arguments {
            match argument {
                CommandArgument::Bareword(word) => {
                    if word.contains(['*', '?']) {
                        let matches = expand_glob(word);
                        if matches.is_empty() {
                            out.push(word.clone());
                        } else {
                            out.extend(matches);
                        }
                    } else {
                        out.push(word.clone());
                    }
                }
                CommandArgument::Value(expr) => {
                    let value = evaluate(expr, &mut self.variables, self.last_success)?;
                    out.push(value.to_string());
                }
                CommandArgument::Parameter { name, value } => {
                    out.push(format!("-{name}"));
                    if let Some(expr) = value {
                        let value = evaluate(expr, &mut self.variables, self.last_success)?;
                        out.push(value.to_string());
                    }
                }
            }
        }
        Ok(out)
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

    fn eval_expression(&mut self, expr: &Expr) -> Result<Value, EngineError> {
        if let ExprKind::Group(statements) = &expr.kind {
            return self.eval_group(statements);
        }
        if let ExprKind::ExpandableString(parts) = &expr.kind {
            let mut text = String::new();
            for part in parts {
                match part {
                    rpsh_syntax::StringPart::Literal(value) => text.push_str(value),
                    rpsh_syntax::StringPart::Variable(name) => {
                        if let Some(value) = lookup(name, &self.variables, self.last_success) {
                            use std::fmt::Write as _;
                            let _ = write!(text, "{value}");
                        }
                    }
                    rpsh_syntax::StringPart::Subexpression(expr) => {
                        use std::fmt::Write as _;
                        let _ = write!(text, "{}", self.eval_expression(expr)?);
                    }
                }
            }
            return Ok(Value::String(text.into()));
        }
        if let ExprKind::Member { target, name } = &expr.kind
            && let ExprKind::Group(statements) = &target.kind
        {
            let target = self.eval_group(statements)?;
            return Ok(member(&target, name)?);
        }
        if let ExprKind::Index { target, index } = &expr.kind
            && let ExprKind::Group(statements) = &target.kind
        {
            let target = self.eval_group(statements)?;
            let index = evaluate(index, &mut self.variables, self.last_success)?;
            return Ok(index_value(&target, &index)?);
        }
        if let ExprKind::Binary {
            op: BinaryOp::Join,
            left,
            right,
        } = &expr.kind
        {
            let left = self.eval_expression(left)?;
            let right = evaluate(right, &mut self.variables, self.last_success)?;
            return Ok(ops::join(&left, &right)?);
        }
        Ok(evaluate(expr, &mut self.variables, self.last_success)?)
    }

    fn eval_group(&mut self, statements: &[Statement]) -> Result<Value, EngineError> {
        let saved = self.capture.take();
        self.capture = Some(Vec::new());
        let flow = self.exec_block(statements, &mut capture_sink());
        let mut outputs = self.capture.take().unwrap_or_default();
        self.capture = saved;
        flow?;
        Ok(match outputs.len() {
            0 => Value::Null,
            1 => outputs.pop().unwrap_or(Value::Null),
            _ => Value::array(outputs),
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
                    positional.push(self.eval_expression(expr)?);
                }
                CommandArgument::Bareword(word) => {
                    positional.push(Value::String(word.clone().into()));
                }
                CommandArgument::Parameter { name, value } => {
                    let value = match value {
                        Some(expr) => self.eval_expression(expr)?,
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

    fn flush_host(&mut self, out: &mut dyn Write) -> Result<(), EngineError> {
        if self.host_objects.is_empty() {
            return Ok(());
        }
        let text = format_host_stream(&self.host_objects);
        self.host_objects.clear();
        out.write_all(text.as_bytes())?;
        Ok(())
    }

    fn write_value(&mut self, value: Value, out: &mut dyn Write) -> Result<(), EngineError> {
        if let Some(capture) = &mut self.capture {
            match value {
                Value::Array(items) => capture.extend(items.iter().cloned()),
                Value::Null | Value::AutomationNull => {}
                value => capture.push(value),
            }
            return Ok(());
        }
        if matches!(value, Value::Object(_)) {
            self.host_objects.push(value);
            return Ok(());
        }
        if let Value::Array(items) = &value
            && items.iter().any(|item| matches!(item, Value::Object(_)))
        {
            self.host_objects.extend(items.iter().cloned());
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
        // An undefined variable reads as `$null` outside strict mode.
        ExprKind::Variable(name) => {
            Ok(lookup(name, variables, last_success).unwrap_or(Value::Null))
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
        ExprKind::Group(_) => Err(RuntimeError::NotImplemented {
            feature: "a parenthesized command in this expression",
        }),
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
            if !object.is_empty() && object.iter().all(|(key, _)| key.parse::<i64>().is_ok()) {
                object = order_hashtable(object);
            }
            Ok(Value::object(object))
        }
        ExprKind::Cast { type_name, value } => {
            let value = evaluate(value, variables, last_success)?;
            cast_to(type_name, value)
        }
        ExprKind::StaticMember { type_name, name } => static_member(type_name, name),
        ExprKind::InvokeStatic {
            type_name,
            name,
            arguments,
        } => {
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(evaluate(argument, variables, last_success)?);
            }
            invoke_static(type_name, name, &values)
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
                BinaryOp::Format => ops::format(&left, &right),
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

fn script_block_argument(arguments: &[CommandArgument]) -> Option<&Vec<Statement>> {
    arguments.iter().find_map(|argument| match argument {
        CommandArgument::Value(Expr {
            kind: ExprKind::ScriptBlock(statements),
            ..
        }) => Some(statements),
        _ => None,
    })
}

fn json_stringify(value: &Value) -> Result<String, RuntimeError> {
    match value {
        Value::Null | Value::AutomationNull => Ok("null".to_owned()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Char(unit) => {
            let ch = char::from_u32(u32::from(*unit)).unwrap_or(char::REPLACEMENT_CHARACTER);
            Ok(json_quote(&ch.to_string()))
        }
        Value::Int32(_)
        | Value::Int64(_)
        | Value::UInt32(_)
        | Value::UInt64(_)
        | Value::Int16(_)
        | Value::UInt16(_)
        | Value::Byte(_)
        | Value::SByte(_)
        | Value::Double(_) => Ok(value.to_string()),
        Value::String(text) => Ok(json_quote(&text.to_string_lossy())),
        Value::Array(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items.iter() {
                parts.push(json_stringify(item)?);
            }
            Ok(format!("[{}]", parts.join(",")))
        }
        Value::Object(entries) => {
            let mut parts = Vec::with_capacity(entries.len());
            for (name, value) in entries.iter() {
                if is_hidden(name) {
                    continue;
                }
                parts.push(format!("{}:{}", json_quote(name), json_stringify(value)?));
            }
            Ok(format!("{{{}}}", parts.join(",")))
        }
        _ => Err(RuntimeError::NotImplemented {
            feature: "ConvertTo-Json for this value",
        }),
    }
}

fn json_quote(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn json_parse(text: &str) -> Result<Value, RuntimeError> {
    let mut parser = JsonParser {
        chars: text.chars().collect(),
        at: 0,
    };
    let value = parser.value()?;
    parser.whitespace();
    if parser.at != parser.chars.len() {
        return Err(RuntimeError::NotImplemented {
            feature: "trailing JSON input",
        });
    }
    Ok(value)
}

struct JsonParser {
    chars: Vec<char>,
    at: usize,
}

impl JsonParser {
    fn whitespace(&mut self) {
        while self
            .chars
            .get(self.at)
            .is_some_and(|ch| ch.is_ascii_whitespace())
        {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn expect(&mut self, expected: char) -> Result<(), RuntimeError> {
        self.whitespace();
        if self.peek() == Some(expected) {
            self.at += 1;
            Ok(())
        } else {
            Err(RuntimeError::NotImplemented {
                feature: "invalid JSON",
            })
        }
    }

    fn value(&mut self) -> Result<Value, RuntimeError> {
        self.whitespace();
        match self.peek() {
            Some('"') => self.string(),
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('t') => self.literal("true", Value::Bool(true)),
            Some('f') => self.literal("false", Value::Bool(false)),
            Some('n') => self.literal("null", Value::Null),
            Some(ch) if ch == '-' || ch.is_ascii_digit() => self.number(),
            _ => Err(RuntimeError::NotImplemented {
                feature: "invalid JSON",
            }),
        }
    }

    fn literal(&mut self, text: &str, value: Value) -> Result<Value, RuntimeError> {
        for expected in text.chars() {
            if self.peek() != Some(expected) {
                return Err(RuntimeError::NotImplemented {
                    feature: "invalid JSON",
                });
            }
            self.at += 1;
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<Value, RuntimeError> {
        self.expect('"')?;
        let mut text = String::new();
        while let Some(ch) = self.peek() {
            self.at += 1;
            match ch {
                '"' => return Ok(Value::String(text.into())),
                '\\' => {
                    let Some(escaped) = self.peek() else {
                        break;
                    };
                    self.at += 1;
                    match escaped {
                        '"' => text.push('"'),
                        '\\' => text.push('\\'),
                        '/' => text.push('/'),
                        'b' => text.push('\u{8}'),
                        'f' => text.push('\u{c}'),
                        'n' => text.push('\n'),
                        'r' => text.push('\r'),
                        't' => text.push('\t'),
                        'u' => {
                            let hex: String = self.chars[self.at..self.at + 4].iter().collect();
                            self.at += 4;
                            if let Ok(code) = u32::from_str_radix(&hex, 16)
                                && let Some(ch) = char::from_u32(code)
                            {
                                text.push(ch);
                            }
                        }
                        _ => {
                            return Err(RuntimeError::NotImplemented {
                                feature: "invalid JSON escape",
                            });
                        }
                    }
                }
                ch => text.push(ch),
            }
        }
        Err(RuntimeError::NotImplemented {
            feature: "invalid JSON",
        })
    }

    fn number(&mut self) -> Result<Value, RuntimeError> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | 'e' | 'E'))
        {
            self.at += 1;
        }
        let text: String = self.chars[start..self.at].iter().collect();
        if let Ok(value) = text.parse::<i32>() {
            Ok(Value::Int32(value))
        } else if let Ok(value) = text.parse::<i64>() {
            Ok(Value::Int64(value))
        } else if let Ok(value) = text.parse::<f64>() {
            Ok(Value::Double(value))
        } else {
            Err(RuntimeError::NotImplemented {
                feature: "invalid JSON number",
            })
        }
    }

    fn array(&mut self) -> Result<Value, RuntimeError> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.whitespace();
        if self.peek() == Some(']') {
            self.at += 1;
            return Ok(Value::array(items));
        }
        loop {
            items.push(self.value()?);
            self.whitespace();
            match self.peek() {
                Some(',') => {
                    self.at += 1;
                }
                Some(']') => {
                    self.at += 1;
                    return Ok(Value::array(items));
                }
                _ => {
                    return Err(RuntimeError::NotImplemented {
                        feature: "invalid JSON",
                    });
                }
            }
        }
    }

    fn object(&mut self) -> Result<Value, RuntimeError> {
        self.expect('{')?;
        let mut entries = Vec::new();
        self.whitespace();
        if self.peek() == Some('}') {
            self.at += 1;
            return Ok(Value::object(entries));
        }
        loop {
            let Value::String(key) = self.string()? else {
                unreachable!()
            };
            self.expect(':')?;
            let value = self.value()?;
            entries.push((key.to_string_lossy(), value));
            self.whitespace();
            match self.peek() {
                Some(',') => {
                    self.at += 1;
                }
                Some('}') => {
                    self.at += 1;
                    return Ok(Value::object(entries));
                }
                _ => {
                    return Err(RuntimeError::NotImplemented {
                        feature: "invalid JSON",
                    });
                }
            }
        }
    }
}

/// Expands `dir/pattern*` against the working directory, sorted. The caller
/// keeps the literal pattern when nothing matches, as pwsh does.
fn expand_glob(pattern: &str) -> Vec<String> {
    let path = Path::new(pattern);
    let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let mut matches = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&directory) {
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if !ops::wildcard_match(&name, file, false) {
                continue;
            }
            let prefix = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty());
            match prefix {
                Some(prefix) => matches.push(format!("{}/{}", prefix.display(), name)),
                None => matches.push(name),
            }
        }
    }
    matches.sort();
    matches
}

/// A script error that `try/catch` or `trap` can catch, from the refusal
/// features that are real PowerShell exceptions rather than missing support.
fn error_record(
    id: &str,
    category: &str,
    exception_type: &str,
    message: &str,
    text: &str,
) -> Value {
    let exception = Value::object(vec![
        (
            "__TypeName".to_owned(),
            Value::String(exception_type.to_owned().into()),
        ),
        (
            "Message".to_owned(),
            Value::String(message.to_owned().into()),
        ),
    ]);
    Value::object(vec![
        ("__Text".to_owned(), Value::String(text.to_owned().into())),
        (
            "FullyQualifiedErrorId".to_owned(),
            Value::String(id.to_owned().into()),
        ),
        (
            "CategoryInfo".to_owned(),
            Value::String(category.to_owned().into()),
        ),
        ("Exception".to_owned(), exception),
    ])
}

fn command_not_found(name: &str) -> RuntimeError {
    RuntimeError::Script {
        id: "CommandNotFoundException",
        category: format!("ObjectNotFound: ({name}:String) [], CommandNotFoundException"),
        exception_type: "System.Management.Automation.CommandNotFoundException",
        message: format!(
            "The term '{name}' is not recognized as a name of a cmdlet, function, script file, or executable program.\nCheck the spelling of the name, or if a path was included, verify that the path is correct and try again."
        ),
    }
}

fn throw_record(value: &Value) -> Value {
    let text = value.to_string();
    error_record(
        &text,
        &format!("OperationStopped: ({text}:String) [], RuntimeException"),
        "System.Management.Automation.RuntimeException",
        &text,
        &text,
    )
}

fn catchable(error: &RuntimeError) -> Option<Value> {
    if let Some(script) = error.script() {
        return Some(error_record(
            script.id,
            script.category,
            script.exception_type,
            script.message,
            script.message,
        ));
    }
    let feature = match error {
        RuntimeError::NotImplemented { feature } => feature,
        RuntimeError::Script { .. } => return None,
    };
    let (id, message) = match *feature {
        "cannot convert a string to a double" => (
            "InvalidCastFromStringToDouble",
            "Cannot convert value to type \"System.Double\".",
        ),
        _ => return None,
    };
    Some(error_record(
        id,
        "NotSpecified: (:) [], RuntimeException",
        "System.Management.Automation.RuntimeException",
        message,
        message,
    ))
}

/// `.GetType()` reports a hidden type name when an object carries one.
fn get_type(value: &Value) -> Value {
    if let Value::Object(entries) = value
        && let Some((_, Value::String(name))) = entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("__TypeName"))
    {
        return Value::Type(name.clone());
    }
    Value::Type(value.type_name().to_owned().into())
}

fn is_numeric_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Int32(_)
            | Value::Int64(_)
            | Value::UInt32(_)
            | Value::UInt64(_)
            | Value::Int16(_)
            | Value::UInt16(_)
            | Value::Byte(_)
            | Value::SByte(_)
            | Value::Double(_)
            | Value::Decimal(_)
    )
}

fn visible_entries(entries: &[(String, Value)]) -> Vec<(&String, &Value)> {
    entries
        .iter()
        .filter(|(key, _)| !is_hidden(key))
        .map(|(key, value)| (key, value))
        .collect()
}

fn hidden_type(value: &Value) -> Option<String> {
    let Value::Object(entries) = value else {
        return None;
    };
    entries.iter().find_map(|(key, value)| {
        if key == "__TypeName" {
            match value {
                Value::String(text) => Some(text.to_string_lossy()),
                _ => None,
            }
        } else {
            None
        }
    })
}

/// The text `Out-String` returns for one pipeline, following the host's
/// default table and list views for the shapes the conformance cases use.
fn format_out_string(input: &[Value]) -> String {
    let Some(first) = input.first() else {
        return String::new();
    };
    let Value::Object(entries) = first else {
        let mut text = String::new();
        for value in input {
            text.push_str(&value.to_string());
            text.push('\n');
        }
        return text;
    };
    let hashtable = hidden_type(first)
        .is_some_and(|name| name.contains("Dictionary") || name.contains("Hashtable"));
    let entries = entries.as_ref();
    if hashtable {
        let visible = visible_entries(entries);
        let rows: Vec<(String, String)> = visible
            .iter()
            .map(|(key, value)| ((*key).clone(), value.to_string()))
            .collect();
        let value_width = rows
            .iter()
            .map(|(_, value)| value.chars().count())
            .chain(std::iter::once("Value".len()))
            .max()
            .unwrap_or(5);
        let mut text = String::from("\n");
        text.push_str(&format!("{:<30} {}", "Name", "Value"));
        text.push('\n');
        text.push_str(&format!("{:<30} {}", "----", "-".repeat(5)));
        text.push('\n');
        let _ = value_width;
        for (name, value) in &rows {
            text.push_str(format!("{:<30} {}", name, value).trim_end());
            text.push('\n');
        }
        text.push('\n');
        return text;
    }
    let visible = visible_entries(entries);
    if visible.len() > 4 {
        let width = visible
            .iter()
            .map(|(name, _)| name.chars().count())
            .max()
            .unwrap_or(0);
        let mut text = String::from("\n");
        for (name, value) in &visible {
            text.push_str(&format!("{:<width$} : {}", name, value, width = width));
            text.push('\n');
        }
        text.push('\n');
        return text;
    }
    let columns: Vec<String> = visible.iter().map(|(name, _)| (*name).clone()).collect();
    let rows: Vec<Vec<String>> = input
        .iter()
        .map(|item| {
            let Value::Object(entries) = item else {
                return Vec::new();
            };
            columns
                .iter()
                .map(|column| {
                    entries
                        .iter()
                        .find(|(key, _)| !is_hidden(key) && key.eq_ignore_ascii_case(column))
                        .map_or_else(String::new, |(_, value)| value.to_string())
                })
                .collect()
        })
        .collect();
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            rows.iter()
                .map(|row| row[index].chars().count())
                .chain(std::iter::once(column.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut text = String::from("\n");
    text.push_str(
        &columns
            .iter()
            .zip(&widths)
            .map(|(column, width)| format!("{column:<width$}"))
            .collect::<Vec<_>>()
            .join(" "),
    );
    text.push('\n');
    text.push_str(
        &columns
            .iter()
            .zip(&widths)
            .map(|(column, width)| format!("{:<width$}", "-".repeat(column.chars().count())))
            .collect::<Vec<_>>()
            .join(" "),
    );
    text.push('\n');
    for (row, _) in rows.iter().enumerate() {
        let cells: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let width = widths[index];
                let aligns_right = input
                    .get(row)
                    .and_then(|item| match item {
                        Value::Object(entries) => entries
                            .iter()
                            .find(|(key, _)| {
                                !is_hidden(key) && key.eq_ignore_ascii_case(&columns[index])
                            })
                            .map(|(_, value)| is_numeric_value(value)),
                        _ => None,
                    })
                    .unwrap_or(false);
                if aligns_right {
                    format!("{:>width$}", rows[row][index])
                } else {
                    format!("{:<width$}", rows[row][index])
                }
            })
            .collect();
        text.push_str(&cells.join(" "));
        text.push('\n');
    }
    text.push('\n');
    text
}

/// .NET's `Hashtable` walks integer keys by descending bucket. For small
/// tables it uses the next prime at least twice the entry count.
fn order_hashtable(mut entries: Vec<(String, Value)>) -> Vec<(String, Value)> {
    let capacity = next_prime((entries.len() * 2).max(3));
    entries.sort_by_key(|(key, _)| {
        let value = key.parse::<i64>().unwrap_or(0) as u64 & 0x7fff_ffff;
        std::cmp::Reverse(value % capacity as u64)
    });
    entries
}

fn weekday(year: i32, month: i32, day: i32) -> i32 {
    // Sakamoto's algorithm; 0 = Sunday.
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut year = year;
    if month < 3 {
        year -= 1;
    }
    (year + year / 4 - year / 100 + year / 400 + OFFSETS[(month as usize).clamp(1, 12) - 1] + day)
        % 7
}

fn next_prime(mut value: usize) -> usize {
    loop {
        if (2..value).all(|factor| !value.is_multiple_of(factor)) {
            return value;
        }
        value += 1;
    }
}

fn format_host_stream(objects: &[Value]) -> String {
    let Some(first) = objects.first() else {
        return String::new();
    };
    let columns: Vec<String> = match first {
        Value::Object(entries) => visible_entries(entries)
            .iter()
            .map(|(name, _)| (*name).clone())
            .collect(),
        _ => return String::new(),
    };
    let mut rows: Vec<Vec<(String, bool)>> = Vec::new();
    for object in objects {
        let Value::Object(entries) = object else {
            continue;
        };
        let hashtable = hidden_type(object)
            .is_some_and(|name| name.contains("Dictionary") || name.contains("Hashtable"));
        if hashtable {
            for (key, value) in visible_entries(entries) {
                rows.push(
                    columns
                        .iter()
                        .map(|column| {
                            if column.eq_ignore_ascii_case("Name") {
                                ((*key).clone(), false)
                            } else if column.eq_ignore_ascii_case("Value") {
                                (value.to_string(), is_numeric_value(value))
                            } else {
                                (String::new(), false)
                            }
                        })
                        .collect(),
                );
            }
        } else {
            rows.push(
                columns
                    .iter()
                    .map(|column| {
                        entries
                            .iter()
                            .find(|(key, _)| !is_hidden(key) && key.eq_ignore_ascii_case(column))
                            .map_or((String::new(), false), |(_, value)| {
                                (value.to_string(), is_numeric_value(value))
                            })
                    })
                    .collect(),
            );
        }
    }
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            rows.iter()
                .map(|row| row[index].0.chars().count())
                .chain(std::iter::once(column.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut text = String::from("\n");
    text.push_str(
        &columns
            .iter()
            .zip(&widths)
            .map(|(column, width)| format!("{column:<width$}"))
            .collect::<Vec<_>>()
            .join(" "),
    );
    text.push('\n');
    text.push_str(
        &columns
            .iter()
            .zip(&widths)
            .map(|(column, width)| format!("{:<width$}", "-".repeat(column.chars().count())))
            .collect::<Vec<_>>()
            .join(" "),
    );
    text.push('\n');
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(index, (value, numeric))| {
                if *numeric {
                    format!("{:>width$}", value, width = widths[index])
                } else {
                    format!("{:<width$}", value, width = widths[index])
                }
            })
            .collect();
        text.push_str(&cells.join(" "));
        text.push('\n');
    }
    text.push('\n');
    text
}

fn culture_rank(text: &str) -> u8 {
    match text.chars().next() {
        Some(ch) if !ch.is_alphanumeric() => 0,
        Some(ch) if ch.is_ascii_digit() => 1,
        Some(_) => 2,
        None => 0,
    }
}

fn culture_case(text: &str) -> u8 {
    match text.chars().next() {
        Some(ch) if ch.is_lowercase() => 0,
        Some(ch) if ch.is_uppercase() => 1,
        _ => 0,
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

fn is_global_scope(name: &str) -> bool {
    name.starts_with("global:") || name.starts_with("script:")
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
        "PSItem" => return variables.get("_").cloned().or(Some(Value::Null)),
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
    if let Some(value) = variables.get(key) {
        return Some(value.clone());
    }
    automatic_variable(key)
}

fn encoding_value() -> Value {
    Value::object(vec![("WebName".to_owned(), Value::String("utf-8".into()))])
}

fn typed_scalar(type_name: &str, text: &str) -> Value {
    Value::object(vec![
        (
            "__TypeName".to_owned(),
            Value::String(type_name.to_owned().into()),
        ),
        ("__Text".to_owned(), Value::String(text.to_owned().into())),
    ])
}

fn ps_version_table() -> Value {
    let mut entries = vec![
        (
            "PSVersion".to_owned(),
            typed_scalar("System.Management.Automation.SemanticVersion", "7.7.0"),
        ),
        ("PSEdition".to_owned(), Value::String("Core".into())),
        ("GitCommitId".to_owned(), Value::String("7.7.0".into())),
        ("OS".to_owned(), Value::String("Linux 0.0.0".into())),
        ("Platform".to_owned(), Value::String("Unix".into())),
        (
            "PSCompatibleVersions".to_owned(),
            typed_scalar("System.Version[]", "1.0 2.0 3.0 4.0 5.0 5.1 6.0 7.0"),
        ),
        (
            "PSRemotingProtocolVersion".to_owned(),
            typed_scalar("System.Version", "2.4"),
        ),
        (
            "SerializationVersion".to_owned(),
            typed_scalar("System.Version", "1.1.0.1"),
        ),
        (
            "WSManStackVersion".to_owned(),
            typed_scalar("System.Version", "3.0"),
        ),
    ];
    entries.push((
        "__TypeName".to_owned(),
        Value::String("System.Management.Automation.PSVersionHashTable".into()),
    ));
    Value::object(entries)
}

fn automatic_variable(name: &str) -> Option<Value> {
    match name {
        "PSStyle" => Some(Value::object(vec![(
            "OutputRendering".to_owned(),
            Value::String("PlainText".into()),
        )])),
        "ErrorView" => Some(Value::String("ConciseView".into())),
        "OutputEncoding" => Some(encoding_value()),
        "Host" => Some(Value::object(vec![
            ("Name".to_owned(), Value::String("ConsoleHost".into())),
            ("Version".to_owned(), Value::String("7.7.0".into())),
            (
                "UI".to_owned(),
                Value::object(vec![(
                    "RawUI".to_owned(),
                    Value::object(vec![(
                        "WindowSize".to_owned(),
                        Value::object(vec![
                            ("Width".to_owned(), Value::Int32(80)),
                            ("Height".to_owned(), Value::Int32(40)),
                        ]),
                    )]),
                )]),
            ),
        ])),
        "PSVersionTable" => Some(ps_version_table()),
        "IsLinux" => Some(Value::Bool(true)),
        "IsWindows" | "IsMacOS" => Some(Value::Bool(false)),
        "IsCoreCLR" => Some(Value::Bool(true)),
        "PSEdition" => Some(Value::String("Core".into())),
        "ErrorActionPreference"
        | "VerbosePreference"
        | "DebugPreference"
        | "WarningPreference"
        | "InformationPreference"
        | "ProgressPreference" => Some(Value::String("SilentlyContinue".into())),
        "PID" => Some(Value::Int32(std::process::id() as i32)),
        "HOME" => std::env::var("HOME")
            .ok()
            .map(|value| Value::String(value.into())),
        _ => None,
    }
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

fn normalize_type(type_name: &str) -> String {
    let lower = type_name.to_ascii_lowercase();
    lower
        .strip_prefix("system.")
        .map_or(lower.clone(), ToOwned::to_owned)
}

fn culture_value() -> Value {
    Value::object(vec![
        ("Name".to_owned(), Value::String("en-US".into())),
        (
            "DateTimeFormat".to_owned(),
            Value::object(vec![(
                "LongTimePattern".to_owned(),
                Value::String("h:mm:ss\u{202F}tt".into()),
            )]),
        ),
    ])
}

fn static_member(type_name: &str, name: &str) -> Result<Value, RuntimeError> {
    let kind = normalize_type(type_name);
    let name = name.to_ascii_lowercase();
    let integer = match &kind[..] {
        "int32" | "int" => Some(("int32", i64::from(i32::MAX), i64::from(i32::MIN))),
        "int64" | "long" => Some(("int64", i64::MAX, i64::MIN)),
        "uint32" | "uint" => Some(("uint32", i64::from(u32::MAX), 0)),
        "int16" | "short" => Some(("int16", i64::from(i16::MAX), i64::from(i16::MIN))),
        "uint16" | "ushort" => Some(("uint16", i64::from(u16::MAX), 0)),
        "byte" => Some(("byte", i64::from(u8::MAX), 0)),
        "sbyte" => Some(("sbyte", i64::from(i8::MAX), i64::from(i8::MIN))),
        _ => None,
    };
    if let Some((kind, max, min)) = integer {
        return match name.as_str() {
            "maxvalue" => match kind {
                "int32" => Ok(Value::Int32(i32::MAX)),
                "int64" => Ok(Value::Int64(i64::MAX)),
                "uint32" => Ok(Value::UInt32(u32::MAX)),
                "int16" => Ok(Value::Int16(i16::MAX)),
                "uint16" => Ok(Value::UInt16(u16::MAX)),
                "byte" => Ok(Value::Byte(u8::MAX)),
                "sbyte" => Ok(Value::SByte(i8::MAX)),
                _ => Ok(Value::Int64(max)),
            },
            "minvalue" => match kind {
                "int32" => Ok(Value::Int32(i32::MIN)),
                "int64" => Ok(Value::Int64(i64::MIN)),
                "uint32" => Ok(Value::UInt32(0)),
                "int16" => Ok(Value::Int16(i16::MIN)),
                "uint16" => Ok(Value::UInt16(0)),
                "byte" => Ok(Value::Byte(0)),
                "sbyte" => Ok(Value::SByte(i8::MIN)),
                _ => Ok(Value::Int64(min)),
            },
            _ => Err(RuntimeError::NotImplemented {
                feature: "this static member",
            }),
        };
    }
    match (kind.as_str(), name.as_str()) {
        ("cultureinfo", "currentculture") | ("cultureinfo", "currentuiculture") => {
            Ok(culture_value())
        }
        ("datetimekind", "utc") => Ok(Value::Int32(1)),
        ("datetimekind", "local") => Ok(Value::Int32(2)),
        ("datetimekind", "unspecified") => Ok(Value::Int32(0)),
        ("console", "isoutputredirected") => Ok(Value::Bool(true)),
        ("console", "windowwidth") => Ok(Value::Int32(0)),
        ("console", "outputencoding") => Ok(encoding_value()),
        ("double", "maxvalue") => Ok(Value::Double(f64::MAX)),
        ("double", "minvalue") => Ok(Value::Double(f64::MIN)),
        ("double", "epsilon") => Ok(Value::Double(f64::EPSILON)),
        ("string", "empty") => Ok(Value::String("".into())),
        ("math", "pi") => Ok(Value::Double(std::f64::consts::PI)),
        ("math", "e") => Ok(Value::Double(std::f64::consts::E)),
        _ => Err(RuntimeError::NotImplemented {
            feature: "this static member",
        }),
    }
}

fn invoke_static(type_name: &str, name: &str, arguments: &[Value]) -> Result<Value, RuntimeError> {
    let kind = normalize_type(type_name);
    let name = name.to_ascii_lowercase();
    match (kind.as_str(), name.as_str()) {
        ("datetime", "new") => {
            let number = |index: usize| -> Result<i32, RuntimeError> {
                arguments.get(index).map_or(Ok(0), convert_int32)
            };
            let year = number(0)?;
            let month = number(1)?;
            let day = number(2)?;
            let hour = number(3)?;
            let minute = number(4)?;
            let second = number(5)?;
            let text = format!("{month:02}/{day:02}/{year:04} {hour:02}:{minute:02}:{second:02}");
            Ok(Value::object(vec![
                (
                    "__TypeName".to_owned(),
                    Value::String("System.DateTime".into()),
                ),
                ("__Year".to_owned(), Value::Int32(year)),
                ("__Month".to_owned(), Value::Int32(month)),
                ("__Day".to_owned(), Value::Int32(day)),
                ("__Hour".to_owned(), Value::Int32(hour)),
                ("__Minute".to_owned(), Value::Int32(minute)),
                ("__Second".to_owned(), Value::Int32(second)),
                ("__Text".to_owned(), Value::String(text.into())),
            ]))
        }
        ("io.file", "readalltext") => {
            let Some(value) = arguments.first() else {
                return Err(RuntimeError::NotImplemented {
                    feature: "ReadAllText with no path",
                });
            };
            let path = value.to_string();
            match std::fs::read_to_string(&path) {
                Ok(text) => Ok(Value::String(text.into())),
                Err(_) => Err(RuntimeError::Script {
                    id: "DirectoryNotFoundException",
                    category: "NotSpecified: (:) [], MethodInvocationException".to_owned(),
                    exception_type: "System.Management.Automation.MethodInvocationException",
                    message: format!(
                        "Exception calling \"ReadAllText\" with \"1\" argument(s): \"Could not find a part of the path '{path}'.\""
                    ),
                }),
            }
        }
        (kind, "new") if kind.starts_with("collections.generic.hashset") => {
            let values = arguments
                .first()
                .map_or_else(Vec::new, |value| flatten_value(value.clone()));
            let mut unique: Vec<Value> = Vec::new();
            for value in values {
                if !unique.iter().any(|existing| existing == &value) {
                    unique.push(value);
                }
            }
            Ok(Value::array(unique))
        }
        (kind, "new") if kind.starts_with("collections.generic.dictionary") => {
            Ok(Value::object(Vec::new()))
        }
        ("random", "new") => {
            let seed = match arguments.first() {
                Some(value) if !value.is_null_like() => convert_int32(value)?,
                _ => 0,
            };
            let mut generator = RandomGenerator::new(seed);
            let value = generator.sample();
            Ok(Value::object(vec![
                (
                    "__TypeName".to_owned(),
                    Value::String("System.Random".into()),
                ),
                ("__Next".to_owned(), Value::Int32(value)),
            ]))
        }
        ("math", "abs") => match arguments.first() {
            Some(Value::Int32(value)) => Ok(Value::Int32(value.abs())),
            Some(Value::Int64(value)) => Ok(Value::Int64(value.abs())),
            Some(value) => Ok(Value::Double(convert_double(value)?.abs())),
            None => Err(RuntimeError::NotImplemented {
                feature: "math with no argument",
            }),
        },
        ("math", "max") | ("math", "min") => {
            let (Some(left), Some(right)) = (arguments.first(), arguments.get(1)) else {
                return Err(RuntimeError::NotImplemented {
                    feature: "math with fewer than two arguments",
                });
            };
            let want_max = name == "max";
            if let (Value::Int32(a), Value::Int32(b)) = (left, right) {
                return Ok(Value::Int32(if want_max { *a.max(b) } else { *a.min(b) }));
            }
            let a = convert_double(left)?;
            let b = convert_double(right)?;
            Ok(Value::Double(if want_max { a.max(b) } else { a.min(b) }))
        }
        ("math", "sqrt") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.sqrt(),
        )),
        ("math", "pow") => {
            let a = convert_double(arguments.first().unwrap_or(&Value::Null))?;
            let b = convert_double(arguments.get(1).unwrap_or(&Value::Null))?;
            Ok(Value::Double(a.powf(b)))
        }
        ("int32" | "int", "parse") | ("int64" | "long", "parse") => {
            let Some(value) = arguments.first() else {
                return Err(RuntimeError::NotImplemented {
                    feature: "Parse with no argument",
                });
            };
            let text = value.to_string();
            let Some(parsed) = parse_integer_text(text.trim()) else {
                return Err(RuntimeError::Script {
                    id: "FormatException",
                    category: "NotSpecified: (:) [], MethodInvocationException".to_owned(),
                    exception_type: "System.Management.Automation.MethodInvocationException",
                    message: format!(
                        "Exception calling \"Parse\" with \"1\" argument(s): \"The input string '{text}' was not in a correct format.\""
                    ),
                });
            };
            if kind.starts_with("int32") || kind == "int" {
                i32::try_from(parsed)
                    .map(Value::Int32)
                    .map_err(|_| RuntimeError::NotImplemented {
                        feature: "integer conversion overflow",
                    })
            } else {
                i64::try_from(parsed)
                    .map(Value::Int64)
                    .map_err(|_| RuntimeError::NotImplemented {
                        feature: "integer conversion overflow",
                    })
            }
        }
        ("string", "isnullorempty") => {
            let empty = arguments
                .first()
                .is_none_or(|value| matches!(value, Value::Null | Value::AutomationNull))
                || arguments
                    .first()
                    .is_some_and(|value| value.to_string().is_empty());
            Ok(Value::Bool(empty))
        }
        ("string", "join") => {
            let Some(separator) = arguments.first() else {
                return Err(RuntimeError::NotImplemented {
                    feature: "Join with no separator",
                });
            };
            let separator = separator.to_string();
            let values: Vec<Value> = if arguments.len() == 2 {
                flatten_value(arguments[1].clone())
            } else {
                arguments[1..].to_vec()
            };
            Ok(Value::String(
                values
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(&separator)
                    .into(),
            ))
        }
        _ => Err(RuntimeError::NotImplemented {
            feature: "this static method",
        }),
    }
}

fn mark_hashtable(value: Value, type_name: &str) -> Value {
    if let Value::Object(entries) = value {
        let mut entries = entries.as_ref().clone();
        entries.push((
            "__TypeName".to_owned(),
            Value::String(type_name.to_owned().into()),
        ));
        Value::object(entries)
    } else {
        value
    }
}

fn is_hidden(key: &str) -> bool {
    key.starts_with("__")
}

/// `[type]value`, the subset of `LanguagePrimitives.ConvertTo` the skeleton
/// needs. Failing conversions are refused rather than approximated.
fn cast_to(type_name: &str, value: Value) -> Result<Value, RuntimeError> {
    if type_name.ends_with("[]") {
        return Ok(match value {
            Value::Array(_) => value,
            other => Value::array(vec![other]),
        });
    }
    match type_name.to_ascii_lowercase().as_str() {
        "pscustomobject" | "system.management.automation.pscustomobject" => Ok(value),
        "ordered" | "ordereddictionary" | "system.collections.specialized.ordereddictionary" => Ok(
            mark_hashtable(value, "System.Collections.Specialized.OrderedDictionary"),
        ),
        "hashtable" | "system.collections.hashtable" => {
            Ok(mark_hashtable(value, "System.Collections.Hashtable"))
        }
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
        Value::String(text) => {
            let lossy = text.to_string_lossy();
            match parse_integer_text(&lossy) {
                Some(value) => i64::try_from(value).map_err(|_| RuntimeError::NotImplemented {
                    feature: "integer conversion overflow",
                }),
                None => Err(RuntimeError::Script {
                    id: "InvalidCastFromStringToInteger",
                    category: "InvalidArgument: (:) [], RuntimeException".to_owned(),
                    exception_type: "System.Management.Automation.RuntimeException",
                    message: format!(
                        "Cannot convert value \"{lossy}\" to type \"System.Int32\". Error: \"The input string '{lossy}' was not in a correct format.\""
                    ),
                }),
            }
        }
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
        .filter(|(name, _)| !is_hidden(name))
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
        return Ok(get_type(target));
    }
    if name.eq_ignore_ascii_case("ToString")
        && !matches!(target, Value::String(_) | Value::Object(_))
    {
        return match arguments.first() {
            Some(spec) => {
                let spec = spec.to_string();
                let template = Value::String(format!("{{0:{spec}}}").into());
                Ok(ops::format(&template, target)?)
            }
            None => Ok(Value::String(target.to_string().into())),
        };
    }
    if target.is_null_like() {
        return Err(RuntimeError::Script {
            id: "InvokeMethodOnNull",
            category: "InvalidOperation: (:) [], RuntimeException".to_owned(),
            exception_type: "System.Management.Automation.RuntimeException",
            message: "You cannot call a method on a null-valued expression.".to_owned(),
        });
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
            let out_of_range = || RuntimeError::Script {
                id: "ArgumentOutOfRangeException",
                category: "NotSpecified: (:) [], MethodInvocationException".to_owned(),
                exception_type: "System.Management.Automation.MethodInvocationException",
                message: if arguments.len() > 1 {
                    "Exception calling \"Substring\" with \"2\" argument(s): \"Length cannot be less than zero. (Parameter 'length')\"".to_owned()
                } else {
                    "Exception calling \"Substring\" with \"1\" argument(s): \"startIndex cannot be larger than length of string. (Parameter 'startIndex')\"".to_owned()
                },
            };
            let start = int_argument(arguments.first())?;
            let start = usize::try_from(start).map_err(|_| out_of_range())?;
            let length = match arguments.get(1) {
                Some(value) => {
                    usize::try_from(int_argument(Some(value))?).map_err(|_| out_of_range())?
                }
                None => units.len().saturating_sub(start),
            };
            let end = start
                .checked_add(length)
                .filter(|end| *end <= units.len())
                .ok_or_else(out_of_range)?;
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
        "tostring" if entries.iter().any(|(key, _)| key == "__Year") => {
            let field = |name: &str| -> i32 {
                entries
                    .iter()
                    .find(|(key, _)| key == name)
                    .and_then(|(_, value)| convert_int32(value).ok())
                    .unwrap_or(0)
            };
            let year = field("__Year");
            let month = field("__Month");
            let day = field("__Day");
            let hour = field("__Hour");
            let minute = field("__Minute");
            let second = field("__Second");
            let format = arguments.first().map(ToString::to_string);
            let text = match format.as_deref() {
                None => {
                    let (hour12, ampm) = if hour == 0 {
                        (12, "AM")
                    } else if hour < 12 {
                        (hour, "AM")
                    } else if hour == 12 {
                        (12, "PM")
                    } else {
                        (hour - 12, "PM")
                    };
                    format!("{month}/{day}/{year} {hour12}:{minute:02}:{second:02}\u{202F}{ampm}")
                }
                Some(format) if format.eq_ignore_ascii_case("D") => {
                    const WEEKDAYS: [&str; 7] = [
                        "Sunday",
                        "Monday",
                        "Tuesday",
                        "Wednesday",
                        "Thursday",
                        "Friday",
                        "Saturday",
                    ];
                    const MONTHS: [&str; 12] = [
                        "January",
                        "February",
                        "March",
                        "April",
                        "May",
                        "June",
                        "July",
                        "August",
                        "September",
                        "October",
                        "November",
                        "December",
                    ];
                    let weekday = weekday(year, month, day) as usize;
                    format!(
                        "{}, {} {}, {}",
                        WEEKDAYS[weekday % 7],
                        MONTHS[(month as usize).clamp(1, 12) - 1],
                        day,
                        year
                    )
                }
                Some(_) => {
                    return Err(RuntimeError::NotImplemented {
                        feature: "this DateTime format",
                    });
                }
            };
            Ok(Value::String(text.into()))
        }
        "next" => {
            let raw = entries
                .iter()
                .find(|(key, _)| key == "__Next")
                .map(|(_, value)| value.clone())
                .ok_or(RuntimeError::NotImplemented {
                    feature: "Next on this object",
                })?;
            match arguments.first() {
                Some(maximum) => {
                    let maximum = convert_int32(maximum)?;
                    let raw = convert_int64(&raw)?;
                    Ok(Value::Int32(
                        (raw as f64 / f64::from(i32::MAX) * f64::from(maximum)) as i32,
                    ))
                }
                None => Ok(raw),
            }
        }
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
        return Ok(get_type(value));
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
            if let Some((_, value)) = entries
                .iter()
                .find(|(key, _)| !is_hidden(key) && key.eq_ignore_ascii_case(name))
            {
                return Ok(value.clone());
            }
            let visible: Vec<&(String, Value)> =
                entries.iter().filter(|(key, _)| !is_hidden(key)).collect();
            if name.eq_ignore_ascii_case("Count") {
                return Ok(Value::Int32(visible.len() as i32));
            }
            if name.eq_ignore_ascii_case("Keys") {
                return Ok(Value::array(
                    visible
                        .iter()
                        .map(|(key, _)| Value::String(key.clone().into()))
                        .collect(),
                ));
            }
            if name.eq_ignore_ascii_case("Values") {
                return Ok(Value::array(
                    visible.iter().map(|(_, value)| value.clone()).collect(),
                ));
            }
            Ok(Value::Null)
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
        let (result, out) = run("1\n2\nclass Foo { }");
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
