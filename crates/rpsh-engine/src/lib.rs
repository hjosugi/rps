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

mod aliases;
mod array_list;
mod encoding;
mod enum_table;
mod environment;
mod errors;
mod globalization;
mod method_error;
mod native;
mod preferences;
mod redirection;
mod sorting;
mod string_compare;
mod string_search;
mod variables;
mod wildcards;

use aliases::Aliases;
use encoding::Encoding;
pub use errors::ScriptException;
use errors::error_record;
use method_error::{ambiguous_overload, overload_error};
use native::NativeOutput;
use redirection::{OutputTarget, record_prefix};
use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use variables::Variables;

use rpsh_commands::{BuiltinModule, MODULES};
use rpsh_format::{CommandTableItem, command_table};
use rpsh_runtime::{PsString, RuntimeError, Value, ops};
use rpsh_syntax::{
    AssignmentValue, BinaryOp, ChainOperator, CommandArgument, Expr, ExprKind, MethodDef,
    NumberLiteral, Parameter, ParseError, PipelineChain, PipelineCommand, ScriptBlockAst,
    Statement, SwitchClause, UnaryOp, parse,
};

/// Runs scripts.
#[derive(Debug, Default)]
pub struct Engine {
    variables: Variables,
    functions: HashMap<String, FunctionDef>,
    last_failed: bool,
    /// The console host can hand its actual descriptors to native children.
    /// Embedded engines continue to use the caller's Write implementation.
    console_stdio: bool,
    non_interactive: bool,
    command_paths: native::LookupPaths,
    /// While running a pipeline stage, output is collected here instead of
    /// being written to the host.
    capture: Option<Vec<Value>>,
    /// The active `trap` block of the function being run.
    trap: Option<Vec<Statement>>,
    /// The active catch or trap for a bare rethrow.
    current_exception: Option<ScriptException>,
    /// Objects written to the host, formatted as one table at the end.
    host_objects: Vec<Value>,
    // ConsoleHost keeps its text writer once the first host value is rendered.
    // Native stdout bypasses that writer and does not select its encoding.
    host_encoding: Option<Encoding>,
    /// The generator Get-Random draws from; -SetSeed replaces it.
    random: Option<RandomGenerator>,
    /// `$global:`/`$script:` variables, kept across function calls.
    globals: HashMap<String, Value>,
    /// User-defined PowerShell classes.
    classes: HashMap<String, ClassDef>,
    /// Session aliases shared by command discovery, dispatch, and the provider.
    aliases: Aliases,
    modules: Vec<LoadedModule>,
    /// Functions introduced by each imported module.
    module_functions: HashMap<String, Vec<String>>,
    /// Locations saved by Push-Location.
    locations: Vec<std::path::PathBuf>,
    /// True when a typed host value was written, so the format stream gets a
    /// final blank line.
    host_text: bool,
}

#[derive(Debug)]
enum LoadedModule {
    Script(String),
    Builtin(&'static BuiltinModule),
}

impl LoadedModule {
    fn name(&self) -> &str {
        match self {
            Self::Script(name) => name,
            Self::Builtin(module) => module.name,
        }
    }

    fn value(&self) -> Value {
        let mut entries = vec![("Name".to_owned(), Value::String(self.name().into()))];
        let kind = match self {
            Self::Script(_) => "Script",
            Self::Builtin(module) => {
                entries.push((
                    "Version".to_owned(),
                    version_value(module.version.to_owned()),
                ));
                "Manifest"
            }
        };
        entries.push(("ModuleType".to_owned(), Value::String(kind.into())));
        Value::object(entries)
    }
}

#[derive(Debug, Clone)]
struct ClassDef {
    name: String,
    base: Option<String>,
    properties: Vec<(String, String)>,
    constructors: Vec<MethodDef>,
    methods: Vec<MethodDef>,
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
    name: String,
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
    /// A PowerShell error that terminated the enclosing block.
    Script(ScriptException),
    /// Writing to the output stream failed.
    Io(io::Error),
}

impl EngineError {
    /// The label of the missing feature, when that is why the script failed.
    pub fn missing_feature(&self) -> Option<&'static str> {
        match self {
            Self::Parse(error) => Some(error.feature()),
            Self::Runtime(error) => Some(error.feature()),
            Self::Io(_) | Self::Script(_) => None,
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::Runtime(error) => error.fmt(f),
            Self::Script(error) => error.fmt(f),
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

    /// Use the process's standard streams for native commands that inherit them.
    pub fn for_console() -> Self {
        Self::for_console_with_mode(false)
    }

    /// Initialize a real console host, optionally disallowing prompts.
    pub fn for_console_with_mode(non_interactive: bool) -> Self {
        Self {
            console_stdio: true,
            non_interactive,
            ..Self::default()
        }
    }

    /// Parses `source` in full, then runs it, writing what reaches the end of
    /// the pipeline to `out`. Returns the process exit code: the argument of
    /// `exit`, or 0 when the script runs to its end.
    pub fn run_script(&mut self, source: &str, out: &mut dyn Write) -> Result<i32, EngineError> {
        self.run_script_with(source, &[], None, out)
    }

    /// ConsoleHost's -Command mode returns 1 when its final pipeline failed.
    pub fn run_command_text(
        &mut self,
        source: &str,
        out: &mut dyn Write,
    ) -> Result<i32, EngineError> {
        let script = parse(source)?;
        self.variables
            .insert("args".to_owned(), Value::array(Vec::new()));
        self.run_ast_with_status(&script, out, true)
    }

    /// Runs a script file or command with its arguments and path, so `param`,
    /// `$args`, `$PSScriptRoot` and `$PSCommandPath` work.
    pub fn run_script_with(
        &mut self,
        source: &str,
        args: &[String],
        script_path: Option<&std::path::Path>,
        out: &mut dyn Write,
    ) -> Result<i32, EngineError> {
        let script = parse(source)?;
        self.variables.insert(
            "args".to_owned(),
            Value::array(
                args.iter()
                    .map(|arg| Value::String(arg.clone().into()))
                    .collect(),
            ),
        );
        if let Some(path) = script_path {
            let full = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            self.variables.insert(
                "PSCommandPath".to_owned(),
                Value::String(full.to_string_lossy().to_string().into()),
            );
            if let Some(parent) = full.parent() {
                self.variables.insert(
                    "PSScriptRoot".to_owned(),
                    Value::String(parent.to_string_lossy().to_string().into()),
                );
            }
        }
        self.run_ast(&script, out)
    }

    /// Runs an already parsed script. For a caller that keeps the AST, this
    /// separates parsing from execution, as upstream's `ScriptBlock` does.
    pub fn run_ast(
        &mut self,
        script: &ScriptBlockAst,
        out: &mut dyn Write,
    ) -> Result<i32, EngineError> {
        self.run_ast_with_status(script, out, false)
    }

    fn run_ast_with_status(
        &mut self,
        script: &ScriptBlockAst,
        out: &mut dyn Write,
        command_mode: bool,
    ) -> Result<i32, EngineError> {
        let flow = self.exec_block(&script.statements, out);
        self.flush_host(out)?;
        if self.host_text {
            self.write_host(out, format_args!("\n"))?;
            self.host_text = false;
        }
        let implicit_exit = i32::from(command_mode && self.last_failed);
        match flow? {
            Flow::Normal => Ok(implicit_exit),
            Flow::Exit(code) => Ok(code),
            Flow::Return(value) => {
                if let Some(value) = value {
                    self.write_value(value, out)?;
                }
                Ok(implicit_exit)
            }
            // `break` and `continue` outside a loop end the script quietly, as
            // upstream's top-level script block does.
            Flow::Break(_) | Flow::Continue(_) => Ok(0),
        }
    }

    fn class_chain(&self, name: &str) -> Vec<ClassDef> {
        let mut chain = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut current = Some(name.to_ascii_lowercase());
        while let Some(key) = current {
            if !seen.insert(key.clone()) {
                break;
            }
            let Some(class) = self.classes.get(&key) else {
                break;
            };
            current = class.base.as_ref().map(|base| base.to_ascii_lowercase());
            chain.push(class.clone());
        }
        chain
    }

    fn construct_class(
        &mut self,
        type_name: &str,
        arguments: &[Value],
    ) -> Result<Value, EngineError> {
        let classes = self.class_chain(type_name);
        let Some(class) = classes.first().cloned() else {
            return Err(RuntimeError::NotImplemented {
                feature: "construction of this class",
            }
            .into());
        };
        let mut entries: Vec<(String, Value)> = Vec::new();
        for class in classes.iter().rev() {
            for (kind, name) in &class.properties {
                let value = match normalize_type(kind).as_str() {
                    "int32" | "int" | "int64" | "long" | "double" | "uint32" | "uint64"
                    | "int16" | "uint16" | "byte" | "sbyte" => Value::Int32(0),
                    "string" => Value::String("".into()),
                    "bool" | "boolean" | "switch" => Value::Bool(false),
                    _ => Value::Null,
                };
                entries.push((name.clone(), value));
            }
        }
        entries.push((
            "__Class".to_owned(),
            Value::String(class.name.clone().into()),
        ));
        entries.push((
            "__TypeName".to_owned(),
            Value::String(class.name.clone().into()),
        ));
        entries.push((
            "__BaseClasses".to_owned(),
            Value::array(
                classes
                    .iter()
                    .skip(1)
                    .map(|class| Value::String(class.name.clone().into()))
                    .collect(),
            ),
        ));
        let mut object = Value::object(entries);
        if class.constructors.is_empty() && !arguments.is_empty() {
            return Err(RuntimeError::NotImplemented {
                feature: "this class constructor",
            }
            .into());
        }
        for ancestor in classes.iter().skip(1).rev() {
            if let Some(constructor) = ancestor
                .constructors
                .iter()
                .find(|constructor| constructor.parameters.is_empty())
                .cloned()
            {
                let saved = std::mem::take(&mut self.variables);
                let mut local = saved.child();
                local.insert("this".to_owned(), object.clone());
                self.variables = local;
                let flow = self.exec_block(&constructor.body, &mut capture_sink());
                if let Value::Object(entries) =
                    self.variables.get("this").cloned().unwrap_or(Value::Null)
                {
                    object = Value::Object(entries);
                }
                self.restore_variables(saved);
                flow?;
            }
        }
        if let Some(constructor) = class
            .constructors
            .iter()
            .find(|constructor| constructor.parameters.len() == arguments.len())
            .cloned()
        {
            let saved = std::mem::take(&mut self.variables);
            let mut local = saved.child();
            for (parameter, value) in constructor.parameters.iter().zip(arguments) {
                let value = match &parameter.type_name {
                    Some(type_name) => cast_to(type_name, value.clone())?,
                    None => value.clone(),
                };
                local.insert(parameter.name.clone(), value);
            }
            local.insert("this".to_owned(), object.clone());
            self.variables = local;
            let flow = self.exec_block(&constructor.body, &mut capture_sink());
            if let Value::Object(entries) =
                self.variables.get("this").cloned().unwrap_or(Value::Null)
            {
                object = Value::Object(entries);
            }
            self.restore_variables(saved);
            flow?;
        }
        Ok(object)
    }

    fn call_method(
        &mut self,
        class_name: &str,
        method: &str,
        target: &Value,
        arguments: &[Value],
    ) -> Result<Value, EngineError> {
        let classes = self.class_chain(class_name);
        if classes.is_empty() {
            return Err(RuntimeError::NotImplemented {
                feature: "this class method",
            }
            .into());
        }
        let Some(definition) = classes
            .iter()
            .flat_map(|class| class.methods.iter())
            .find(|definition| definition.name.eq_ignore_ascii_case(method))
            .cloned()
        else {
            return Ok(Value::Null);
        };
        let saved = std::mem::take(&mut self.variables);
        let mut local = saved.child();
        for (parameter, value) in definition.parameters.iter().zip(arguments) {
            let value = match &parameter.type_name {
                Some(type_name) => cast_to(type_name, value.clone())?,
                None => value.clone(),
            };
            local.insert(parameter.name.clone(), value);
        }
        local.insert("this".to_owned(), target.clone());
        self.variables = local;
        let saved_capture = self.capture.take();
        self.capture = Some(Vec::new());
        let flow = self.exec_block(&definition.body, &mut capture_sink());
        let mut outputs = self.capture.take().unwrap_or_default();
        self.capture = saved_capture;
        self.restore_variables(saved);
        match flow? {
            Flow::Return(Some(value)) => Ok(value),
            Flow::Return(None) => Ok(Value::Null),
            _ => Ok(match outputs.len() {
                0 => Value::Null,
                1 => outputs.pop().unwrap_or(Value::Null),
                _ => Value::array(outputs),
            }),
        }
    }

    fn restore_variables(&mut self, mut saved: Variables) {
        saved.restore_session(&mut self.variables);
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
            let result = self
                .exec_statement(statement, out)
                .map_err(|err| self.record_exception(err, statement.span()));
            match result {
                Ok(Flow::Normal) => {}
                Err(EngineError::Script(error)) if self.trap.is_some() => {
                    let trap = self.trap.take().expect("active trap");
                    let flow = self.handle_exception(&error, &trap, out);
                    self.trap = Some(trap);
                    if matches!(flow?, Flow::Continue(_)) {
                        continue;
                    }
                    return Ok(Flow::Normal);
                }
                result => return result,
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
                let value = self.eval_pipeline_expression(expr)?;
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
                Ok(Flow::Normal)
            }
            Statement::CoalesceAssign { name, value, .. } => {
                if lookup(name, &self.variables, !self.last_failed).is_none_or(|v| v.is_null_like())
                {
                    let value = self.eval_expression(value)?;
                    self.assign_variable(name, value)?;
                }
                self.last_failed = false;
                Ok(Flow::Normal)
            }
            Statement::AppendAssign { name, value, .. } => {
                let value = self.eval_expression(value)?;
                let old = lookup(name, &self.variables, !self.last_failed).unwrap_or(Value::Null);
                let updated = ops::add(&old, &value)?;
                self.assign_variable(name, updated)?;
                self.last_failed = false;
                Ok(Flow::Normal)
            }
            Statement::CompoundAssign {
                name, op, value, ..
            } => {
                let value = self.eval_expression(value)?;
                let old = lookup(name, &self.variables, !self.last_failed).unwrap_or(Value::Null);
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
                self.assign_variable(name, updated)?;
                self.last_failed = false;
                Ok(Flow::Normal)
            }
            Statement::Assign { name, value, .. } => {
                let value = self.eval_pipeline_expression(value)?;
                self.assign_variable(name, value)?;
                Ok(Flow::Normal)
            }
            Statement::MemberAssign {
                target,
                name,
                value,
                ..
            } => {
                let value = self.eval_expression(value)?;
                self.assign_property(
                    target,
                    name,
                    value,
                    "member assignment on a value that is not an object",
                )?;
                self.last_failed = false;
                Ok(Flow::Normal)
            }
            Statement::StaticMemberAssign {
                type_name,
                name,
                value,
                ..
            } => {
                if normalize_type(type_name) != "console"
                    || !name.eq_ignore_ascii_case("OutputEncoding")
                {
                    return Err(RuntimeError::NotImplemented {
                        feature: "setting this static property",
                    }
                    .into());
                }
                let value = self.eval_expression(value)?;
                self.flush_host(out)?;
                encoding::set_console(&value)?;
                self.last_failed = false;
                Ok(Flow::Normal)
            }
            Statement::IndexAssign {
                target,
                index,
                value,
                ..
            } => {
                let value = self.eval_expression(value)?;
                let index = self.eval_expression(index)?;
                let key = assignment_key(target)?;
                if let Some(Value::ArrayList(list)) = self.variables.get(key) {
                    array_list::set_index(list, &index, value)?;
                } else {
                    self.assign_property(
                        target,
                        &index.to_string(),
                        value,
                        "index assignment on a value that is not an object",
                    )?;
                }
                self.last_failed = false;
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
                        name: name.clone(),
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
            Statement::PipelineChain { chain, .. } => self.exec_chain(chain, out),
            Statement::AssignChain { name, chain, .. } => {
                let (flow, output) = self.capture_chain(chain)?;
                if matches!(flow, Flow::Normal) {
                    self.assign_variable(name, output)?;
                }
                Ok(flow)
            }
            Statement::AssignPipeline { name, commands, .. } => {
                let outputs = self.pipeline_output(commands)?;
                self.assign_variable(name, scalar_or_array(outputs.into_iter()))?;
                Ok(Flow::Normal)
            }
            Statement::MultiAssign { targets, value, .. } => {
                let result = match value {
                    AssignmentValue::Expression(expr) => self.eval_pipeline_expression(expr)?,
                    AssignmentValue::Chain(chain) => {
                        let (flow, output) = self.capture_chain(chain)?;
                        if !matches!(flow, Flow::Normal) {
                            return Ok(flow);
                        }
                        output
                    }
                    AssignmentValue::Pipeline(commands) => {
                        scalar_or_array(self.pipeline_output(commands)?.into_iter())
                    }
                };
                self.assign_multiple(targets, result)?;
                Ok(Flow::Normal)
            }
            Statement::Switch {
                condition,
                file,
                wildcard,
                regex,
                case_sensitive,
                clauses,
                ..
            } => self.exec_switch(
                condition,
                *file,
                *wildcard,
                *regex,
                *case_sensitive,
                clauses,
                out,
            ),
            Statement::If {
                condition,
                then,
                elseif,
                otherwise,
                ..
            } => {
                let value = self.eval_expression(condition)?;
                if ops::is_true(&value) {
                    self.last_failed = false;
                    return self.exec_block(then, out);
                }
                for (condition, body) in elseif {
                    let value = self.eval_expression(condition)?;
                    if ops::is_true(&value) {
                        self.last_failed = false;
                        return self.exec_block(body, out);
                    }
                }
                self.last_failed = false;
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
                let value = self.eval_expression(collection)?;
                let items = flatten_value(value);
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
                    let value = self.eval_expression(condition)?;
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
                let result = match self.exec_block(body, out) {
                    Err(EngineError::Script(error)) => {
                        match catches.iter().find(|clause| {
                            clause.types.is_empty()
                                || clause.types.iter().any(|kind| error.matches(kind))
                        }) {
                            Some(clause) => self.handle_exception(&error, &clause.body, out),
                            None => Err(EngineError::Script(error)),
                        }
                    }
                    other => other,
                };
                if let Some(finally) = finally {
                    let flow = self.exec_block(finally, out)?;
                    if !matches!(flow, Flow::Normal) {
                        return Ok(flow);
                    }
                }
                result
            }
            Statement::Class {
                name,
                base,
                properties,
                constructors,
                methods,
                ..
            } => {
                self.classes.insert(
                    name.to_ascii_lowercase(),
                    ClassDef {
                        name: name.clone(),
                        base: base.clone(),
                        properties: properties.clone(),
                        constructors: constructors.clone(),
                        methods: methods.clone(),
                    },
                );
                Ok(Flow::Normal)
            }
            Statement::Enum { name, members, .. } => {
                let mut next = 0i64;
                for (member, value) in members {
                    let value = value.unwrap_or(next);
                    next = value + 1;
                    let typed = Value::object(vec![
                        ("__TypeName".to_owned(), Value::String(name.clone().into())),
                        ("__Text".to_owned(), Value::String(member.clone().into())),
                        ("__EnumValue".to_owned(), Value::Int64(value)),
                    ]);
                    self.variables
                        .insert(format!("__enum::{name}::{member}"), typed);
                }
                Ok(Flow::Normal)
            }
            Statement::DotSource { path, .. } => {
                let source = std::fs::read_to_string(path).map_err(|_| {
                    RuntimeError::Script {
                        id: "CommandNotFoundException",
                        category: format!("ObjectNotFound: ({path}:String) [], CommandNotFoundException"),
                        exception_type: "System.Management.Automation.CommandNotFoundException",
                        message: format!("The term '{path}' is not recognized as a name of a cmdlet, function, script file, or executable program."),
                    }
                })?;
                let script = parse(&source)?;
                self.exec_block(&script.statements, out)
            }
            Statement::Param { parameters, .. } => {
                let positional = match self.variables.get("args") {
                    Some(Value::Array(items)) => items.as_ref().clone(),
                    _ => Vec::new(),
                };
                let mut bound = 0;
                for parameter in parameters {
                    let value = if let Some(value) = positional.get(bound) {
                        bound += 1;
                        value.clone()
                    } else if let Some(default) = &parameter.default {
                        evaluate(default, &mut self.variables, !self.last_failed)?
                    } else {
                        Value::Null
                    };
                    let value = match &parameter.type_name {
                        Some(type_name) => cast_to(type_name, value)?,
                        None => value,
                    };
                    self.variables.insert(parameter.name.clone(), value);
                }
                self.variables.insert(
                    "args".to_owned(),
                    Value::array(positional[bound..].to_vec()),
                );
                Ok(Flow::Normal)
            }
            Statement::Trap { body, .. } => {
                self.trap = Some(body.clone());
                Ok(Flow::Normal)
            }
            Statement::Throw { value, .. } => {
                let error = match value {
                    None if let Some(current) = &self.current_exception => current.rethrow(),
                    _ => {
                        let value = value
                            .as_ref()
                            .map(|expr| self.eval_expression(expr))
                            .transpose()?
                            .unwrap_or(Value::Null);
                        errors::thrown(value)
                    }
                };
                Err(EngineError::Script(error))
            }
            Statement::Return { value, .. } => {
                let value = match value {
                    Some(value) => Some(self.eval_expression(value)?),
                    None => None,
                };
                Ok(Flow::Return(value))
            }
            Statement::Break { label, .. } => Ok(Flow::Break(label.clone())),
            Statement::Continue { label, .. } => Ok(Flow::Continue(label.clone())),
            Statement::Exit { code: None, .. } => Ok(Flow::Exit(0)),
            Statement::Exit {
                code: Some(code), ..
            } => match self.eval_expression(code)? {
                Value::Int32(code) => Ok(Flow::Exit(code)),
                _ => Err(RuntimeError::NotImplemented {
                    feature: "exit with a value other than Int32",
                }
                .into()),
            },
        }
    }

    fn builtin_loaded(&self, module: &BuiltinModule) -> bool {
        self.modules.iter().any(
            |entry| matches!(entry, LoadedModule::Builtin(loaded) if loaded.name == module.name),
        )
    }

    fn assign_variable(&mut self, name: &str, value: Value) -> Result<(), EngineError> {
        if let Some(name) = environment::drive_name(name) {
            environment::set(&mut self.variables, name, &value);
            return Ok(());
        }
        let key = strip_scope(name);
        if key.eq_ignore_ascii_case("Error") {
            if !self.variables.nested || is_global_scope(name) {
                return Err(errors::constant_error(false).into());
            }
            self.variables.insert("Error".into(), value);
            return Ok(());
        }
        let (key, value) = if let Some(canonical) = preferences::variable_name(key) {
            (canonical, preferences::assignment(canonical, &value)?)
        } else {
            (key, value)
        };
        let key = if let Some(canonical) = encoding::variable_name(key) {
            if !value.is_null_like() && Encoding::from_value(&value).is_none() {
                return Err(RuntimeError::Script {
                    id: "RuntimeException",
                    category: "MetadataError: (:) [], ArgumentTransformationMetadataException".into(),
                    exception_type: "System.Management.Automation.ArgumentTransformationMetadataException",
                    message: format!("Cannot convert the \"{value}\" value of type \"{}\" to type \"System.Text.Encoding\".", get_type(&value)),
                }.into());
            }
            canonical
        } else { key }.to_owned();
        if is_global_scope(name) {
            self.globals.insert(key.clone(), value.clone());
        }
        self.variables.insert(key, value);
        Ok(())
    }

    fn assign_property(
        &mut self,
        target: &Expr,
        name: &str,
        value: Value,
        unsupported: &'static str,
    ) -> Result<(), EngineError> {
        let key = assignment_key(target)?;
        if let Some(Value::ArrayList(list)) = self.variables.get(key)
            && name.eq_ignore_ascii_case("Capacity")
        {
            return Ok(array_list::set_capacity(list, &value)?);
        }
        let Some(Value::Object(entries)) = self.variables.get(key) else {
            return Err(RuntimeError::NotImplemented {
                feature: unsupported,
            }
            .into());
        };
        if let Some(table) = environment::table_entries(entries) {
            let mut table = table.to_vec();
            if let Some((_, slot)) = table.iter_mut().find(|(entry, _)| entry == name) {
                *slot = value;
            } else {
                table.push((name.to_owned(), value));
            }
            self.variables
                .insert(key.to_owned(), environment::table(table));
            return Ok(());
        }
        let mut entries = entries.as_ref().clone();
        if let Some(entry) = entries
            .iter_mut()
            .find(|(entry, _)| entry.eq_ignore_ascii_case(name))
        {
            entry.1 = value;
        } else {
            entries.push((name.to_owned(), value));
        }
        self.variables
            .insert(key.to_owned(), Value::object(entries));
        Ok(())
    }

    fn assign_multiple(&mut self, targets: &[Expr], value: Value) -> Result<(), EngineError> {
        let Some((last, rest)) = targets.split_last() else {
            return Ok(());
        };
        let mut values = flatten_value(value).into_iter();
        for target in rest {
            self.assign_element(target, values.next().unwrap_or(Value::Null))?;
        }
        self.assign_element(last, scalar_or_array(values))
    }

    fn assign_element(&mut self, target: &Expr, value: Value) -> Result<(), EngineError> {
        match &target.kind {
            ExprKind::Variable(name) => {
                self.assign_variable(name, value)?;
                Ok(())
            }
            ExprKind::Paren(inner) => self.assign_element(inner, value),
            ExprKind::Array(targets) => self.assign_multiple(targets, value),
            ExprKind::Member { target, name } => self.assign_property(
                target,
                name,
                value,
                "member assignment on a value that is not an object",
            ),
            ExprKind::Index { target, index } => {
                let index = self.eval_expression(index)?;
                self.assign_property(
                    target,
                    &index.to_string(),
                    value,
                    "index assignment on a value that is not an object",
                )
            }
            _ => Err(RuntimeError::NotImplemented {
                feature: "this multiple-assignment target",
            }
            .into()),
        }
    }

    fn import_builtin(&mut self, module: &'static BuiltinModule) {
        if !self.builtin_loaded(module) {
            self.modules.push(LoadedModule::Builtin(module));
        }
        self.aliases.import(module);
    }

    fn autoload_unqualified(&self) -> bool {
        self.variables
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("PSModuleAutoLoadingPreference"))
            .is_none_or(|(_, value)| {
                value.is_null_like()
                    || matches!(value.to_string().to_ascii_lowercase().as_str(), "all" | "2")
            })
    }

    fn discover_alias(&mut self, name: &str) -> Option<String> {
        if let Some(alias) = self.aliases.get(name) {
            return Some(alias.definition.to_owned());
        }
        if let Some(module) = MODULES.iter().find(|module| module.exports_alias(name))
            && !self.functions.contains_key(&name.to_ascii_lowercase())
            && !self.builtin_loaded(module)
            && self.autoload_unqualified()
        {
            self.import_builtin(module);
            return self.aliases.definition(name);
        }
        None
    }

    fn call_function(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let resolved = self.discover_alias(name);
        let name = resolved.as_deref().unwrap_or(name);
        if !self.functions.contains_key(&name.to_ascii_lowercase()) {
            if let Some(outputs) = self.invoke_builtin(name, arguments, &[])? {
                for value in outputs {
                    self.write_value(value, out)?;
                }
                return Ok(Flow::Normal);
            }
            let destination = if self.capture.is_some() {
                NativeOutput::Objects
            } else {
                self.flush_host(out)?;
                NativeOutput::Host(out)
            };
            for value in self.run_native(name, arguments, None, destination)? {
                self.write_value(value, out)?;
            }
            return Ok(Flow::Normal);
        }
        let function = self.function(name)?;
        let (positional, named) = self.evaluate_arguments(arguments)?;
        let mut local = self.variables.child();
        let bound = self.bind_parameters(&function, &positional, &named, &mut local)?;
        local.insert(
            "args".to_owned(),
            Value::array(positional[bound..].to_vec()),
        );
        if function.filter {
            local.entry("_".to_owned()).or_insert(Value::Null);
        }
        let saved_trap = self.trap.take();
        let saved = std::mem::replace(&mut self.variables, local);
        let flow = if function.has_blocks() {
            self.exec_named_blocks(&function, None, out)
        } else {
            self.exec_block(&function.body, out)
        };
        self.restore_variables(saved);
        self.trap = saved_trap;
        if matches!(flow, Ok(Flow::Normal | Flow::Return(_))) {
            self.last_failed = false;
        }
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

    #[allow(clippy::too_many_arguments)]
    fn exec_switch(
        &mut self,
        condition: &Expr,
        file: bool,
        wildcard: bool,
        regex: bool,
        case_sensitive: bool,
        clauses: &[SwitchClause],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let value = self.eval_expression(condition)?;
        let items = if file {
            let path = value.to_string();
            let text =
                std::fs::read_to_string(&path).map_err(|_| RuntimeError::NotImplemented {
                    feature: "switch -File of a missing file",
                })?;
            text.lines()
                .map(|line| Value::String(line.into()))
                .collect()
        } else {
            flatten_value(value)
        };
        let previous = self.variables.get("_").cloned();
        let mut result = Flow::Normal;
        'items: for item in items {
            self.variables.insert("_".to_owned(), item.clone());
            let mut matched = false;
            for clause in clauses.iter().filter(|clause| clause.label.is_some()) {
                let label = clause.label.as_ref().unwrap();
                let is_match = if wildcard {
                    let value = evaluate(label, &mut self.variables, !self.last_failed)?;
                    let pattern = match &value {
                        Value::Object(entries)
                            if let Some((pattern, _)) = rpsh_runtime::wildcard::stored(entries) =>
                        {
                            pattern.clone()
                        }
                        other => other.to_ps_string(),
                    };
                    rpsh_runtime::wildcard::operator_match(
                        &item.to_ps_string(),
                        &pattern,
                        case_sensitive,
                    )?
                } else if regex {
                    let pattern = evaluate(label, &mut self.variables, !self.last_failed)?;
                    matches!(
                        regex_match(&item, &pattern, case_sensitive, false, &mut self.variables)?,
                        Value::Bool(true)
                    )
                } else if matches!(label.kind, ExprKind::ScriptBlock { .. }) {
                    self.scriptblock_truth(label)?
                } else {
                    let expected = evaluate(label, &mut self.variables, !self.last_failed)?;
                    matches!(
                        compare_values(&item, &expected, ops::Comparison::Eq, case_sensitive)?,
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
        let ExprKind::ScriptBlock { statements, .. } = &expr.kind else {
            return Ok(ops::is_true(&self.eval_expression(expr)?));
        };
        let saved = self.capture.take();
        self.capture = Some(Vec::new());
        let flow = self.exec_block(statements, &mut capture_sink());
        let captured = self.capture.take().unwrap_or_default();
        self.capture = saved;
        flow?;
        Ok(ops::is_true(&scalar_or_array(captured.into_iter())))
    }

    fn exec_pipeline(
        &mut self,
        commands: &[PipelineCommand],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let host = self
            .capture
            .is_none()
            .then_some(&mut *out as &mut dyn Write);
        let outputs = self.collect_pipeline(commands, host)?;
        for value in outputs {
            self.write_value(value, out)?;
        }
        Ok(Flow::Normal)
    }

    fn exec_chain(
        &mut self,
        chain: &PipelineChain,
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let mut flow = self.exec_chain_operand(&chain.first, out)?;
        for (operator, commands) in &chain.rest {
            if !matches!(flow, Flow::Normal) {
                break;
            }
            let run = match operator {
                ChainOperator::AndAnd => !self.last_failed,
                ChainOperator::OrOr => self.last_failed,
            };
            if run {
                flow = self.exec_chain_operand(commands, out)?;
            }
        }
        Ok(flow)
    }

    fn exec_chain_operand(
        &mut self,
        commands: &[PipelineCommand],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        match commands {
            [PipelineCommand::Command { name, arguments }] => {
                self.call_function(name, arguments, out)
            }
            _ => self.exec_pipeline(commands, out),
        }
    }

    fn capture_chain(&mut self, chain: &PipelineChain) -> Result<(Flow, Value), EngineError> {
        let saved = self.capture.replace(Vec::new());
        let flow = self.exec_chain(chain, &mut capture_sink());
        let outputs = std::mem::replace(&mut self.capture, saved).unwrap_or_default();
        Ok((flow?, scalar_or_array(outputs.into_iter())))
    }

    fn pipeline_output(&mut self, commands: &[PipelineCommand]) -> Result<Vec<Value>, EngineError> {
        self.collect_pipeline(commands, None)
    }

    fn collect_pipeline(
        &mut self,
        mut commands: &[PipelineCommand],
        mut host: Option<&mut dyn Write>,
    ) -> Result<Vec<Value>, EngineError> {
        let mut input = Vec::new();
        let mut failed = false;
        let mut has_upstream = false;
        while let Some((command, remaining)) = commands.split_first() {
            commands = remaining;
            if let Some(first) = self.native_stage(command) {
                let mut stages = vec![first];
                while stages.last().is_some_and(|stage| stage.file.is_none()) {
                    let Some((next, remaining)) = commands.split_first() else {
                        break;
                    };
                    let Some(next) = self.native_stage(next) else {
                        break;
                    };
                    stages.push(next);
                    commands = remaining;
                }
                let file = stages.last().and_then(|stage| stage.file);
                let target = match file {
                    Some((path, append)) => {
                        Some(OutputTarget::open(self.eval_expression(path)?, append)?)
                    }
                    None => None,
                };
                let destination = if let Some(target) = &target {
                    NativeOutput::Redirected(target)
                } else if commands.is_empty()
                    && let Some(out) = host.take()
                {
                    self.flush_host(out)?;
                    NativeOutput::Host(out)
                } else {
                    NativeOutput::Objects
                };
                input =
                    self.run_native_stages(&stages, has_upstream.then_some(&input), destination)?;
            } else {
                input = match command {
                    PipelineCommand::Expression(expr) => {
                        flatten_value(self.eval_pipeline_expression(expr)?)
                    }
                    PipelineCommand::Command { name, arguments } => {
                        self.run_command(name, arguments, input, None)?
                    }
                    PipelineCommand::Redirected {
                        command,
                        redirections,
                    } => self.run_redirected(command, input, redirections)?,
                };
            }
            has_upstream = true;
            failed |= self.last_failed;
        }
        self.last_failed = failed;
        Ok(input)
    }

    /// Runs a command for a pipeline stage and collects its output.
    fn run_command(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        input: Vec<Value>,
        output: Option<&OutputTarget>,
    ) -> Result<Vec<Value>, EngineError> {
        let resolved = self.discover_alias(name);
        let name = resolved.as_deref().unwrap_or(name);
        if !self.functions.contains_key(&name.to_ascii_lowercase()) {
            if let Some(outputs) = self.invoke_builtin(name, arguments, &input)? {
                return Ok(outputs);
            }
            let destination = output.map_or(NativeOutput::Objects, NativeOutput::Redirected);
            return self.run_native(name, arguments, Some(&input), destination);
        }
        let function = self.function(name)?;
        let (positional, named) = self.evaluate_arguments(arguments)?;
        let saved_trap = self.trap.take();
        let saved_capture = self.capture.take();
        let result = self.pipeline_function_output(&function, &positional, &named, input);
        self.capture = saved_capture;
        self.trap = saved_trap;
        if result.is_ok() {
            self.last_failed = false;
        }
        result
    }

    fn pipeline_function_output(
        &mut self,
        function: &FunctionDef,
        positional: &[Value],
        named: &[(String, Value)],
        input: Vec<Value>,
    ) -> Result<Vec<Value>, EngineError> {
        let mut outputs = Vec::new();
        if function.filter {
            for item in input {
                let mut local = self.variables.child();
                let bound = self.bind_parameters(function, positional, named, &mut local)?;
                local.insert(
                    "args".to_owned(),
                    Value::array(positional[bound..].to_vec()),
                );
                local.insert("_".to_owned(), item);
                let saved = std::mem::replace(&mut self.variables, local);
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
            let mut local = self.variables.child();
            let bound = self.bind_parameters(function, positional, named, &mut local)?;
            local.insert(
                "args".to_owned(),
                Value::array(positional[bound..].to_vec()),
            );
            if !has_blocks {
                local.insert("input".to_owned(), Value::array(input.clone()));
            }
            let saved = std::mem::replace(&mut self.variables, local);
            self.capture = Some(Vec::new());
            let flow = if has_blocks {
                self.exec_named_blocks(function, Some(input), &mut capture_sink())
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
        Ok(outputs)
    }

    fn invoke_builtin(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        input: &[Value],
    ) -> Result<Option<Vec<Value>>, EngineError> {
        let outputs = self.run_builtin(name, arguments, input)?;
        if outputs.is_some() {
            self.last_failed = name.eq_ignore_ascii_case("Write-Error");
        }
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
        let lower = name.to_ascii_lowercase();
        let module = MODULES.iter().find(|module| module.exports_cmdlet(&lower));
        if let Some(module) = module
            && !self.builtin_loaded(module)
        {
            if !self.autoload_unqualified() {
                return Ok(None);
            }
            self.import_builtin(module);
        }
        match lower.as_str() {
            "write-verbose" | "write-debug" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let is_verbose = name.eq_ignore_ascii_case("write-verbose");
                let preference = if is_verbose {
                    "VerbosePreference"
                } else {
                    "DebugPreference"
                };
                let enabled = named.iter().any(|(name, value)| {
                    (name.eq_ignore_ascii_case("verbose") || name.eq_ignore_ascii_case("debug"))
                        && ops::is_true(value)
                }) || self
                    .variables
                    .get(preference)
                    .is_some_and(|value| value.to_string().eq_ignore_ascii_case("continue"));
                if !enabled {
                    return Ok(Some(Vec::new()));
                }
                let mut outputs = Vec::new();
                for value in positional {
                    outputs.extend(flatten_value(value));
                }
                Ok(Some(outputs))
            }
            "write-output" | "write-warning" | "write-information" | "write-error" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                if lower == "write-error" {
                    let action = named
                        .iter()
                        .find(|(key, _)| {
                            key.eq_ignore_ascii_case("ErrorAction")
                                || key.eq_ignore_ascii_case("ea")
                        })
                        .map(|(_, value)| value)
                        .or_else(|| self.variables.get("ErrorActionPreference"));
                    if action.is_some_and(|value| {
                        matches!(
                            value.to_string().to_ascii_lowercase().as_str(),
                            "silentlycontinue" | "ignore"
                        )
                    }) {
                        return Ok(Some(Vec::new()));
                    }
                }
                let no_enumerate = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("noenumerate") && ops::is_true(value)
                });
                let mut values: Vec<Value> = positional;
                values.extend(
                    named
                        .iter()
                        .filter(|(name, _)| name.eq_ignore_ascii_case("inputobject"))
                        .map(|(_, value)| value.clone()),
                );
                if values.is_empty() && lower == "write-output" {
                    values.extend_from_slice(input);
                }
                let mut outputs = Vec::new();
                for value in values {
                    if no_enumerate {
                        outputs.push(value);
                    } else {
                        outputs.extend(flatten_value(value));
                    }
                }
                Ok(Some(outputs))
            }
            "foreach-object" => {
                let mut member_name_named = None;
                for argument in arguments {
                    if let CommandArgument::Parameter { name, value, .. } = argument
                        && name.eq_ignore_ascii_case("membername")
                        && let Some(expr) = value
                    {
                        member_name_named = Some(self.eval_expression(expr)?.to_string());
                    }
                }
                let block_for = |wanted: &str| -> Option<Vec<Statement>> {
                    arguments.iter().find_map(|argument| match argument {
                        CommandArgument::Parameter { name, value, .. }
                            if name.eq_ignore_ascii_case(wanted) =>
                        {
                            value.as_ref().and_then(|expr| match &expr.kind {
                                ExprKind::ScriptBlock { statements, .. } => {
                                    Some(statements.clone())
                                }
                                _ => None,
                            })
                        }
                        _ => None,
                    })
                };
                let begin = block_for("begin");
                let process = block_for("process");
                let end = block_for("end");
                if begin.is_some() || process.is_some() || end.is_some() {
                    let saved = self.capture.take();
                    self.capture = Some(Vec::new());
                    if let Some(begin) = begin {
                        let _ = self.exec_block(&begin, &mut capture_sink())?;
                    }
                    if let Some(process) = process {
                        for item in input {
                            let previous = self.variables.insert("_".to_owned(), item.clone());
                            self.exec_block(&process, &mut capture_sink())?;
                            match previous {
                                Some(value) => self.variables.insert("_".to_owned(), value),
                                None => self.variables.remove("_"),
                            };
                        }
                    }
                    if let Some(end) = end {
                        self.exec_block(&end, &mut capture_sink())?;
                    }
                    let outputs = self.capture.take().unwrap_or_default();
                    self.capture = saved;
                    return Ok(Some(outputs));
                }
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
                let member_name = member_name_named.or_else(|| {
                    arguments.iter().find_map(|argument| match argument {
                        CommandArgument::Bareword(word) => Some(word.clone()),
                        _ => None,
                    })
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
            "where-object" => {
                if script_block_argument(arguments).is_none() {
                    let (positional, named) = self.evaluate_arguments(arguments)?;
                    let property = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                        .map(|(_, value)| value.to_string())
                        .or_else(|| positional.first().map(ToString::to_string));
                    if let Some(property) = property {
                        if let Some((comparison, expected)) =
                            named.iter().find_map(|(name, value)| {
                                comparison_for(name).map(|comparison| (comparison, value))
                            })
                        {
                            let mut outputs = Vec::new();
                            for item in input {
                                let value = member(item, &property).unwrap_or(Value::Null);
                                if matches!(
                                    compare_values(&value, expected, comparison, false)?,
                                    Value::Bool(true)
                                ) {
                                    outputs.push(item.clone());
                                }
                            }
                            return Ok(Some(outputs));
                        }
                        let outputs = input
                            .iter()
                            .filter(|item| {
                                member(item, &property).is_ok_and(|value| ops::is_true(&value))
                            })
                            .cloned()
                            .collect();
                        return Ok(Some(outputs));
                    }
                }
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
                let flag = |wanted: &str| {
                    named
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case(wanted))
                };
                let mut properties: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if properties.iter().any(|name| name.contains(['*', '?']))
                    && let Some(Value::Object(entries)) = input.first()
                {
                    let visible = visible_entries(entries);
                    let mut expanded = Vec::new();
                    for name in &properties {
                        if name.contains(['*', '?']) {
                            for (candidate, _) in &visible {
                                if ops::wildcard_match(candidate, name, false) {
                                    expanded.push((*candidate).clone());
                                }
                            }
                        } else {
                            expanded.push(name.clone());
                        }
                    }
                    if !expanded.is_empty() {
                        properties = expanded;
                    }
                }
                if properties.len() > 1 {
                    let mut outputs = Vec::new();
                    for property in &properties {
                        let total = input.len() as i32;
                        let mut sum = Value::Int32(0);
                        let mut minimum: Option<Value> = None;
                        let mut maximum: Option<Value> = None;
                        let mut average = 0.0;
                        let mut sum_squares = 0.0;
                        let mut count = 0;
                        let mut words = 0;
                        let mut lines = 0;
                        let mut characters = 0;
                        for item in input {
                            let item = match member(item, property) {
                                Ok(value) => value,
                                Err(_) => continue,
                            };
                            let item = &item;
                            let text = item.to_string();
                            words += text.split_whitespace().count();
                            lines += text.lines().count().max(1);
                            characters += text.chars().count();
                            if !is_numeric_value(item) {
                                continue;
                            }
                            count += 1;
                            sum = ops::add(&sum, item)?;
                            let number = convert_double(item)?;
                            average += number;
                            sum_squares += number * number;
                            let compare = |a: &Value, b: &Value| {
                                matches!(
                                    ops::compare(a, b, ops::Comparison::Lt, false),
                                    Ok(Value::Bool(true))
                                )
                            };
                            if minimum
                                .as_ref()
                                .is_none_or(|current| compare(item, current))
                            {
                                minimum = Some(item.clone());
                            }
                            if maximum
                                .as_ref()
                                .is_none_or(|current| compare(current, item))
                            {
                                maximum = Some(item.clone());
                            }
                        }
                        let average = if count > 0 {
                            average / f64::from(count)
                        } else {
                            0.0
                        };
                        let mut entries = vec![("Count".to_owned(), Value::Int32(total))];
                        if flag("sum") {
                            entries.push(("Sum".to_owned(), sum));
                        }
                        if flag("average") {
                            entries.push(("Average".to_owned(), Value::Double(average)));
                        }
                        if flag("minimum") {
                            entries.push(("Minimum".to_owned(), minimum.unwrap_or(Value::Null)));
                        }
                        if flag("maximum") {
                            entries.push(("Maximum".to_owned(), maximum.unwrap_or(Value::Null)));
                        }
                        if flag("word") {
                            entries.push(("Words".to_owned(), Value::Int32(words as i32)));
                        }
                        if flag("line") {
                            entries.push(("Lines".to_owned(), Value::Int32(lines as i32)));
                        }
                        if flag("character") {
                            entries
                                .push(("Characters".to_owned(), Value::Int32(characters as i32)));
                        }
                        if flag("standarddeviation") {
                            let deviation = if count > 1 {
                                let mean = average;
                                ((sum_squares / f64::from(count) - mean * mean) * f64::from(count)
                                    / f64::from(count - 1))
                                .max(0.0)
                                .sqrt()
                            } else {
                                0.0
                            };
                            entries
                                .push(("StandardDeviation".to_owned(), Value::Double(deviation)));
                        }
                        entries.push((
                            "Property".to_owned(),
                            Value::String(property.clone().into()),
                        ));
                        outputs.push(Value::object(entries));
                    }
                    return Ok(Some(outputs));
                }
                let property = properties.first().cloned();
                let total = input.len() as i32;
                let mut sum = Value::Int32(0);
                let mut minimum: Option<Value> = None;
                let mut maximum: Option<Value> = None;
                let mut average = 0.0;
                let mut sum_squares = 0.0;
                let mut count = 0;
                let mut words = 0;
                let mut lines = 0;
                let mut characters = 0;
                for item in input {
                    let item = match &property {
                        Some(property) => match member(item, property) {
                            Ok(value) => value,
                            Err(_) => continue,
                        },
                        None => item.clone(),
                    };
                    let item = &item;
                    let text = item.to_string();
                    words += text.split_whitespace().count();
                    lines += text.lines().count().max(1);
                    characters += text.chars().count();
                    if !is_numeric_value(item) {
                        continue;
                    }
                    count += 1;
                    sum = ops::add(&sum, item)?;
                    let number = convert_double(item)?;
                    average += number;
                    sum_squares += number * number;
                    let compare = |a: &Value, b: &Value| {
                        matches!(
                            ops::compare(a, b, ops::Comparison::Lt, false),
                            Ok(Value::Bool(true))
                        )
                    };
                    if minimum
                        .as_ref()
                        .is_none_or(|current| compare(item, current))
                    {
                        minimum = Some(item.clone());
                    }
                    if maximum
                        .as_ref()
                        .is_none_or(|current| compare(current, item))
                    {
                        maximum = Some(item.clone());
                    }
                }
                let average = if count > 0 {
                    average / f64::from(count)
                } else {
                    0.0
                };
                let mut entries = vec![("Count".to_owned(), Value::Int32(total))];
                if flag("sum") {
                    entries.push(("Sum".to_owned(), sum));
                }
                if flag("average") {
                    entries.push(("Average".to_owned(), Value::Double(average)));
                }
                if flag("minimum") {
                    entries.push(("Minimum".to_owned(), minimum.unwrap_or(Value::Null)));
                }
                if flag("maximum") {
                    entries.push(("Maximum".to_owned(), maximum.unwrap_or(Value::Null)));
                }
                if flag("word") {
                    entries.push(("Words".to_owned(), Value::Int32(words as i32)));
                }
                if flag("line") {
                    entries.push(("Lines".to_owned(), Value::Int32(lines as i32)));
                }
                if flag("character") {
                    entries.push(("Characters".to_owned(), Value::Int32(characters as i32)));
                }
                if flag("standarddeviation") {
                    let deviation = if count > 1 {
                        let mean = average;
                        ((sum_squares / f64::from(count) - mean * mean) * f64::from(count)
                            / f64::from(count - 1))
                        .max(0.0)
                        .sqrt()
                    } else {
                        0.0
                    };
                    entries.push(("StandardDeviation".to_owned(), Value::Double(deviation)));
                }
                Ok(Some(vec![Value::object(entries)]))
            }
            "sort-object" => self.sort_objects(arguments, input).map(Some),
            "group-object" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let property = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                    .map(|(_, value)| value.to_string());
                let case_sensitive = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("casesensitive") && ops::is_true(value)
                });
                let no_element = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("noelement") && ops::is_true(value)
                });
                let as_hashtable = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("ashashtable") && ops::is_true(value)
                });
                let properties: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if properties
                    .iter()
                    .any(|property| property.contains(['*', '?']))
                {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Group-Object with wildcard properties",
                    }
                    .into());
                }
                if properties.len() > 1 {
                    let mut groups: Vec<(String, Vec<Value>, Vec<Value>)> = Vec::new();
                    for item in input {
                        let components: Vec<Value> = properties
                            .iter()
                            .map(|property| member(item, property).unwrap_or(Value::Null))
                            .collect();
                        let key = components
                            .iter()
                            .map(|value| {
                                if case_sensitive {
                                    value.to_string()
                                } else {
                                    value.to_string().to_lowercase()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\u{0}");
                        if let Some((_, _, members)) =
                            groups.iter_mut().find(|(group, _, _)| group == &key)
                        {
                            members.push(item.clone());
                        } else {
                            groups.push((key, components, vec![item.clone()]));
                        }
                    }
                    groups
                        .sort_by(|left, right| left.2[0].to_string().cmp(&right.2[0].to_string()));
                    if as_hashtable {
                        let mut entries: Vec<(String, Value)> = groups
                            .into_iter()
                            .map(|(_, components, members)| {
                                (
                                    components
                                        .iter()
                                        .map(ToString::to_string)
                                        .collect::<Vec<_>>()
                                        .join(", "),
                                    Value::array(members),
                                )
                            })
                            .collect();
                        entries.push((
                            "__TypeName".to_owned(),
                            Value::String("System.Collections.Hashtable".into()),
                        ));
                        return Ok(Some(vec![Value::object(entries)]));
                    }
                    let outputs = groups
                        .into_iter()
                        .map(|(_, components, group)| {
                            let name = components
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", ");
                            let mut entries = vec![
                                ("Values".to_owned(), Value::array(components)),
                                ("Count".to_owned(), Value::Int32(group.len() as i32)),
                            ];
                            if !no_element {
                                entries.push(("Group".to_owned(), Value::array(group)));
                            }
                            entries.push(("Name".to_owned(), Value::String(name.into())));
                            Value::object(entries)
                        })
                        .collect();
                    return Ok(Some(outputs));
                }
                let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
                for item in input {
                    let key = match &property {
                        Some(property) => member(item, property)
                            .map_or_else(|_| String::new(), |value| value.to_string()),
                        None => item.to_string(),
                    };
                    if let Some((_, members)) = groups.iter_mut().find(|(name, _)| {
                        if case_sensitive {
                            name == &key
                        } else {
                            name.eq_ignore_ascii_case(&key)
                        }
                    }) {
                        members.push(item.clone());
                    } else {
                        groups.push((key, vec![item.clone()]));
                    }
                }
                groups.sort_by_key(|(name, _)| name.to_lowercase());
                if as_hashtable {
                    let mut entries: Vec<(String, Value)> = groups
                        .into_iter()
                        .map(|(name, group)| (name, Value::array(group)))
                        .collect();
                    entries.push((
                        "__TypeName".to_owned(),
                        Value::String("System.Collections.Hashtable".into()),
                    ));
                    return Ok(Some(vec![Value::object(entries)]));
                }
                let outputs = groups
                    .into_iter()
                    .map(|(name, group)| {
                        let mut entries = vec![
                            ("Name".to_owned(), Value::String(name.into())),
                            ("Count".to_owned(), Value::Int32(group.len() as i32)),
                        ];
                        if !no_element {
                            entries.push(("Group".to_owned(), Value::array(group)));
                        }
                        Value::object(entries)
                    })
                    .collect();
                Ok(Some(outputs))
            }
            "get-culture" | "get-uiculture" => Ok(Some(vec![globalization::current_culture()])),
            "resolve-path" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                    })
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Some(path) = path else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Resolve-Path without a path",
                    }
                    .into());
                };
                if path.contains(['*', '?']) {
                    let mut matches: Vec<Value> = expand_glob(&path)
                        .into_iter()
                        .map(|path| {
                            let path = std::fs::canonicalize(&path)
                                .map_or(path, |path| path.to_string_lossy().into_owned());
                            Value::object(vec![("Path".to_owned(), Value::String(path.into()))])
                        })
                        .collect();
                    matches.sort_by_key(ToString::to_string);
                    return Ok(Some(matches));
                }
                let resolved = std::fs::canonicalize(&path).map_err(|_| RuntimeError::Script {
                    id: "PathNotFound,Microsoft.PowerShell.Commands.ResolvePathCommand",
                    category: format!(
                        "ObjectNotFound: ({path}:String) [Resolve-Path], ItemNotFoundException"
                    ),
                    exception_type: "System.Management.Automation.ItemNotFoundException",
                    message: format!("Cannot find path '{path}' because it does not exist."),
                })?;
                Ok(Some(vec![Value::object(vec![(
                    "Path".to_owned(),
                    Value::String(resolved.to_string_lossy().to_string().into()),
                )])]))
            }
            "set-item" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                    })
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Some(path) = path else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Set-Item without a path",
                    }
                    .into());
                };
                let value = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("value"))
                    .map(|(_, value)| value.clone())
                    .or_else(|| positional.get(1).cloned())
                    .unwrap_or(Value::Null);
                let Some((provider, name)) = path.split_once(':') else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Set-Item outside a provider",
                    }
                    .into());
                };
                let name = name.trim_start_matches("\\");
                match provider.to_ascii_lowercase().as_str() {
                    "env" => {
                        environment::set(&mut self.variables, name, &value);
                    }
                    "variable" => {
                        let key = strip_scope(name);
                        let key = encoding::variable_name(key).unwrap_or(key).to_owned();
                        if is_global_scope(name) {
                            self.globals.insert(key.clone(), value.clone());
                        }
                        self.variables.insert(key, value);
                    }
                    _ => {
                        return Err(RuntimeError::NotImplemented {
                            feature: "Set-Item for this provider",
                        }
                        .into());
                    }
                }
                Ok(Some(Vec::new()))
            }
            "set-itemproperty" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional
                        .first()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_default(),
                };
                let name = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                    .map(|(_, value)| value.to_string());
                let value = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("value"))
                    .map(|(_, value)| value.clone())
                    .or_else(|| positional.get(1).cloned());
                let (Some(name), Some(value)) = (name, value) else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Set-ItemProperty without a name and value",
                    }
                    .into());
                };
                if !name.eq_ignore_ascii_case("IsReadOnly") {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Set-ItemProperty with this property",
                    }
                    .into());
                }
                let readonly = ops::is_true(&value);
                for path in paths {
                    let metadata = std::fs::metadata(&path).map_err(|_| RuntimeError::Script {
                        id: "PathNotFound,Microsoft.PowerShell.Commands.SetItemPropertyCommand",
                        category: format!(
                            "ObjectNotFound: ({path}:String) [Set-ItemProperty], ItemNotFoundException"
                        ),
                        exception_type: "System.Management.Automation.ItemNotFoundException",
                        message: format!("Cannot find path '{path}' because it does not exist."),
                    })?;
                    let mut permissions = metadata.permissions();
                    permissions.set_readonly(readonly);
                    std::fs::set_permissions(&path, permissions).map_err(|_| {
                        RuntimeError::NotImplemented {
                            feature: "Set-ItemProperty cannot write this file",
                        }
                    })?;
                }
                Ok(Some(Vec::new()))
            }
            "get-itemproperty" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Get-ItemProperty without a path",
                    }
                    .into());
                }
                let names: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut outputs = Vec::new();
                for path in paths {
                    let path = expand_home(&path);
                    let matched = if path.contains(['*', '?']) {
                        expand_glob(&path)
                    } else {
                        vec![path]
                    };
                    for path in matched {
                        if !std::path::Path::new(&path).exists() {
                            return Err(RuntimeError::Script {
                                id: "PathNotFound,Microsoft.PowerShell.Commands.GetItemPropertyCommand",
                                category: format!(
                                    "ObjectNotFound: ({path}:String) [Get-ItemProperty], ItemNotFoundException"
                                ),
                                exception_type: "System.Management.Automation.ItemNotFoundException",
                                message: format!(
                                    "Cannot find path '{path}' because it does not exist."
                                ),
                            }
                            .into());
                        }
                        let value = item_value(&path);
                        if !names.is_empty()
                            && let Value::Object(entries) = &value
                        {
                            outputs.push(Value::object(
                                entries
                                    .iter()
                                    .filter(|(key, _)| {
                                        key == "__TypeName"
                                            || names
                                                .iter()
                                                .any(|name| name.eq_ignore_ascii_case(key))
                                    })
                                    .map(|(key, value)| (key.clone(), value.clone()))
                                    .collect(),
                            ));
                            continue;
                        }
                        outputs.push(value);
                    }
                }
                Ok(Some(outputs))
            }
            "get-item" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Get-Item without a path",
                    }
                    .into());
                }
                let mut outputs = Vec::new();
                for path in paths {
                    if path.contains(['*', '?'])
                        && let Some(items) = self.provider_children(&path, false, None)
                    {
                        outputs.extend(items);
                        continue;
                    }
                    if let Some(item) = self.provider_item(&path) {
                        outputs.push(item?);
                        continue;
                    }
                    let path = expand_home(&path);
                    if path.contains(['*', '?']) {
                        for matched in expand_glob(&path) {
                            outputs.push(item_value(&matched));
                        }
                        continue;
                    }
                    if !std::path::Path::new(&path).exists() {
                        return Err(RuntimeError::Script {
                            id: "PathNotFound,Microsoft.PowerShell.Commands.GetItemCommand",
                            category: format!(
                                "ObjectNotFound: ({path}:String) [Get-Item], ItemNotFoundException"
                            ),
                            exception_type: "System.Management.Automation.ItemNotFoundException",
                            message: format!(
                                "Cannot find path '{path}' because it does not exist."
                            ),
                        }
                        .into());
                    }
                    outputs.push(item_value(&path));
                }
                Ok(Some(outputs))
            }
            "rename-item" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                            || name.eq_ignore_ascii_case("literalpath")
                    })
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let new_name = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("newname"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.get(1).map(ToString::to_string));
                let (Some(path), Some(new_name)) = (path, new_name) else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Rename-Item without a path and new name",
                    }
                    .into());
                };
                let target = std::path::Path::new(&path).parent().map_or_else(
                    || std::path::PathBuf::from(&new_name),
                    |parent| parent.join(&new_name),
                );
                std::fs::rename(&path, &target).map_err(|_| RuntimeError::Script {
                    id: "PathNotFound,Microsoft.PowerShell.Commands.RenameItemCommand",
                    category: format!(
                        "ObjectNotFound: ({path}:String) [Rename-Item], ItemNotFoundException"
                    ),
                    exception_type: "System.Management.Automation.ItemNotFoundException",
                    message: format!("Cannot find path '{path}' because it does not exist."),
                })?;
                Ok(Some(Vec::new()))
            }
            "new-object" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let type_name = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("typename"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Some(type_name) = type_name else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "New-Object without a type name",
                    }
                    .into());
                };
                if type_name.eq_ignore_ascii_case("PSObject")
                    || type_name.eq_ignore_ascii_case("PSCustomObject")
                    || type_name.eq_ignore_ascii_case("System.Management.Automation.PSCustomObject")
                {
                    if let Some((_, property)) = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                        && let Value::Object(entries) = property
                    {
                        return Ok(Some(vec![Value::object(
                            visible_entries(entries)
                                .iter()
                                .map(|(name, value)| ((*name).clone(), (*value).clone()))
                                .collect(),
                        )]));
                    }
                    return Ok(Some(vec![Value::object(Vec::new())]));
                }
                Err(RuntimeError::NotImplemented {
                    feature: "New-Object with this type",
                }
                .into())
            }
            "new-item" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                    })
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Some(path) = path else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "New-Item without a path",
                    }
                    .into());
                };
                let provider_path = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                    .map(|(_, value)| value.to_string())
                    .filter(|_| path.ends_with(':'))
                    .map_or_else(|| path.clone(), |name| format!("{path}{name}"));
                if let Some((provider, name)) = provider_path.split_once(':') {
                    let name = name.trim_start_matches("\\");
                    if !name.is_empty() && provider.eq_ignore_ascii_case("alias") {
                        let value = named
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case("value"))
                            .map(|(_, value)| value.clone())
                            .unwrap_or(Value::Null);
                        let definition = value.to_string();
                        if self.aliases.get(name).is_some()
                            && !named.iter().any(|(key, value)| {
                                key.eq_ignore_ascii_case("force") && ops::is_true(value)
                            })
                        {
                            return Err(RuntimeError::NotImplemented {
                                feature: "New-Item on an existing alias without Force",
                            }
                            .into());
                        }
                        let alias = self.aliases.set(name.to_owned(), definition);
                        return Ok(Some(vec![alias.value()]));
                    }
                    if !name.is_empty()
                        && (provider.eq_ignore_ascii_case("env")
                            || provider.eq_ignore_ascii_case("variable"))
                    {
                        let value = named
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case("value"))
                            .map(|(_, value)| value.clone())
                            .unwrap_or(Value::Null);
                        let value = if provider.eq_ignore_ascii_case("env") {
                            environment::set(&mut self.variables, name, &value);
                            environment::get(&self.variables, name).unwrap_or(Value::Null)
                        } else {
                            self.variables
                                .insert(strip_scope(name).to_owned(), value.clone());
                            value
                        };
                        return Ok(Some(vec![Value::object(vec![
                            ("Name".to_owned(), Value::String(name.into())),
                            ("Value".to_owned(), value),
                        ])]));
                    }
                }
                let item_type = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("itemtype"))
                    .map_or_else(|| "file".to_owned(), |(_, value)| value.to_string());
                let path = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                    .map(|(_, value)| value.to_string())
                {
                    Some(name) => format!("{path}/{name}"),
                    None => path,
                };
                let target = std::path::Path::new(&path);
                if item_type.eq_ignore_ascii_case("symboliclink") {
                    let Some(link_target) = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("target"))
                        .map(|(_, value)| value.to_string())
                    else {
                        return Err(RuntimeError::NotImplemented {
                            feature: "New-Item SymbolicLink without -Target",
                        }
                        .into());
                    };
                    #[cfg(unix)]
                    {
                        std::os::unix::fs::symlink(&link_target, target).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "New-Item cannot create this symlink",
                            }
                        })?;
                        return Ok(Some(Vec::new()));
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = link_target;
                        return Err(RuntimeError::NotImplemented {
                            feature: "New-Item SymbolicLink on this platform",
                        }
                        .into());
                    }
                }
                if item_type.eq_ignore_ascii_case("directory") {
                    std::fs::create_dir_all(target).map_err(|_| RuntimeError::NotImplemented {
                        feature: "New-Item cannot create this directory",
                    })?;
                } else {
                    if let Some(parent) = target.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let value = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("value"))
                        .map_or_else(String::new, |(_, value)| value.to_string());
                    std::fs::write(target, value.as_bytes()).map_err(|_| {
                        RuntimeError::NotImplemented {
                            feature: "New-Item cannot create this file",
                        }
                    })?;
                }
                Ok(Some(vec![Value::object(vec![
                    (
                        "__TypeName".to_owned(),
                        Value::String(
                            if item_type.eq_ignore_ascii_case("directory") {
                                "System.IO.DirectoryInfo"
                            } else {
                                "System.IO.FileInfo"
                            }
                            .into(),
                        ),
                    ),
                    ("FullName".to_owned(), Value::String(path.into())),
                ])]))
            }
            "remove-item" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Remove-Item without a path",
                    }
                    .into());
                }
                let mut targets = Vec::new();
                for path in paths {
                    let provider_path = path.split_once(':').is_some_and(|(provider, _)| {
                        matches!(
                            provider.to_ascii_lowercase().as_str(),
                            "env" | "variable" | "alias" | "function"
                        )
                    });
                    if path.contains(['*', '?']) && !provider_path {
                        targets.extend(expand_glob(&path));
                    } else {
                        targets.push(path);
                    }
                }
                for path in targets {
                    if let Some((provider, name)) = path.split_once(':') {
                        let name = name.trim_start_matches("\\");
                        let wildcard = name.contains(['*', '?']);
                        match provider.to_ascii_lowercase().as_str() {
                            "env" => {
                                if wildcard {
                                    for key in environment::entries(&self.variables)
                                        .keys()
                                        .filter(|key| ops::wildcard_match(key, name, false))
                                    {
                                        environment::set(&mut self.variables, key, &Value::Null);
                                    }
                                } else {
                                    environment::set(&mut self.variables, name, &Value::Null);
                                }
                                continue;
                            }
                            "variable" => {
                                if wildcard {
                                    let keys: Vec<String> = self
                                        .variables
                                        .keys()
                                        .filter(|key| ops::wildcard_match(key, name, false))
                                        .cloned()
                                        .collect();
                                    for key in keys {
                                        self.variables.remove(&key);
                                        self.globals.remove(&key);
                                    }
                                } else {
                                    let key = strip_scope(name);
                                    self.variables.remove(key);
                                    self.globals.remove(key);
                                }
                                continue;
                            }
                            "alias" => {
                                let force = named.iter().any(|(key, value)| {
                                    key.eq_ignore_ascii_case("force") && ops::is_true(value)
                                });
                                let keys: Vec<String> = if wildcard {
                                    self.aliases
                                        .iter()
                                        .filter(|alias| {
                                            ops::wildcard_match(alias.name, name, false)
                                        })
                                        .map(|alias| alias.name.to_owned())
                                        .collect()
                                } else {
                                    vec![name.to_owned()]
                                };
                                if !force
                                    && keys.iter().any(|key| {
                                        self.aliases
                                            .get(key)
                                            .is_some_and(|alias| alias.options.read_only())
                                    })
                                {
                                    return Err(RuntimeError::NotImplemented {
                                        feature: "removing a read-only alias without Force",
                                    }
                                    .into());
                                }
                                for key in keys {
                                    self.aliases.remove(&key);
                                }
                                continue;
                            }
                            "function" => {
                                if wildcard {
                                    let keys: Vec<String> = self
                                        .functions
                                        .keys()
                                        .filter(|key| ops::wildcard_match(key, name, false))
                                        .cloned()
                                        .collect();
                                    for key in keys {
                                        self.functions.remove(&key);
                                    }
                                } else {
                                    self.functions.remove(&name.to_ascii_lowercase());
                                }
                                continue;
                            }
                            _ => {}
                        }
                    }
                    let target = std::path::Path::new(&path);
                    if target.is_dir() {
                        std::fs::remove_dir_all(target).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "Remove-Item cannot remove this directory",
                            }
                        })?;
                    } else if target.exists() {
                        std::fs::remove_file(target).map_err(|_| RuntimeError::NotImplemented {
                            feature: "Remove-Item cannot remove this file",
                        })?;
                    }
                }
                Ok(Some(Vec::new()))
            }
            "copy-item" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let destination_named = named
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("destination"));
                let mut paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None if destination_named => {
                        positional.iter().map(ToString::to_string).collect()
                    }
                    None => positional
                        .first()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_default(),
                };
                let destination = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("destination"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.get(1).map(ToString::to_string));
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Copy-Item without source and destination",
                    }
                    .into());
                }
                let Some(destination) = destination else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Copy-Item without source and destination",
                    }
                    .into());
                };
                let recurse = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("recurse") && ops::is_true(value)
                });
                let mut sources = Vec::new();
                for path in paths.drain(..) {
                    if path.contains(['*', '?']) {
                        sources.extend(expand_glob(&path));
                    } else {
                        sources.push(path);
                    }
                }
                for source in sources {
                    let target = if std::path::Path::new(&destination).is_dir() {
                        let name = std::path::Path::new(&source)
                            .file_name()
                            .map_or_else(String::new, |name| name.to_string_lossy().to_string());
                        format!("{destination}/{name}")
                    } else {
                        destination.clone()
                    };
                    if std::path::Path::new(&source).is_dir() && recurse {
                        copy_directory(&source, &target).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "Copy-Item cannot copy this directory",
                            }
                        })?;
                    } else {
                        std::fs::copy(&source, &target).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "Copy-Item cannot copy this file",
                            }
                        })?;
                    }
                }
                Ok(Some(Vec::new()))
            }
            "move-item" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let destination_named = named
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("destination"));
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None if destination_named => {
                        positional.iter().map(ToString::to_string).collect()
                    }
                    None => positional
                        .first()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_default(),
                };
                let destination = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("destination"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.get(1).map(ToString::to_string));
                let Some(destination) = destination else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Move-Item without source and destination",
                    }
                    .into());
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Move-Item without source and destination",
                    }
                    .into());
                }
                let mut sources = Vec::new();
                for path in paths {
                    if path.contains(['*', '?']) {
                        sources.extend(expand_glob(&path));
                    } else {
                        sources.push(path);
                    }
                }
                for source in sources {
                    let target = if std::path::Path::new(&destination).is_dir() {
                        let name = std::path::Path::new(&source)
                            .file_name()
                            .map_or_else(String::new, |name| name.to_string_lossy().to_string());
                        format!("{destination}/{name}")
                    } else {
                        destination.clone()
                    };
                    std::fs::rename(&source, &target).map_err(|_| {
                        RuntimeError::NotImplemented {
                            feature: "Move-Item cannot move this file",
                        }
                    })?;
                }
                Ok(Some(Vec::new()))
            }
            "get-childitem" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let mut paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty() {
                    paths.push(".".to_owned());
                }
                let recurse = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("recurse") && ops::is_true(value)
                });
                let files_only = named
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("file") && ops::is_true(value));
                let directories_only = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("directory") && ops::is_true(value)
                });
                let names_only = named
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("name") && ops::is_true(value));
                let filter = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("filter"))
                    .map(|(_, value)| value.to_string());
                let force = named
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("force") && ops::is_true(value));
                let attributes: Vec<(bool, String)> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("attributes"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(|value| {
                                let text = value.to_string();
                                match text.strip_prefix('!') {
                                    Some(rest) => (true, rest.to_ascii_lowercase()),
                                    None => (false, text.to_ascii_lowercase()),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                for (_, attribute) in &attributes {
                    if attribute != "directory"
                        && attribute != "hidden"
                        && attribute != "readonly"
                        && attribute != "reparsepoint"
                    {
                        return Err(RuntimeError::NotImplemented {
                            feature: "Get-ChildItem with this attribute",
                        }
                        .into());
                    }
                }
                if paths.len() == 1
                    && let Some(items) =
                        self.provider_children(&paths[0], names_only, filter.as_deref())
                {
                    return Ok(Some(items));
                }
                let exclude: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("exclude"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let include: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("include"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let depth = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("depth"))
                    .map(|(_, value)| convert_int32(value))
                    .transpose()?;
                let recurse = recurse || depth.is_some();
                let wildcard_path = paths.iter().any(|path| path.contains(['*', '?']));
                let metadata_base = if paths.len() == 1 && !wildcard_path {
                    Some(expand_home(&paths[0]))
                } else {
                    None
                };
                let mut entries: Vec<(String, bool, u64)> = Vec::new();
                for path in &paths {
                    let path = expand_home(path);
                    if path.contains(['*', '?']) {
                        for matched in expand_glob(&path) {
                            let target = std::path::Path::new(&matched);
                            let is_dir = target.is_dir();
                            let length = target.metadata().map_or(0, |meta| meta.len());
                            entries.push((matched, is_dir, length));
                        }
                    } else if std::path::Path::new(&path).is_file() {
                        let length = std::fs::metadata(&path).map_or(0, |meta| meta.len());
                        let name = if names_only {
                            path.rsplit('/').next().unwrap_or(&path).to_owned()
                        } else {
                            path
                        };
                        entries.push((name, false, length));
                    } else {
                        let include_hidden = force
                            || attributes
                                .iter()
                                .any(|(negated, attribute)| !negated && attribute == "hidden");
                        collect_children(&path, recurse, include_hidden, "", &mut entries);
                    }
                }
                if let Some(depth) = depth {
                    let depth = depth.max(0) as usize;
                    entries.retain(|(name, _, _)| name.matches('/').count() <= depth);
                }
                entries.retain(|(name, is_dir, _)| {
                    let leaf = name.rsplit('/').next().unwrap_or(name);
                    for (negated, attribute) in &attributes {
                        let metadata_path = metadata_base.as_ref().map_or_else(
                            || name.clone(),
                            |base| {
                                format!(
                                    "{}/{}",
                                    base.trim_end_matches('/'),
                                    name.trim_start_matches('/')
                                )
                            },
                        );
                        let value = if attribute == "directory" {
                            *is_dir
                        } else if attribute == "readonly" {
                            std::fs::metadata(name)
                                .map(|meta| meta.permissions().readonly())
                                .unwrap_or(false)
                        } else if attribute == "reparsepoint" {
                            std::fs::symlink_metadata(&metadata_path)
                                .map(|meta| meta.file_type().is_symlink())
                                .unwrap_or(false)
                        } else {
                            leaf.starts_with('.')
                        };
                        if value == *negated {
                            return false;
                        }
                    }
                    if files_only && *is_dir {
                        return false;
                    }
                    if directories_only && !*is_dir {
                        return false;
                    }
                    let leaf = name.rsplit('/').next().unwrap_or(name);
                    if exclude
                        .iter()
                        .any(|pattern| ops::wildcard_match(leaf, pattern, false))
                    {
                        return false;
                    }
                    if !include.is_empty()
                        && !include
                            .iter()
                            .any(|pattern| ops::wildcard_match(leaf, pattern, false))
                    {
                        return false;
                    }
                    match &filter {
                        Some(filter) => ops::wildcard_match(leaf, filter, false),
                        None => true,
                    }
                });
                entries.sort_by_key(|(name, is_dir, _)| (!*is_dir, name.to_lowercase()));
                Ok(Some(
                    entries
                        .into_iter()
                        .map(|(name, is_dir, length)| {
                            if names_only {
                                let leaf = if wildcard_path {
                                    name.rsplit('/').next().unwrap_or(&name).to_owned()
                                } else {
                                    name.clone()
                                };
                                Value::String(leaf.into())
                            } else {
                                let leaf = name.rsplit('/').next().unwrap_or(&name).to_owned();
                                let mut entries = vec![
                                    (
                                        "__TypeName".to_owned(),
                                        Value::String(
                                            if is_dir {
                                                "System.IO.DirectoryInfo"
                                            } else {
                                                "System.IO.FileInfo"
                                            }
                                            .into(),
                                        ),
                                    ),
                                    ("Name".to_owned(), Value::String(leaf.clone().into())),
                                    ("FullName".to_owned(), Value::String(name.clone().into())),
                                    ("PSIsContainer".to_owned(), Value::Bool(is_dir)),
                                    (
                                        "Mode".to_owned(),
                                        Value::String(
                                            if is_dir { "d----" } else { "-----" }.into(),
                                        ),
                                    ),
                                    ("Length".to_owned(), Value::Int64(length as i64)),
                                ];
                                let metadata_path = metadata_base.as_ref().map_or_else(
                                    || name.clone(),
                                    |base| {
                                        format!(
                                            "{}/{}",
                                            base.trim_end_matches('/'),
                                            name.trim_start_matches('/')
                                        )
                                    },
                                );
                                if let Ok(metadata) = std::fs::symlink_metadata(&metadata_path)
                                    && metadata.file_type().is_symlink()
                                {
                                    entries.push((
                                        "LinkType".to_owned(),
                                        Value::String("SymbolicLink".into()),
                                    ));
                                    entries.push((
                                        "Target".to_owned(),
                                        Value::String(
                                            std::fs::read_link(&metadata_path)
                                                .map_or_else(
                                                    |_| String::new(),
                                                    |path| path.to_string_lossy().into_owned(),
                                                )
                                                .into(),
                                        ),
                                    ));
                                }
                                if !is_dir {
                                    let path = std::path::Path::new(&leaf);
                                    entries.push((
                                        "BaseName".to_owned(),
                                        Value::String(
                                            path.file_stem()
                                                .map_or_else(String::new, |value| {
                                                    value.to_string_lossy().into_owned()
                                                })
                                                .into(),
                                        ),
                                    ));
                                    entries.push((
                                        "Extension".to_owned(),
                                        Value::String(
                                            path.extension()
                                                .map_or_else(String::new, |value| {
                                                    format!(".{}", value.to_string_lossy())
                                                })
                                                .into(),
                                        ),
                                    ));
                                }
                                Value::object(entries)
                            }
                        })
                        .collect(),
                ))
            }
            "out-null" => Ok(Some(Vec::new())),
            "out-host" | "out-default" => Ok(Some(input.to_vec())),
            "join-string" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let separator = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("separator"))
                    .map_or_else(String::new, |(_, value)| value.to_string());
                let property = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                    .map(|(_, value)| value.to_string());
                let double_quote = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("doublequote") && ops::is_true(value)
                });
                let single_quote = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("singlequote") && ops::is_true(value)
                });
                let prefix = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("outputprefix"))
                    .map_or_else(String::new, |(_, value)| value.to_string());
                let suffix = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("outputsuffix"))
                    .map_or_else(String::new, |(_, value)| value.to_string());
                let format_string = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("formatstring"))
                    .map(|(_, value)| value.to_string());
                let values: Vec<String> = input
                    .iter()
                    .map(|item| match &property {
                        Some(property) => member(item, property)
                            .map_or_else(|_| String::new(), |value| value.to_string()),
                        None => item.to_string(),
                    })
                    .map(|value| {
                        if double_quote {
                            format!("\"{}\"", value.replace('"', "\"\""))
                        } else if single_quote {
                            format!("'{}'", value.replace('\'', "''"))
                        } else {
                            value
                        }
                    })
                    .map(|value| match &format_string {
                        Some(format) => format.replace("{0}", &value),
                        None => value,
                    })
                    .collect();
                Ok(Some(vec![Value::String(
                    format!("{prefix}{}{suffix}", values.join(&separator)).into(),
                )]))
            }
            "clear-content" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.iter().any(|path| {
                    path.split_once(':').is_some_and(|(provider, name)| {
                        !name.is_empty()
                            && matches!(
                                provider.to_ascii_lowercase().as_str(),
                                "env" | "variable" | "alias" | "function"
                            )
                    })
                }) {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Clear-Content for this provider",
                    }
                    .into());
                }
                for path in paths {
                    let files = if path.contains(['*', '?']) {
                        expand_glob(&path)
                    } else {
                        vec![path]
                    };
                    for path in files {
                        std::fs::write(&path, b"").map_err(|_| RuntimeError::NotImplemented {
                            feature: "Clear-Content cannot write this file",
                        })?;
                    }
                }
                Ok(Some(Vec::new()))
            }
            "add-content" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path_named = named.iter().any(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                });
                let paths: Vec<String> = if path_named {
                    named
                        .iter()
                        .find(|(name, _)| {
                            name.eq_ignore_ascii_case("path")
                                || name.eq_ignore_ascii_case("literalpath")
                        })
                        .map(|(_, value)| {
                            flatten_value(value.clone())
                                .iter()
                                .map(ToString::to_string)
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    positional
                        .first()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_default()
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Add-Content without a path",
                    }
                    .into());
                }
                if paths.iter().any(|path| {
                    path.split_once(':').is_some_and(|(provider, name)| {
                        !name.is_empty()
                            && matches!(
                                provider.to_ascii_lowercase().as_str(),
                                "env" | "variable" | "alias" | "function"
                            )
                    })
                }) {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Add-Content for this provider",
                    }
                    .into());
                }
                let values: Vec<Value> = if !input.is_empty() {
                    input.to_vec()
                } else {
                    let named_values: Vec<Value> = named
                        .iter()
                        .filter(|(name, _)| name.eq_ignore_ascii_case("value"))
                        .map(|(_, value)| value.clone())
                        .collect();
                    if named_values.is_empty() {
                        let skip = usize::from(!path_named);
                        positional
                            .iter()
                            .skip(skip)
                            .flat_map(|value| flatten_value(value.clone()))
                            .collect()
                    } else {
                        named_values
                    }
                };
                let no_newline = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("nonewline") && ops::is_true(value)
                });
                let mut text = String::new();
                for value in flatten_values(values) {
                    text.push_str(&value.to_string());
                    if !no_newline {
                        text.push('\n');
                    }
                }
                use std::io::Write as _;
                for path in paths {
                    let files = if path.contains(['*', '?']) {
                        expand_glob(&path)
                    } else {
                        vec![path]
                    };
                    for path in files {
                        let mut file = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                            .map_err(|_| RuntimeError::NotImplemented {
                                feature: "Add-Content cannot open this file",
                            })?;
                        file.write_all(text.as_bytes()).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "Add-Content cannot write this file",
                            }
                        })?;
                    }
                }
                Ok(Some(Vec::new()))
            }
            "out-file" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("filepath"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Some(path) = path else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Out-File without a path",
                    }
                    .into());
                };
                let no_newline = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("nonewline") && ops::is_true(value)
                });
                let mut text = String::new();
                for item in input {
                    text.push_str(&item.to_string());
                    if !no_newline {
                        text.push('\n');
                    }
                }
                if named
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("append") && ops::is_true(value))
                {
                    use std::io::Write as _;
                    let mut file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .map_err(|_| RuntimeError::NotImplemented {
                            feature: "Out-File cannot open this file",
                        })?;
                    file.write_all(text.as_bytes())
                        .map_err(|_| RuntimeError::NotImplemented {
                            feature: "Out-File cannot write this file",
                        })?;
                } else {
                    std::fs::write(&path, text).map_err(|_| RuntimeError::NotImplemented {
                        feature: "Out-File cannot write this file",
                    })?;
                }
                Ok(Some(Vec::new()))
            }
            "get-content" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Get-Content without a path",
                    }
                    .into());
                }
                if paths.iter().any(|path| {
                    path.split_once(':').is_some_and(|(provider, name)| {
                        !name.is_empty()
                            && matches!(
                                provider.to_ascii_lowercase().as_str(),
                                "alias" | "function"
                            )
                    })
                }) {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Set-Content for this provider",
                    }
                    .into());
                }
                if named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("asbytestream") && ops::is_true(value)
                }) {
                    let mut outputs = Vec::new();
                    for path in paths {
                        let path = expand_home(&path);
                        let files = if path.contains(['*', '?']) {
                            expand_glob(&path)
                        } else {
                            vec![path]
                        };
                        for file in files {
                            let bytes =
                                std::fs::read(&file).map_err(|_| RuntimeError::NotImplemented {
                                    feature: "Get-Content of a missing file",
                                })?;
                            outputs.extend(bytes.into_iter().map(Value::Byte));
                        }
                    }
                    return Ok(Some(outputs));
                }
                let mut text = String::new();
                for path in paths {
                    if let Some((provider, name)) = path.split_once(':') {
                        let name = name.trim_start_matches("\\");
                        if !name.is_empty() {
                            let value = if provider.eq_ignore_ascii_case("env") {
                                environment::get(&self.variables, name)
                            } else if provider.eq_ignore_ascii_case("variable") {
                                let key = strip_scope(name);
                                self.variables
                                    .get(key)
                                    .cloned()
                                    .or_else(|| automatic_variable(key))
                            } else {
                                None
                            };
                            if let Some(value) = value {
                                if !text.is_empty() && !text.ends_with('\n') {
                                    text.push('\n');
                                }
                                text.push_str(&value.to_string());
                                continue;
                            }
                        }
                    }
                    let path = expand_home(&path);
                    if path.contains(['*', '?']) {
                        for matched in expand_glob(&path) {
                            let Ok(content) = std::fs::read_to_string(&matched) else {
                                continue;
                            };
                            if !text.is_empty() && !text.ends_with('\n') {
                                text.push('\n');
                            }
                            text.push_str(&content);
                        }
                    } else {
                        let content = std::fs::read_to_string(&path).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "Get-Content of a missing file",
                            }
                        })?;
                        if !text.is_empty() && !text.ends_with('\n') {
                            text.push('\n');
                        }
                        text.push_str(&content);
                    }
                }
                if named
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("raw") && ops::is_true(value))
                {
                    return Ok(Some(vec![Value::String(text.into())]));
                }
                if let Some((_, delimiter)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("delimiter"))
                {
                    let delimiter = delimiter.to_string();
                    let mut parts: Vec<Value> = text
                        .split(&delimiter)
                        .map(|part| Value::String(part.into()))
                        .collect();
                    if text.ends_with(&delimiter) {
                        parts.pop();
                    }
                    return Ok(Some(parts));
                }
                let mut lines: Vec<Value> = text
                    .split('\n')
                    .map(|line| Value::String(line.trim_end_matches('\r').into()))
                    .collect();
                if text.ends_with('\n') {
                    lines.pop();
                }
                if let Some((_, count)) = named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("totalcount")
                        || name.eq_ignore_ascii_case("first")
                        || name.eq_ignore_ascii_case("head")
                        || name.eq_ignore_ascii_case("totalcount")
                }) {
                    let count = convert_int32(count)?.max(0) as usize;
                    lines.truncate(count);
                }
                if let Some((_, count)) = named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("tail") || name.eq_ignore_ascii_case("last")
                }) {
                    let count = convert_int32(count)?.max(0) as usize;
                    if lines.len() > count {
                        lines = lines.split_off(lines.len() - count);
                    }
                }
                Ok(Some(lines))
            }
            "test-path" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let mut paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty()
                    && let Some((_, value)) = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("isvalid"))
                    && let Value::String(text) = value
                {
                    paths.push(text.to_string_lossy());
                }
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Test-Path without a path",
                    }
                    .into());
                }
                let is_valid = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("isvalid") && ops::is_true(value)
                });
                let path_type = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("pathtype"))
                    .map(|(_, value)| value.to_string());
                let mut outputs = Vec::new();
                for path in paths {
                    let path_text = expand_home(&path);
                    if is_valid {
                        let valid = !path_text.is_empty()
                            && !path_text.contains('\0')
                            && !path_text.contains(':');
                        outputs.push(Value::Bool(valid));
                        continue;
                    }
                    if let Some(found) = self.provider_path_exists(&path_text) {
                        outputs.push(Value::Bool(found));
                        continue;
                    }
                    let matches = if path_text.contains(['*', '?']) {
                        expand_glob(&path_text).into_iter().any(|path| {
                            let target = std::path::Path::new(&path);
                            match path_type.as_deref() {
                                Some(kind) if kind.eq_ignore_ascii_case("leaf") => target.is_file(),
                                Some(kind) if kind.eq_ignore_ascii_case("container") => {
                                    target.is_dir()
                                }
                                _ => target.exists(),
                            }
                        })
                    } else {
                        let target = std::path::Path::new(&path_text);
                        match path_type.as_deref() {
                            Some(kind) if kind.eq_ignore_ascii_case("leaf") => target.is_file(),
                            Some(kind) if kind.eq_ignore_ascii_case("container") => target.is_dir(),
                            _ => target.exists(),
                        }
                    };
                    outputs.push(Value::Bool(matches));
                }
                Ok(Some(outputs))
            }
            "join-path" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let named_paths: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("path"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let child = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("childpath"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| {
                        if named_paths.is_empty() {
                            positional.get(1).map(ToString::to_string)
                        } else {
                            None
                        }
                    });
                let additional: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("additionalchildpath"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut roots = named_paths;
                if roots.is_empty()
                    && let Some(first) = positional.first()
                {
                    roots.push(first.to_string());
                }
                if roots.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Join-Path without a path",
                    }
                    .into());
                }
                let mut outputs = Vec::new();
                for root in roots {
                    let mut parts = vec![root];
                    if let Some(child) = &child {
                        parts.push(child.clone());
                    }
                    parts.extend(additional.iter().cloned());
                    outputs.push(Value::String(parts.join("/").into()));
                }
                Ok(Some(outputs))
            }
            "split-path" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                    })
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Some(path) = path else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Split-Path without a path",
                    }
                    .into());
                };
                if named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("noqualifier") && ops::is_true(value)
                }) {
                    return Ok(Some(vec![Value::String(path.into())]));
                }
                let path = std::path::Path::new(&path);
                if named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("isabsolute") && ops::is_true(value)
                }) {
                    return Ok(Some(vec![Value::Bool(path.is_absolute())]));
                }
                if named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("extension") && ops::is_true(value)
                }) {
                    let extension = path
                        .extension()
                        .map(|value| format!(".{}", value.to_string_lossy()))
                        .unwrap_or_default();
                    return Ok(Some(vec![Value::String(extension.into())]));
                }
                if named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("leafbase") && ops::is_true(value)
                }) {
                    let base = path
                        .file_stem()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    return Ok(Some(vec![Value::String(base.into())]));
                }
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
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let separator = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("separator"))
                    .map_or_else(|| " ".to_owned(), |(_, value)| value.to_string());
                let mut parts = Vec::new();
                for value in positional {
                    for item in flatten_value(value) {
                        parts.push(item.to_string());
                    }
                }
                Ok(Some(vec![Value::String(parts.join(&separator).into())]))
            }
            "set-content" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path_named = named.iter().any(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                });
                let paths: Vec<String> = if path_named {
                    named
                        .iter()
                        .find(|(name, _)| {
                            name.eq_ignore_ascii_case("path")
                                || name.eq_ignore_ascii_case("literalpath")
                        })
                        .map(|(_, value)| {
                            flatten_value(value.clone())
                                .iter()
                                .map(ToString::to_string)
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    positional
                        .first()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_default()
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Set-Content without a path",
                    }
                    .into());
                }
                let no_newline = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("nonewline") && ops::is_true(value)
                });
                let mut text = String::new();
                let values: Vec<Value> = if !input.is_empty() {
                    input.to_vec()
                } else {
                    let named_values: Vec<Value> = named
                        .iter()
                        .filter(|(name, _)| name.eq_ignore_ascii_case("value"))
                        .map(|(_, value)| value.clone())
                        .collect();
                    if named_values.is_empty() {
                        let skip = usize::from(!path_named);
                        positional
                            .iter()
                            .skip(skip)
                            .flat_map(|value| flatten_value(value.clone()))
                            .collect()
                    } else {
                        named_values
                    }
                };
                let values = flatten_values(values);
                let provider_paths = paths.iter().all(|path| {
                    path.split_once(':').is_some_and(|(provider, name)| {
                        !name.is_empty()
                            && (provider.eq_ignore_ascii_case("env")
                                || provider.eq_ignore_ascii_case("variable"))
                    })
                });
                if provider_paths {
                    let text = values
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n");
                    for path in paths {
                        let Some((provider, name)) = path.split_once(':') else {
                            continue;
                        };
                        let name = name.trim_start_matches("\\");
                        let value = Value::String(text.as_str().into());
                        if provider.eq_ignore_ascii_case("env") {
                            environment::set(&mut self.variables, name, &value);
                        } else {
                            self.variables.insert(strip_scope(name).to_owned(), value);
                        }
                    }
                    return Ok(Some(Vec::new()));
                }
                if named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("asbytestream") && ops::is_true(value)
                }) {
                    let bytes: Vec<u8> = values
                        .iter()
                        .map(|value| convert_int32(value).map(|value| value as u8))
                        .collect::<Result<_, _>>()?;
                    for path in paths {
                        std::fs::write(&path, &bytes).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "Set-Content of an unwritable file",
                            }
                        })?;
                    }
                    return Ok(Some(Vec::new()));
                }
                for item in values {
                    text.push_str(&item.to_string());
                    if !no_newline {
                        text.push('\n');
                    }
                }
                for path in paths {
                    let files = if path.contains(['*', '?']) {
                        expand_glob(&path)
                    } else {
                        vec![path]
                    };
                    for path in files {
                        std::fs::write(&path, &text).map_err(|_| RuntimeError::NotImplemented {
                            feature: "Set-Content of an unwritable file",
                        })?;
                    }
                }
                Ok(Some(Vec::new()))
            }
            "remove-module" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let names: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if names.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Remove-Module without a name",
                    }
                    .into());
                }
                let mut targets = Vec::new();
                for name in names {
                    if name.contains(['*', '?']) {
                        targets.extend(
                            self.modules
                                .iter()
                                .map(LoadedModule::name)
                                .filter(|module| ops::wildcard_match(module, &name, false))
                                .map(str::to_owned),
                        );
                    } else {
                        targets.push(name);
                    }
                }
                for target in targets {
                    let lower = target.to_ascii_lowercase();
                    self.modules.retain(|entry| {
                        if entry.name().eq_ignore_ascii_case(&target) {
                            if let LoadedModule::Builtin(module) = entry {
                                self.aliases.unload(module);
                            }
                            false
                        } else {
                            true
                        }
                    });
                    if let Some(functions) = self.module_functions.remove(&lower) {
                        for function in functions {
                            self.functions.remove(&function);
                        }
                    }
                }
                Ok(Some(Vec::new()))
            }
            "import-module" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Import-Module without a name",
                    }
                    .into());
                }
                let mut files = Vec::new();
                for path in paths {
                    if path.contains(['*', '?']) {
                        let matches = expand_glob(&path);
                        if matches.len() != 1 {
                            return Err(RuntimeError::Script {
                                id: "Modules_InvalidModulePath",
                                category: format!(
                                    "InvalidArgument: ({path}:String) [Import-Module], ArgumentException"
                                ),
                                exception_type: "System.ArgumentException",
                                message: "The path cannot be processed because it resolved to more than one file; only one file at a time can be processed.".to_owned(),
                            }
                            .into());
                        }
                        files.push(matches[0].clone());
                    } else {
                        files.push(path);
                    }
                }
                for path in files {
                    if let Some(module) = MODULES
                        .iter()
                        .find(|module| path.eq_ignore_ascii_case(module.name))
                    {
                        if named.iter().any(|(key, _)| {
                            !matches!(key.to_ascii_lowercase().as_str(), "name" | "force")
                        }) {
                            return Err(RuntimeError::NotImplemented {
                                feature: "built-in module import options",
                            }
                            .into());
                        }
                        self.import_builtin(module);
                        continue;
                    }
                    let source =
                        std::fs::read_to_string(&path).map_err(|_| RuntimeError::Script {
                            id: "Modules_ModuleNotFound",
                            category: format!("ResourceUnavailable: ({path}:String) [Import-Module], FileNotFoundException"),
                            exception_type: "System.IO.FileNotFoundException",
                            message: format!("The specified module '{path}' was not loaded because no valid module file was found."),
                        })?;
                    let script = parse(&source)?;
                    let before: std::collections::HashSet<String> =
                        self.functions.keys().cloned().collect();
                    self.exec_block(&script.statements, &mut capture_sink())?;
                    let module_name = std::path::Path::new(&path)
                        .file_stem()
                        .map_or_else(|| path.clone(), |stem| stem.to_string_lossy().to_string());
                    let added: Vec<String> = self
                        .functions
                        .keys()
                        .filter(|name| !before.contains(*name))
                        .cloned()
                        .collect();
                    self.module_functions
                        .insert(module_name.to_ascii_lowercase(), added);
                    if !self
                        .modules
                        .iter()
                        .any(|module| module.name().eq_ignore_ascii_case(&module_name))
                    {
                        self.modules.push(LoadedModule::Script(module_name));
                    }
                }
                Ok(Some(Vec::new()))
            }
            "invoke-expression" => {
                let (positional, _) = self.evaluate_arguments(arguments)?;
                let Some(source) = positional.first() else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Invoke-Expression without a script",
                    }
                    .into());
                };
                let script = parse(&source.to_string())?;
                let saved = self.capture.take();
                self.capture = Some(Vec::new());
                let flow = self.exec_block(&script.statements, &mut capture_sink());
                let outputs = self.capture.take().unwrap_or_default();
                self.capture = saved;
                flow?;
                Ok(Some(outputs))
            }
            "get-variable" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let wanted: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                let value_only = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("valueonly") && ops::is_true(value)
                });
                let get = |name: &str| {
                    let key = strip_scope(name);
                    if key.eq_ignore_ascii_case("Error") {
                        let global = is_global_scope(name);
                        let value = self.variables.error_value(global);
                        if value_only {
                            return value;
                        }
                        let options = if global || !self.variables.contains_key("Error") {
                            "Constant"
                        } else {
                            "None"
                        };
                        return Value::object(vec![
                            ("Name".into(), Value::String("Error".into())),
                            ("Value".into(), value),
                            (
                                "Options".into(),
                                typed_scalar(
                                    "System.Management.Automation.ScopedItemOptions",
                                    options,
                                ),
                            ),
                            ("Visibility".into(), Value::String("Public".into())),
                            ("Description".into(), Value::String("".into())),
                        ]);
                    }
                    let value =
                        lookup(name, &self.variables, !self.last_failed).unwrap_or(Value::Null);
                    if value_only {
                        value
                    } else {
                        Value::object(vec![
                            ("Name".into(), Value::String(key.into())),
                            ("Value".into(), value),
                        ])
                    }
                };
                if wanted.is_empty() {
                    return Ok(Some(
                        self.variables
                            .visible_names()
                            .into_iter()
                            .map(get)
                            .collect(),
                    ));
                }
                let mut outputs = Vec::new();
                for name in wanted {
                    if name.contains(['*', '?']) {
                        outputs.extend(
                            self.variables
                                .visible_names()
                                .into_iter()
                                .filter(|key| ops::wildcard_match(key, &name, false))
                                .map(get),
                        );
                    } else {
                        outputs.push(get(&name));
                    }
                }
                Ok(Some(outputs))
            }
            "set-variable" | "new-variable" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let value_named = named
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("value"));
                let names: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None if value_named => positional.iter().map(ToString::to_string).collect(),
                    None => positional
                        .first()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_default(),
                };
                let value = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("value"))
                    .map(|(_, value)| value.clone())
                    .or_else(|| positional.get(1).cloned())
                    .unwrap_or(Value::Null);
                if names.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Set-Variable without a name",
                    }
                    .into());
                }
                let pass_thru = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("passthru") && ops::is_true(value)
                });
                let mut outputs = Vec::new();
                for name in names {
                    self.check_error_variable(&lower, &name, &named)?;

                    self.assign_variable(&name, value.clone()).map_err(|err| match err {
                        EngineError::Runtime(RuntimeError::Script { id: "RuntimeException", category, exception_type, message }) => {
                            let id = if lower == "set-variable" {
                                "RuntimeException,Microsoft.PowerShell.Commands.SetVariableCommand"
                            } else {
                                "RuntimeException,Microsoft.PowerShell.Commands.NewVariableCommand"
                            };
                            EngineError::Runtime(RuntimeError::Script { id, category, exception_type, message })
                        }
                        other => other,
                    })?;
                    if pass_thru {
                        let key = strip_scope(&name);
                        let key = encoding::variable_name(key).unwrap_or(key);
                        outputs.push(Value::object(vec![
                            ("Name".to_owned(), Value::String(key.into())),
                            ("Value".to_owned(), value.clone()),
                        ]));
                    }
                }
                Ok(Some(outputs))
            }
            "clear-variable" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let names: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if names.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Clear-Variable without a name",
                    }
                    .into());
                }
                for name in names {
                    self.check_error_variable(&lower, &name, &named)?;

                    if name.contains(['*', '?']) {
                        let keys: Vec<String> = self
                            .variables
                            .keys()
                            .filter(|key| ops::wildcard_match(key, &name, false))
                            .cloned()
                            .collect();
                        for key in keys {
                            self.variables.insert(key, Value::Null);
                        }
                    } else {
                        self.variables
                            .insert(strip_scope(&name).to_owned(), Value::Null);
                    }
                }
                Ok(Some(Vec::new()))
            }
            "remove-variable" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let names: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                for name in names {
                    self.check_error_variable(&lower, &name, &named)?;

                    if name.contains(['*', '?']) {
                        let keys: Vec<String> = self
                            .variables
                            .keys()
                            .filter(|key| ops::wildcard_match(key, &name, false))
                            .cloned()
                            .collect();
                        for key in keys {
                            self.variables.remove(&key);
                            self.globals.remove(&key);
                        }
                    } else {
                        let key = strip_scope(&name);
                        self.variables.remove(key);
                        self.globals.remove(key);
                    }
                }
                Ok(Some(Vec::new()))
            }
            "tee-object" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("filepath"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                if let Some(path) = path {
                    let mut text = String::new();
                    for item in input {
                        text.push_str(&item.to_string());
                        text.push('\n');
                    }
                    if named.iter().any(|(name, value)| {
                        name.eq_ignore_ascii_case("append") && ops::is_true(value)
                    }) {
                        use std::io::Write as _;
                        let mut file = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                            .map_err(|_| RuntimeError::NotImplemented {
                                feature: "Tee-Object cannot open this file",
                            })?;
                        file.write_all(text.as_bytes()).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "Tee-Object cannot write this file",
                            }
                        })?;
                    } else {
                        std::fs::write(&path, text).map_err(|_| RuntimeError::NotImplemented {
                            feature: "Tee-Object cannot write this file",
                        })?;
                    }
                }
                if let Some((_, variable)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("variable"))
                {
                    let variable = variable.to_string();
                    self.variables.insert(
                        strip_scope(&variable).to_owned(),
                        Value::array(input.to_vec()),
                    );
                }
                Ok(Some(input.to_vec()))
            }
            "convertto-csv" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let no_header = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("noheader") && ops::is_true(value)
                });
                let delimiter = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("delimiter"))
                    .map_or_else(|| ",".to_owned(), |(_, value)| value.to_string());
                let mut outputs: Vec<Value> = Vec::new();
                let mut columns: Vec<String> = Vec::new();
                for item in input {
                    if let Value::Object(entries) = item {
                        if columns.is_empty() {
                            columns = visible_entries(entries)
                                .iter()
                                .map(|(name, _)| (*name).clone())
                                .collect();
                            if !no_header {
                                outputs.push(Value::String(
                                    columns
                                        .iter()
                                        .map(|name| csv_quote(name))
                                        .collect::<Vec<_>>()
                                        .join(&delimiter)
                                        .into(),
                                ));
                            }
                        }
                        let cells: Vec<String> = columns
                            .iter()
                            .map(|column| {
                                entries
                                    .iter()
                                    .find(|(key, _)| {
                                        !is_hidden(key) && key.eq_ignore_ascii_case(column)
                                    })
                                    .map_or_else(String::new, |(_, value)| value.to_string())
                            })
                            .map(|value| csv_quote(&value))
                            .collect();
                        outputs.push(Value::String(cells.join(&delimiter).into()));
                    }
                }
                Ok(Some(outputs))
            }
            "export-csv" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                    })
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Some(path) = path else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Export-Csv without a path",
                    }
                    .into());
                };
                let no_header = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("noheader") && ops::is_true(value)
                });
                let delimiter = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("delimiter"))
                    .map_or_else(|| ",".to_owned(), |(_, value)| value.to_string());
                let mut lines: Vec<String> = Vec::new();
                let mut columns: Vec<String> = Vec::new();
                for item in input {
                    if let Value::Object(entries) = item {
                        if columns.is_empty() {
                            columns = visible_entries(entries)
                                .iter()
                                .map(|(name, _)| (*name).clone())
                                .collect();
                            if !no_header {
                                lines.push(
                                    columns
                                        .iter()
                                        .map(|name| csv_quote(name))
                                        .collect::<Vec<_>>()
                                        .join(&delimiter),
                                );
                            }
                        }
                        let cells: Vec<String> = columns
                            .iter()
                            .map(|column| {
                                entries
                                    .iter()
                                    .find(|(key, _)| {
                                        !is_hidden(key) && key.eq_ignore_ascii_case(column)
                                    })
                                    .map_or_else(String::new, |(_, value)| value.to_string())
                            })
                            .map(|value| csv_quote(&value))
                            .collect();
                        lines.push(cells.join(&delimiter));
                    }
                }
                let append = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("append") && ops::is_true(value)
                });
                if append {
                    let exists = std::fs::metadata(&path)
                        .map(|meta| meta.len() > 0)
                        .unwrap_or(false);
                    if exists && !no_header && !lines.is_empty() {
                        lines.remove(0);
                    }
                    use std::io::Write as _;
                    let mut file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .map_err(|_| RuntimeError::NotImplemented {
                            feature: "Export-Csv cannot open this file",
                        })?;
                    let mut text = lines.join("\n");
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    file.write_all(text.as_bytes())
                        .map_err(|_| RuntimeError::NotImplemented {
                            feature: "Export-Csv cannot write this file",
                        })?;
                    return Ok(Some(Vec::new()));
                }
                let mut text = lines.join("\n");
                if !text.is_empty() {
                    text.push('\n');
                }
                std::fs::write(&path, text).map_err(|_| RuntimeError::NotImplemented {
                    feature: "Export-Csv cannot write this file",
                })?;
                Ok(Some(Vec::new()))
            }
            "import-csv" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Import-Csv without a path",
                    }
                    .into());
                }
                let delimiter = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("delimiter"))
                    .map_or(',', |(_, value)| {
                        value.to_string().chars().next().unwrap_or(',')
                    });
                let custom_header: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("header"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut outputs = Vec::new();
                for path in paths {
                    let files = if path.contains(['*', '?']) {
                        expand_glob(&path)
                    } else {
                        vec![path]
                    };
                    for path in files {
                        let text = std::fs::read_to_string(&path).map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "Import-Csv of a missing file",
                            }
                        })?;
                        let mut lines = text.lines();
                        let columns = if custom_header.is_empty() {
                            let Some(header) = lines.next() else {
                                continue;
                            };
                            parse_csv_line_with(header, delimiter)
                        } else {
                            custom_header.clone()
                        };
                        for line in lines {
                            if line.trim().is_empty() {
                                continue;
                            }
                            let cells = parse_csv_line_with(line, delimiter);
                            let mut entries = Vec::new();
                            for (index, column) in columns.iter().enumerate() {
                                let value = cells.get(index).cloned().unwrap_or_default();
                                entries.push((column.clone(), Value::String(value.into())));
                            }
                            outputs.push(Value::object(entries));
                        }
                    }
                }
                Ok(Some(outputs))
            }
            "convertfrom-csv" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let mut lines: Vec<String> = Vec::new();
                for item in input {
                    if let Value::Array(items) = item {
                        lines.extend(items.iter().map(ToString::to_string));
                    } else {
                        lines.push(item.to_string());
                    }
                }
                let delimiter = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("delimiter"))
                    .map_or(',', |(_, value)| {
                        value.to_string().chars().next().unwrap_or(',')
                    });
                let custom_header = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("header"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    });
                let columns = if let Some(header) = custom_header {
                    header
                } else {
                    let Some(header) = lines.first() else {
                        return Ok(Some(Vec::new()));
                    };
                    let columns = parse_csv_line_with(header, delimiter);
                    lines.remove(0);
                    columns
                };
                let mut outputs = Vec::new();
                for line in &lines {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let cells = parse_csv_line_with(line, delimiter);
                    let mut entries = Vec::new();
                    for (index, column) in columns.iter().enumerate() {
                        let value = cells.get(index).cloned().unwrap_or_default();
                        entries.push((column.clone(), Value::String(value.into())));
                    }
                    outputs.push(Value::object(entries));
                }
                Ok(Some(outputs))
            }
            "get-member" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let member_type = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("membertype"))
                    .map(|(_, value)| value.to_string());
                let name_filter = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                    .map(|(_, value)| value.to_string());
                let mut outputs = Vec::new();
                for item in input {
                    let Value::Object(entries) = item else {
                        continue;
                    };
                    for (name, _) in visible_entries(entries) {
                        if let Some(filter) = &name_filter
                            && !ops::wildcard_match(name, filter, false)
                        {
                            continue;
                        }
                        if member_type
                            .as_deref()
                            .is_some_and(|wanted| !wanted.eq_ignore_ascii_case("NoteProperty"))
                        {
                            continue;
                        }
                        outputs.push(Value::object(vec![
                            ("Name".to_owned(), Value::String(name.clone().into())),
                            (
                                "MemberType".to_owned(),
                                Value::String("NoteProperty".into()),
                            ),
                        ]));
                    }
                    break;
                }
                Ok(Some(outputs))
            }
            "format-table" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let property = expand_property_value(
                    input,
                    named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                        .map(|(_, value)| value.clone()),
                );
                let hide_headers = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("hidetableheaders") && ops::is_true(value)
                });
                let auto_size = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("autosize") && ops::is_true(value)
                });
                Ok(Some(mark_format(
                    input,
                    "Table",
                    property,
                    hide_headers,
                    auto_size,
                )))
            }
            "format-list" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let property = expand_property_value(
                    input,
                    named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                        .map(|(_, value)| value.clone()),
                );
                Ok(Some(mark_format(input, "List", property, false, false)))
            }
            "compare-object" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let include_equal = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("includeequal") && ops::is_true(value)
                });
                let case_sensitive = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("casesensitive") && ops::is_true(value)
                });
                let same = |a: &str, b: &str| {
                    if case_sensitive {
                        a == b
                    } else {
                        a.eq_ignore_ascii_case(b)
                    }
                };
                let (Some(left), Some(right)) = (positional.first(), positional.get(1)) else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Compare-Object without two collections",
                    }
                    .into());
                };
                let left = flatten_value(left.clone());
                let right = flatten_value(right.clone());
                let properties: Vec<String> = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                    .map(|(_, value)| {
                        flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                if !properties.is_empty() {
                    let describe = |item: &Value| -> (String, Vec<(String, Value)>) {
                        let mut key = Vec::new();
                        let mut entries = Vec::new();
                        for property in &properties {
                            let value = member(item, property).unwrap_or(Value::Null);
                            key.push(if case_sensitive {
                                value.to_string()
                            } else {
                                value.to_string().to_lowercase()
                            });
                            entries.push((property.clone(), value));
                        }
                        (key.join("\u{0}"), entries)
                    };
                    let mut outputs = Vec::new();
                    let mut right_keys: Vec<String> = Vec::new();
                    for item in &right {
                        let (key, entries) = describe(item);
                        right_keys.push(key.clone());
                        let present = left.iter().any(|left| describe(left).0 == key);
                        if include_equal && present {
                            let mut object = entries;
                            object.push(("SideIndicator".to_owned(), Value::String("==".into())));
                            outputs.push(Value::object(object));
                        }
                    }
                    for item in &right {
                        let (key, entries) = describe(item);
                        if !left.iter().any(|left| describe(left).0 == key) {
                            let mut object = entries;
                            object.push(("SideIndicator".to_owned(), Value::String("=>".into())));
                            outputs.push(Value::object(object));
                        }
                    }
                    for item in &left {
                        let (key, entries) = describe(item);
                        if !right_keys.contains(&key) {
                            let mut object = entries;
                            object.push(("SideIndicator".to_owned(), Value::String("<=".into())));
                            outputs.push(Value::object(object));
                        }
                    }
                    return Ok(Some(outputs));
                }
                let mut outputs = Vec::new();
                if include_equal {
                    for item in &right {
                        let text = item.to_string();
                        if left.iter().any(|value| same(&value.to_string(), &text)) {
                            outputs.push(Value::object(vec![
                                ("InputObject".to_owned(), item.clone()),
                                ("SideIndicator".to_owned(), Value::String("==".into())),
                            ]));
                        }
                    }
                }
                for item in &right {
                    let text = item.to_string();
                    if !left.iter().any(|value| same(&value.to_string(), &text)) {
                        outputs.push(Value::object(vec![
                            ("InputObject".to_owned(), item.clone()),
                            ("SideIndicator".to_owned(), Value::String("=>".into())),
                        ]));
                    }
                }
                for item in &left {
                    let text = item.to_string();
                    if !right.iter().any(|value| same(&value.to_string(), &text)) {
                        outputs.push(Value::object(vec![
                            ("InputObject".to_owned(), item.clone()),
                            ("SideIndicator".to_owned(), Value::String("<=".into())),
                        ]));
                    }
                }
                Ok(Some(outputs))
            }
            "get-filehash" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let paths: Vec<String> = match named.iter().find(|(name, _)| {
                    name.eq_ignore_ascii_case("path") || name.eq_ignore_ascii_case("literalpath")
                }) {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                if paths.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Get-FileHash without a path",
                    }
                    .into());
                }
                let mut targets = Vec::new();
                for path in paths {
                    if path.contains(['*', '?']) {
                        targets.extend(expand_glob(&path));
                    } else {
                        targets.push(path);
                    }
                }
                let mut outputs = Vec::new();
                for path in targets {
                    let bytes = std::fs::read(&path).map_err(|_| RuntimeError::Script {
                        id: "FileNotFoundException",
                        category: format!(
                            "ObjectNotFound: ({path}:String) [Get-FileHash], FileNotFoundException"
                        ),
                        exception_type: "System.IO.FileNotFoundException",
                        message: format!("Cannot find path '{path}' because it does not exist."),
                    })?;
                    let algorithm = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("algorithm"))
                        .map_or_else(
                            || "SHA256".to_owned(),
                            |(_, value)| value.to_string().to_ascii_uppercase(),
                        );
                    let digest: Vec<u8> = match algorithm.as_str() {
                        "SHA256" => sha256(&bytes).to_vec(),
                        "SHA1" => sha1(&bytes).to_vec(),
                        "MD5" => md5(&bytes).to_vec(),
                        _ => {
                            return Err(RuntimeError::NotImplemented {
                                feature: "Get-FileHash with this algorithm",
                            }
                            .into());
                        }
                    };
                    let hash = digest
                        .iter()
                        .map(|byte| format!("{byte:02X}"))
                        .collect::<String>();
                    outputs.push(Value::object(vec![
                        ("Hash".to_owned(), Value::String(hash.into())),
                        ("Algorithm".to_owned(), Value::String(algorithm.into())),
                        ("Path".to_owned(), Value::String(path.into())),
                    ]));
                }
                Ok(Some(outputs))
            }
            "select-string" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let patterns: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("pattern"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional
                        .first()
                        .map(|value| vec![value.to_string()])
                        .unwrap_or_default(),
                };
                if patterns.is_empty() {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Select-String without a pattern",
                    }
                    .into());
                }
                let simple_match = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("simplematch") && ops::is_true(value)
                });
                let case_sensitive = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("casesensitive") && ops::is_true(value)
                });
                let regexes: Vec<rpsh_runtime::regex::Regex> = if simple_match {
                    Vec::new()
                } else {
                    patterns
                        .iter()
                        .map(|pattern| {
                            rpsh_runtime::regex::Regex::new(pattern, case_sensitive)
                                .map_err(|feature| RuntimeError::NotImplemented { feature })
                        })
                        .collect::<Result<_, _>>()?
                };
                let mut lines: Vec<String> = Vec::new();
                if input.is_empty() {
                    let paths: Vec<String> = match named.iter().find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                    }) {
                        Some((_, value)) => flatten_value(value.clone())
                            .iter()
                            .map(ToString::to_string)
                            .collect(),
                        None => positional
                            .get(1)
                            .map(|value| vec![value.to_string()])
                            .unwrap_or_default(),
                    };
                    for path in paths {
                        let files = if path.contains(['*', '?']) {
                            expand_glob(&path)
                        } else {
                            vec![path]
                        };
                        for path in files {
                            if let Ok(text) = std::fs::read_to_string(&path) {
                                lines.extend(text.lines().map(ToOwned::to_owned));
                            }
                        }
                    }
                } else {
                    for item in input {
                        if let Value::Array(items) = item {
                            lines.extend(items.iter().map(ToString::to_string));
                        } else {
                            lines.push(item.to_string());
                        }
                    }
                }
                let not_match = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("notmatch") && ops::is_true(value)
                });
                let mut outputs = Vec::new();
                for (index, line) in lines.iter().enumerate() {
                    let matched = if simple_match {
                        patterns.iter().any(|pattern| {
                            if case_sensitive {
                                line.contains(pattern)
                            } else {
                                line.to_lowercase().contains(&pattern.to_lowercase())
                            }
                        })
                    } else {
                        regexes.iter().any(|regex| regex.find(line).is_some())
                    };
                    if matched != not_match {
                        outputs.push(Value::object(vec![
                            ("Line".to_owned(), Value::String(line.clone().into())),
                            ("LineNumber".to_owned(), Value::Int32(index as i32 + 1)),
                        ]));
                    }
                }
                let quiet = named
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("quiet") && ops::is_true(value));
                if quiet {
                    return Ok(Some(if outputs.is_empty() {
                        Vec::new()
                    } else {
                        vec![Value::Bool(true)]
                    }));
                }
                Ok(Some(outputs))
            }
            "get-date" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let date = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("date"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Some(date) = date else {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Get-Date without -Date",
                    }
                    .into());
                };
                let (year, month, day, hour, minute, second) =
                    parse_datetime_text(&date).ok_or(RuntimeError::NotImplemented {
                        feature: "this Get-Date input",
                    })?;
                let format = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("format"))
                    .map(|(_, value)| value.to_string());
                match format {
                    Some(format) => Ok(Some(vec![Value::String(
                        format_datetime(year, month, day, hour, minute, second, &format).into(),
                    )])),
                    None => {
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
                        let (hour12, ampm) = if hour == 0 {
                            (12, "AM")
                        } else if hour < 12 {
                            (hour, "AM")
                        } else if hour == 12 {
                            (12, "PM")
                        } else {
                            (hour - 12, "PM")
                        };
                        let host_text = format!(
                            "{}, {} {}, {} {hour12}:{minute:02}:{second:02}\u{202F}{ampm}",
                            WEEKDAYS[weekday(year, month, day) as usize % 7],
                            MONTHS[(month as usize).clamp(1, 12) - 1],
                            day,
                            year
                        );
                        let mut value = datetime_value(year, month, day, hour, minute, second);
                        if let Value::Object(entries) = &value {
                            let mut entries = entries.as_ref().clone();
                            entries
                                .push(("__HostText".to_owned(), Value::String(host_text.into())));
                            value = Value::object(entries);
                        }
                        Ok(Some(vec![value]))
                    }
                }
            }
            "get-host" => Ok(Some(vec![
                automatic_variable("Host").unwrap_or(Value::Null),
            ])),
            "get-module" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let wanted: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                let mut outputs = Vec::new();
                for module in &self.modules {
                    let matches = wanted.is_empty()
                        || wanted
                            .iter()
                            .any(|wanted| ops::wildcard_match(module.name(), wanted, false));
                    if matches {
                        outputs.push(module.value());
                    }
                }
                Ok(Some(outputs))
            }
            "get-alias" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                Ok(Some(self.aliases.query(positional, &named)?))
            }
            "set-alias" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let name = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let value = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("value"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.get(1).map(ToString::to_string));
                if let (Some(name), Some(value)) = (name, value) {
                    let force = named.iter().any(|(key, value)| {
                        key.eq_ignore_ascii_case("force") && ops::is_true(value)
                    });
                    if named.iter().any(|(key, value)| {
                        key.eq_ignore_ascii_case("option")
                            && !value.to_string().eq_ignore_ascii_case("none")
                    }) {
                        return Err(RuntimeError::NotImplemented {
                            feature: "setting alias options",
                        }
                        .into());
                    }
                    if let Some(alias) = self.aliases.get(&name)
                        && (alias.options.all_scope() || (!force && alias.options.read_only()))
                    {
                        return Err(RuntimeError::NotImplemented {
                            feature: "changing a protected alias",
                        }
                        .into());
                    }
                    self.aliases.set(name, value);
                }
                Ok(Some(Vec::new()))
            }
            "get-command" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                if let Some((_, module)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("module"))
                {
                    let modules: Vec<String> = flatten_value(module.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect();
                    let modules = wildcards::Command::GetCommand.patterns(&modules)?;
                    let mut outputs = Vec::new();
                    for (name, functions) in &self.module_functions {
                        if !wildcards::matches_any(&modules, name)? {
                            continue;
                        }
                        for function in functions {
                            if let Some(definition) = self.functions.get(function) {
                                outputs.push(Value::object(vec![
                                    (
                                        "Name".to_owned(),
                                        Value::String(definition.name.clone().into()),
                                    ),
                                    ("CommandType".to_owned(), Value::String("Function".into())),
                                ]));
                            }
                        }
                    }
                    outputs.sort_by_key(|value| value.to_string());
                    return Ok(Some(outputs));
                }
                let wanted: Vec<String> = match named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                {
                    Some((_, value)) => flatten_value(value.clone())
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    None => positional.iter().map(ToString::to_string).collect(),
                };
                let command_type = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("commandtype"))
                    .map(|(_, value)| value.to_string());
                if command_type
                    .as_deref()
                    .is_none_or(|kind| kind.eq_ignore_ascii_case("Alias"))
                {
                    for name in &wanted {
                        if MODULES.iter().any(|module| module.exports_alias(name)) {
                            self.discover_alias(name);
                        }
                    }
                }
                let requested_names = wanted;
                let wanted = wildcards::Command::GetCommand.patterns(&requested_names)?;
                let word_patterns = |field: &str| {
                    let patterns = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(field))
                        .map(|(_, values)| {
                            flatten_value(values.clone())
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    wildcards::Command::GetCommand.patterns(&patterns)
                };
                let verb = word_patterns("verb")?;
                let noun = word_patterns("noun")?;
                let mut names: Vec<(String, &'static str)> = self
                    .functions
                    .values()
                    .map(|function| (function.name.clone(), "Function"))
                    .collect();
                for builtin in BUILTIN_COMMANDS {
                    names.push(((*builtin).to_owned(), "Cmdlet"));
                }
                names.extend(
                    self.aliases
                        .iter()
                        .map(|alias| (alias.name.to_owned(), "Alias")),
                );
                let imported_only = named.iter().any(|(key, value)| {
                    key.eq_ignore_ascii_case("ListImported") && ops::is_true(value)
                });
                if !imported_only && self.autoload_unqualified() {
                    for module in MODULES.iter().filter(|module| !self.builtin_loaded(module)) {
                        for &(name, _) in module.aliases {
                            if self.aliases.get(name).is_none() {
                                names.push((name.to_owned(), "Alias"));
                            }
                        }
                    }
                }
                names.sort_by(|left, right| left.0.cmp(&right.0));
                let mut outputs = Vec::new();
                for (name, kind) in names {
                    let matches = wanted.is_empty() || wildcards::matches_any(&wanted, &name)?;
                    let type_matches = command_type
                        .as_deref()
                        .is_none_or(|wanted| wanted.eq_ignore_ascii_case(kind));
                    let (name_verb, name_noun) = match name.split_once('-') {
                        Some((verb, noun)) => (verb, noun),
                        None => ("", name.as_str()),
                    };
                    let verb_matches = verb.is_empty() || wildcards::matches_any(&verb, name_verb)?;
                    let noun_matches = noun.is_empty() || wildcards::matches_any(&noun, name_noun)?;
                    if matches && type_matches && verb_matches && noun_matches {
                        if kind == "Alias" {
                            if let Some(alias) = self.aliases.get(&name) {
                                outputs.push(alias.value());
                            } else if let Some(module) =
                                MODULES.iter().find(|module| module.exports_alias(&name))
                            {
                                outputs.push(aliases::available_export(&name, module));
                            }
                        } else {
                            outputs.push(Value::object(vec![
                                ("Name".to_owned(), Value::String(name.into())),
                                ("CommandType".to_owned(), Value::String(kind.into())),
                            ]));
                        }
                    }
                }
                if !requested_names.is_empty()
                    && verb.is_empty()
                    && noun.is_empty()
                    && command_type
                        .as_deref()
                        .is_none_or(|kind| kind.eq_ignore_ascii_case("Application"))
                {
                    let all = command_type.is_some()
                        || named.iter().any(|(key, value)| {
                            key.eq_ignore_ascii_case("All") && ops::is_true(value)
                        });
                    for name in requested_names {
                        let already_found = !all
                            && outputs.iter().any(|entry| {
                                member(entry, "Name").is_ok_and(|value| {
                                    value.to_string().eq_ignore_ascii_case(&name)
                                })
                            });
                        if already_found
                            || rpsh_runtime::wildcard::contains_wildcard_characters(
                                name.encode_utf16(),
                            )
                        {
                            continue;
                        }
                        outputs.extend(
                            self.application_paths(&name, all)
                                .into_iter()
                                .filter(|path| {
                                    !path
                                        .extension()
                                        .is_some_and(|ext| ext.eq_ignore_ascii_case("ps1"))
                                })
                                .map(native::application_info),
                        );
                    }
                }
                Ok(Some(outputs))
            }
            "get-unique" => {
                let mut outputs = Vec::new();
                let mut previous: Option<String> = None;
                for item in input {
                    let key = item.to_string();
                    if previous.as_deref() != Some(key.as_str()) {
                        outputs.push(item.clone());
                        previous = Some(key);
                    }
                }
                Ok(Some(outputs))
            }
            "get-psdrive" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let wanted = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let drives = [
                    ("/", "FileSystem", "FileSystem"),
                    ("Alias", "Alias", "Alias"),
                    ("Env", "Environment", "Environment"),
                    ("Function", "Function", "Function"),
                    ("Temp", "/tmp", "FileSystem"),
                    ("Variable", "Variable", "Variable"),
                ];
                let mut outputs = Vec::new();
                for (name, root, provider) in drives {
                    if let Some(wanted) = &wanted
                        && !ops::wildcard_match(name, wanted, false)
                    {
                        continue;
                    }
                    outputs.push(Value::object(vec![
                        ("Name".to_owned(), Value::String(name.into())),
                        ("Root".to_owned(), Value::String(root.into())),
                        (
                            "Provider".to_owned(),
                            Value::object(vec![(
                                "Name".to_owned(),
                                Value::String(provider.into()),
                            )]),
                        ),
                    ]));
                }
                Ok(Some(outputs))
            }
            "get-psprovider" => {
                let providers = [
                    ("Alias", "Alias"),
                    ("Environment", "Env"),
                    ("FileSystem", "FileSystem"),
                    ("Function", "Function"),
                    ("Variable", "Variable"),
                ];
                Ok(Some(
                    providers
                        .into_iter()
                        .map(|(name, drive)| {
                            Value::object(vec![
                                ("Name".to_owned(), Value::String(name.into())),
                                ("Drives".to_owned(), Value::String(drive.into())),
                            ])
                        })
                        .collect(),
                ))
            }
            "get-location" => Ok(Some(vec![Value::object(vec![(
                "Path".to_owned(),
                Value::String(
                    std::env::current_dir()
                        .map_or_else(|_| String::new(), |path| path.display().to_string())
                        .into(),
                ),
            )])])),
            "set-location" => {
                self.command_paths.remember_process_directory();
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                    })
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                if let Some(path) = path
                    && std::env::set_current_dir(&path).is_err()
                {
                    return Err(RuntimeError::Script {
                        id: "PathNotFound,Microsoft.PowerShell.Commands.SetLocationCommand",
                        category: format!(
                            "ObjectNotFound: ({path}:String) [Set-Location], ItemNotFoundException"
                        ),
                        exception_type: "System.Management.Automation.ItemNotFoundException",
                        message: format!("Cannot find path '{path}' because it does not exist."),
                    }
                    .into());
                }
                Ok(Some(Vec::new()))
            }
            "push-location" => {
                self.command_paths.remember_process_directory();
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let path = named
                    .iter()
                    .find(|(name, _)| {
                        name.eq_ignore_ascii_case("path")
                            || name.eq_ignore_ascii_case("literalpath")
                    })
                    .map(|(_, value)| value.to_string())
                    .or_else(|| positional.first().map(ToString::to_string));
                let Ok(current) = std::env::current_dir() else {
                    return Ok(Some(Vec::new()));
                };
                self.locations.push(current);
                if let Some(path) = path {
                    let _ = std::env::set_current_dir(path);
                }
                Ok(Some(Vec::new()))
            }
            "pop-location" => {
                if let Some(path) = self.locations.pop() {
                    let _ = std::env::set_current_dir(path);
                }
                Ok(Some(Vec::new()))
            }
            "new-timespan" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let start = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("start"))
                    .map(|(_, value)| value.clone());
                let end = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("end"))
                    .map(|(_, value)| value.clone());
                if start.is_some() || end.is_some() {
                    let fields = |value: &Value| -> Option<[i32; 6]> {
                        datetime_fields(value).or_else(|| {
                            parse_datetime_text(&value.to_string())
                                .map(|parts| [parts.0, parts.1, parts.2, parts.3, parts.4, parts.5])
                        })
                    };
                    let (Some(start), Some(end)) = (start.as_ref(), end.as_ref()) else {
                        return Err(RuntimeError::NotImplemented {
                            feature: "New-TimeSpan with only one endpoint",
                        }
                        .into());
                    };
                    let (Some(start), Some(end)) = (fields(start), fields(end)) else {
                        return Err(RuntimeError::NotImplemented {
                            feature: "New-TimeSpan with this endpoint",
                        }
                        .into());
                    };
                    return Ok(Some(vec![timespan_value(
                        datetime_ticks(end) - datetime_ticks(start),
                    )]));
                }
                let amount = |wanted: &str| -> Result<f64, RuntimeError> {
                    named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
                        .map_or(Ok(0.0), |(_, value)| convert_double(value))
                };
                let ticks = (amount("days")? * TICKS_PER_DAY as f64
                    + amount("hours")? * TICKS_PER_HOUR as f64
                    + amount("minutes")? * TICKS_PER_MINUTE as f64
                    + amount("seconds")? * TICKS_PER_SECOND as f64
                    + amount("milliseconds")? * TICKS_PER_MILLISECOND as f64)
                    .round() as i64;
                Ok(Some(vec![timespan_value(ticks)]))
            }
            "start-sleep" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let seconds = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("seconds"))
                    .map(|(_, value)| convert_double(value))
                    .transpose()?
                    .or_else(|| {
                        named
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case("milliseconds"))
                            .map(|(_, value)| convert_double(value).map(|ms| ms / 1000.0))
                            .transpose()
                            .ok()
                            .flatten()
                    })
                    .or_else(|| {
                        positional
                            .first()
                            .map(convert_double)
                            .transpose()
                            .ok()
                            .flatten()
                    })
                    .unwrap_or(0.0);
                if seconds > 0.0 {
                    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
                }
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
                    let minimum = named
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("minimum"))
                        .map_or(Ok(0), |(_, value)| convert_int32(value))?;
                    let value = if minimum == 0 {
                        generator.next_max(maximum)
                    } else {
                        generator.next_range(minimum, maximum)
                    };
                    return Ok(Some(vec![Value::Int32(value)]));
                }
                let input = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("inputobject"))
                    .map(|(_, value)| value.clone())
                    .or_else(|| positional.first().cloned())
                    .or_else(|| {
                        if input.is_empty() {
                            None
                        } else {
                            Some(Value::array(input.to_vec()))
                        }
                    });
                if let Some(input) = input {
                    let items = flatten_value(input);
                    if named.iter().any(|(name, value)| {
                        name.eq_ignore_ascii_case("shuffle") && ops::is_true(value)
                    }) {
                        let mut items = items;
                        let length = items.len();
                        let mut outputs = Vec::with_capacity(length);
                        for index in 0..length {
                            let pick = generator.next_range(index as i32, length as i32) as usize;
                            outputs.push(items[pick].clone());
                            if index != pick {
                                items[pick] = items[index].clone();
                            }
                        }
                        return Ok(Some(outputs));
                    }
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
                let (_, named) = self.evaluate_arguments(arguments)?;
                let width = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("width"))
                    .map_or(Ok(i32::MAX), |(_, value)| convert_int32(value))?;
                if width < 2 {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Out-String Width below two columns",
                    }
                    .into());
                }
                let text = format_out_string(input, width as usize);
                let stream = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("stream") && ops::is_true(value)
                });
                if stream {
                    let mut lines: Vec<String> = text.split('\n').map(ToOwned::to_owned).collect();
                    if text.ends_with('\n') {
                        lines.pop();
                    }
                    return Ok(Some(
                        lines
                            .into_iter()
                            .map(|line| Value::String(line.into()))
                            .collect(),
                    ));
                }
                Ok(Some(vec![Value::String(text.into())]))
            }
            "convertto-json" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let compress = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("compress") && ops::is_true(value)
                });
                let values = if input.is_empty() {
                    positional
                } else {
                    input.to_vec()
                };
                let as_array = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("asarray") && ops::is_true(value)
                });
                if as_array {
                    let array = Value::array(values);
                    let text = if compress {
                        json_stringify(&array)?
                    } else {
                        json_stringify_pretty(&array, 0)?
                    };
                    return Ok(Some(vec![Value::String(text.into())]));
                }
                let mut outputs = Vec::new();
                for value in values {
                    let text = if compress {
                        json_stringify(&value)?
                    } else {
                        json_stringify_pretty(&value, 0)?
                    };
                    outputs.push(Value::String(text.into()));
                }
                Ok(Some(outputs))
            }
            "convertfrom-json" => {
                let (positional, named) = self.evaluate_arguments(arguments)?;
                let as_hashtable = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("ashashtable") && ops::is_true(value)
                });
                let values: Vec<Value> = if input.is_empty() {
                    positional
                        .iter()
                        .flat_map(|value| flatten_value(value.clone()))
                        .collect()
                } else {
                    input.to_vec()
                };
                let mut outputs = Vec::new();
                for item in values {
                    let value = json_parse(&item.to_string())?;
                    outputs.push(if as_hashtable {
                        mark_json_hashtables(value)
                    } else {
                        value
                    });
                }
                Ok(Some(outputs))
            }
            "select-object" => {
                let (_, named) = self.evaluate_arguments(arguments)?;
                let mut items = input.to_vec();
                let unique = named.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case("unique") && ops::is_true(value)
                });
                let property = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("property"))
                    .map(|(_, value)| value.clone());
                let expand = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("expandproperty"))
                    .map(|(_, value)| value.clone());
                if let Some(property) = property.or(expand.clone()) {
                    let mut names = flatten_value(property);
                    if expand.is_none()
                        && names.iter().any(|name| {
                            let text = name.to_string();
                            text != "*" && text.contains(['*', '?'])
                        })
                        && let Some(Value::Object(entries)) = items.first()
                    {
                        let first_names = visible_entries(entries);
                        let mut expanded = Vec::new();
                        for name in names {
                            let text = name.to_string();
                            if text != "*" && text.contains(['*', '?']) {
                                for (candidate, _) in &first_names {
                                    if ops::wildcard_match(candidate, &text, false) {
                                        expanded.push(Value::String((*candidate).clone().into()));
                                    }
                                }
                            } else {
                                expanded.push(name);
                            }
                        }
                        names = expanded;
                    }
                    items = items
                        .into_iter()
                        .filter_map(|item| {
                            if expand.is_some() {
                                names.first().and_then(|name| {
                                    member(&item, &name.to_string())
                                        .ok()
                                        .filter(|value| !value.is_null_like())
                                })
                            } else if names.iter().any(|name| name.to_string() == "*") {
                                match &item {
                                    Value::Object(entries) => Some(Value::object(
                                        visible_entries(entries)
                                            .into_iter()
                                            .map(|(name, value)| (name.clone(), value.clone()))
                                            .collect(),
                                    )),
                                    _ => Some(item),
                                }
                            } else {
                                let mut entries = Vec::new();
                                for name in &names {
                                    let key = name.to_string();
                                    if let Ok(value) = member(&item, &key) {
                                        entries.push((key, value));
                                    }
                                }
                                Some(Value::object(entries))
                            }
                        })
                        .collect();
                }
                if unique {
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
                if let Some((_, last)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("last"))
                {
                    let last = convert_int32(last)?.max(0) as usize;
                    if items.len() > last {
                        items = items.split_off(items.len() - last);
                    }
                }
                if let Some((_, index)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("index"))
                {
                    let indices = flatten_value(index.clone());
                    items = indices
                        .iter()
                        .filter_map(|value| convert_int32(value).ok())
                        .filter_map(|index| {
                            let index = usize::try_from(index).ok()?;
                            items.get(index).cloned()
                        })
                        .collect();
                }
                if let Some((_, skip_last)) = named
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("skiplast"))
                {
                    let skip_last = convert_int32(skip_last)?.max(0) as usize;
                    if skip_last >= items.len() {
                        items.clear();
                    } else {
                        items.truncate(items.len() - skip_last);
                    }
                }
                Ok(Some(items))
            }
            _ if module.is_some() => Err(RuntimeError::NotImplemented {
                feature: "invoking an unimplemented built-in module cmdlet",
            }
            .into()),
            _ => Ok(None),
        }
    }

    fn provider_children(
        &self,
        path: &str,
        names_only: bool,
        filter: Option<&str>,
    ) -> Option<Vec<Value>> {
        let (provider, name) = path.split_once(':')?;
        let pattern = if name.is_empty() { "*" } else { name };
        let matches = |candidate: &str| {
            ops::wildcard_match(candidate, pattern, false)
                && filter.is_none_or(|filter| ops::wildcard_match(candidate, filter, false))
        };
        let mut outputs = Vec::new();
        match provider.to_ascii_lowercase().as_str() {
            "env" => {
                let entries = environment::entries(&self.variables);
                for (name, value) in entries {
                    if !matches(&name) {
                        continue;
                    }
                    outputs.push(if names_only {
                        Value::String(name.into())
                    } else {
                        Value::object(vec![
                            ("Name".to_owned(), Value::String(name.into())),
                            ("Value".to_owned(), value),
                        ])
                    });
                }
            }
            "variable" => {
                for name in self.variables.visible_names() {
                    if !matches(name) {
                        continue;
                    }
                    outputs.push(if names_only {
                        Value::String(name.into())
                    } else {
                        Value::object(vec![
                            ("Name".into(), Value::String(name.into())),
                            (
                                "Value".into(),
                                lookup(name, &self.variables, !self.last_failed)
                                    .unwrap_or(Value::Null),
                            ),
                        ])
                    });
                }
            }
            "alias" => {
                let mut entries: Vec<_> = self
                    .aliases
                    .iter()
                    .filter(|alias| matches(alias.name))
                    .collect();
                entries.sort_by_cached_key(aliases::name_sort_key);
                outputs.extend(entries.into_iter().map(|alias| {
                    if names_only {
                        Value::String(alias.name.into())
                    } else {
                        alias.value()
                    }
                }));
            }
            "function" => {
                let mut functions: Vec<&FunctionDef> = self.functions.values().collect();
                functions.sort_by_key(|function| function.name.to_lowercase());
                for function in functions {
                    if !matches(&function.name) {
                        continue;
                    }
                    outputs.push(if names_only {
                        Value::String(function.name.clone().into())
                    } else {
                        Value::object(vec![(
                            "Name".to_owned(),
                            Value::String(function.name.clone().into()),
                        )])
                    });
                }
            }
            _ => return None,
        }
        Some(outputs)
    }

    fn provider_item(&self, path: &str) -> Option<Result<Value, RuntimeError>> {
        let (provider, name) = path.split_once(':')?;
        let name = name.trim_start_matches("\\");
        let missing = |path: &str| {
            Err(RuntimeError::Script {
                id: "PathNotFound,Microsoft.PowerShell.Commands.GetItemCommand",
                category: format!(
                    "ObjectNotFound: ({path}:String) [Get-Item], ItemNotFoundException"
                ),
                exception_type: "System.Management.Automation.ItemNotFoundException",
                message: format!("Cannot find path '{path}' because it does not exist."),
            })
        };
        match provider.to_ascii_lowercase().as_str() {
            "env" => Some(match environment::get(&self.variables, name) {
                Some(value) => Ok(Value::object(vec![
                    ("Name".to_owned(), Value::String(name.into())),
                    ("Value".to_owned(), value),
                ])),
                None => missing(path),
            }),
            "variable" => {
                let key = strip_scope(name).to_owned();
                Some(
                    match self
                        .variables
                        .get(&key)
                        .cloned()
                        .or_else(|| automatic_variable(&key))
                    {
                        Some(value) => Ok(Value::object(vec![
                            ("Name".to_owned(), Value::String(key.into())),
                            ("Value".to_owned(), value),
                        ])),
                        None => missing(path),
                    },
                )
            }
            "alias" => Some(match self.aliases.get(name) {
                Some(alias) => Ok(alias.value()),
                None => missing(path),
            }),
            "function" => Some(match self.functions.get(&name.to_ascii_lowercase()) {
                Some(function) => Ok(Value::object(vec![(
                    "Name".to_owned(),
                    Value::String(function.name.clone().into()),
                )])),
                None => missing(path),
            }),
            _ => None,
        }
    }

    fn provider_path_exists(&self, path: &str) -> Option<bool> {
        let (provider, name) = path.split_once(':')?;
        let name = name.trim_start_matches("\\");
        if name.is_empty()
            && matches!(
                provider.to_ascii_lowercase().as_str(),
                "env" | "variable" | "function" | "alias"
            )
        {
            return Some(true);
        }
        let wildcard = name.contains(['*', '?']);
        match provider.to_ascii_lowercase().as_str() {
            "env" => Some(if wildcard {
                environment::entries(&self.variables)
                    .keys()
                    .any(|candidate| ops::wildcard_match(candidate, name, false))
            } else {
                environment::get(&self.variables, name).is_some()
            }),
            "variable" => {
                if wildcard {
                    return Some(
                        self.variables
                            .keys()
                            .any(|candidate| ops::wildcard_match(candidate, name, false)),
                    );
                }
                let key = strip_scope(name);
                Some(
                    self.variables.contains_key(key)
                        || self.globals.contains_key(key)
                        || automatic_variable(key).is_some(),
                )
            }
            "function" => Some(if wildcard {
                self.functions
                    .values()
                    .any(|function| ops::wildcard_match(&function.name, name, false))
            } else {
                self.functions.contains_key(&name.to_ascii_lowercase())
            }),
            "alias" => Some(if wildcard {
                self.aliases
                    .iter()
                    .any(|alias| ops::wildcard_match(alias.name, name, false))
            } else {
                self.aliases.get(name).is_some()
            }),
            _ => None,
        }
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

    fn eval_pipeline_expression(&mut self, expr: &Expr) -> Result<Value, EngineError> {
        let value = self.eval_expression(expr)?;
        let mut root = expr;
        while let ExprKind::Paren(inner) | ExprKind::Subexpression(inner) = &root.kind {
            root = inner;
        }
        if !matches!(root.kind, ExprKind::Group(_)) {
            self.last_failed = false;
        }
        Ok(value)
    }

    fn eval_expression(&mut self, expr: &Expr) -> Result<Value, EngineError> {
        if let ExprKind::TypeLiteral(name) = &expr.kind
            && self.classes.contains_key(&name.to_ascii_lowercase())
        {
            return Ok(Value::Type(name.clone().into()));
        }
        if let ExprKind::InvokeStatic {
            type_name,
            name,
            arguments,
        } = &expr.kind
            && name.eq_ignore_ascii_case("new")
            && self.classes.contains_key(&type_name.to_ascii_lowercase())
        {
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(self.eval_expression(argument)?);
            }
            return self.construct_class(type_name, &values);
        }
        if let ExprKind::InvokeMember {
            target,
            name,
            arguments,
        } = &expr.kind
        {
            let target = self.eval_expression(target)?;
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(self.eval_expression(argument)?);
            }
            if name.eq_ignore_ascii_case("GetType") && values.is_empty() {
                return Ok(get_type(&target));
            }
            if let Value::Object(entries) = &target
                && let Some(class) = object_class(entries)
                && self.classes.contains_key(&class.to_ascii_lowercase())
            {
                return self.call_method(&class, name, &target, &values);
            }
            return Ok(invoke_member(&target, name, &values)?);
        }
        if let ExprKind::Group(statements) = &expr.kind {
            return self.eval_group(statements);
        }
        if let ExprKind::ExpandableString(parts) | ExprKind::ExpandableBareword(parts) = &expr.kind
        {
            let mut text = String::new();
            for part in parts {
                match part {
                    rpsh_syntax::StringPart::Literal(value) => text.push_str(value),
                    rpsh_syntax::StringPart::Variable(name) => {
                        if let Some(value) = lookup(name, &self.variables, !self.last_failed) {
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
        if let ExprKind::Member { target, name } = &expr.kind {
            let target = self.eval_expression(target)?;
            return Ok(member(&target, name)?);
        }
        if let ExprKind::Index { target, index } = &expr.kind {
            let target = self.eval_expression(target)?;
            let index = self.eval_expression(index)?;
            return Ok(index_value(&target, &index)?);
        }
        if let ExprKind::Binary { op, left, right } = &expr.kind
            && (contains_group(left) || contains_group(right))
        {
            let left = self.eval_expression(left)?;
            let right = self.eval_expression(right)?;
            let result = match op {
                BinaryOp::Join => ops::join(&left, &right)?,
                BinaryOp::Add => add_values(&left, &right)?,
                BinaryOp::Subtract => subtract_values(&left, &right)?,
                BinaryOp::Multiply => ops::multiply(&left, &right)?,
                BinaryOp::Divide => ops::divide(&left, &right)?,
                BinaryOp::Remainder => ops::remainder(&left, &right)?,
                BinaryOp::Eq => compare_values(&left, &right, ops::Comparison::Eq, false)?,
                BinaryOp::Ne => compare_values(&left, &right, ops::Comparison::Ne, false)?,
                BinaryOp::Gt => compare_values(&left, &right, ops::Comparison::Gt, false)?,
                BinaryOp::Ge => compare_values(&left, &right, ops::Comparison::Ge, false)?,
                BinaryOp::Lt => compare_values(&left, &right, ops::Comparison::Lt, false)?,
                BinaryOp::Le => compare_values(&left, &right, ops::Comparison::Le, false)?,
                BinaryOp::CEq => compare_values(&left, &right, ops::Comparison::Eq, true)?,
                BinaryOp::CNe => compare_values(&left, &right, ops::Comparison::Ne, true)?,
                BinaryOp::CGt => compare_values(&left, &right, ops::Comparison::Gt, true)?,
                BinaryOp::CGe => compare_values(&left, &right, ops::Comparison::Ge, true)?,
                BinaryOp::CLt => compare_values(&left, &right, ops::Comparison::Lt, true)?,
                BinaryOp::CLe => compare_values(&left, &right, ops::Comparison::Le, true)?,
                BinaryOp::Like => compare_values(&left, &right, ops::Comparison::Like, false)?,
                BinaryOp::NotLike => {
                    compare_values(&left, &right, ops::Comparison::NotLike, false)?
                }
                BinaryOp::Contains => ops::contains(&left, &right, false)?,
                BinaryOp::NotContains => ops::contains(&left, &right, false)
                    .map(|value| Value::Bool(!matches!(value, Value::Bool(true))))?,
                BinaryOp::CContains => ops::contains(&left, &right, true)?,
                BinaryOp::In => ops::contains(&right, &left, false)?,
                BinaryOp::NotIn => ops::contains(&right, &left, false)
                    .map(|value| Value::Bool(!matches!(value, Value::Bool(true))))?,
                BinaryOp::CIn => ops::contains(&right, &left, true)?,
                _ => {
                    return Err(RuntimeError::NotImplemented {
                        feature: "this operator on a group expression",
                    }
                    .into());
                }
            };
            return Ok(result);
        }
        Ok(evaluate(expr, &mut self.variables, !self.last_failed)?)
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
                CommandArgument::Parameter { name, value, .. } => {
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
        local: &mut Variables,
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
                evaluate(default, local, !self.last_failed)?
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
                let value = self.eval_expression(condition)?;
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

    fn write_host(&mut self, out: &mut dyn Write, text: fmt::Arguments<'_>) -> io::Result<()> {
        let encoding = self.host_encoding.get_or_insert_with(encoding::console);
        encoding::write_host(*encoding, out, text)
    }

    fn flush_host(&mut self, out: &mut dyn Write) -> Result<(), EngineError> {
        if self.host_objects.is_empty() {
            return Ok(());
        }
        let text = format_host_stream(&self.host_objects);
        self.host_objects.clear();
        self.write_host(out, format_args!("{text}"))?;
        Ok(())
    }

    fn write_value(&mut self, value: Value, out: &mut dyn Write) -> Result<(), EngineError> {
        if let Value::ArrayList(list) = value {
            return self.write_value(Value::array(list.items().to_vec()), out);
        }
        if let Some(capture) = &mut self.capture {
            match value {
                Value::Array(items) => capture.extend(items.iter().cloned()),
                Value::Null | Value::AutomationNull => {}
                value => capture.push(value),
            }
            return Ok(());
        }
        if self.host_objects.first().is_some_and(uses_command_table) {
            self.host_objects.extend(flatten_value(value));
            return Ok(());
        }
        if let Value::Object(entries) = &value {
            if let Some(prefix) = record_prefix(&value) {
                self.flush_host(out)?;
                self.write_host(out, format_args!("{prefix}{value}\n"))?;
                return Ok(());
            }
            let host_text = entries
                .iter()
                .find(|(key, _)| key == "__HostText")
                .and_then(|(_, value)| match value {
                    Value::String(text) => Some(text.clone()),
                    _ => None,
                });
            if let Some(text) = host_text {
                self.flush_host(out)?;
                self.write_host(out, format_args!("\n{text}\n"))?;
                self.host_text = true;
                return Ok(());
            }
            if datetime_fields(&value).is_some() || is_command_info(&value) {
                self.host_objects.push(value);
                return Ok(());
            }
            let text = entries
                .iter()
                .find(|(key, _)| key == "__Text")
                .and_then(|(_, value)| match value {
                    Value::String(text) => Some(text.clone()),
                    _ => None,
                });
            if let Some(text) = text {
                self.flush_host(out)?;
                self.write_host(out, format_args!("{text}\n"))?;
                return Ok(());
            }
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
                self.flush_host(out)?;
                for item in items.iter() {
                    if !item.is_null_like() {
                        self.write_host(out, format_args!("{item}\n"))?;
                    }
                }
            }
            Value::Null | Value::AutomationNull => {}
            value => {
                self.flush_host(out)?;
                self.write_host(out, format_args!("{value}\n"))?;
            }
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
    Exit(i32),
}

fn evaluate(
    expr: &Expr,
    variables: &mut Variables,
    last_success: bool,
) -> Result<Value, RuntimeError> {
    match &expr.kind {
        ExprKind::Int32(value) => Ok(Value::Int32(*value)),
        ExprKind::Number(value) => Ok(number_value(value)),
        ExprKind::String(value) | ExprKind::Bareword(value) => {
            Ok(Value::String(value.clone().into()))
        }
        ExprKind::Paren(inner) => evaluate(inner, variables, last_success),
        // An undefined variable reads as `$null` outside strict mode.
        ExprKind::Variable(name) => {
            Ok(lookup(name, variables, last_success).unwrap_or(Value::Null))
        }
        ExprKind::ExpandableString(parts) | ExprKind::ExpandableBareword(parts) => {
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
        ExprKind::TypeLiteral(name) => Ok(Value::Type(type_full_name(name).into())),
        ExprKind::Coalesce { left, right } => {
            let value = evaluate(left, variables, last_success)?;
            if matches!(value, Value::Null) {
                evaluate(right, variables, last_success)
            } else {
                Ok(value)
            }
        }
        ExprKind::ScriptBlock { source, .. } => Ok(Value::ScriptBlock(source.as_str().into())),
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
        ExprKind::StaticMember { type_name, name } => static_member(type_name, name, variables),
        ExprKind::InvokeStatic {
            type_name,
            name,
            arguments,
        } => {
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(evaluate(argument, variables, last_success)?);
            }
            invoke_static(type_name, name, &values, variables)
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
        } => negate_value(&evaluate(operand, variables, last_success)?),
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
                BinaryOp::Add => add_values(&left, &right),
                BinaryOp::Subtract => subtract_values(&left, &right),
                BinaryOp::Multiply => ops::multiply(&left, &right),
                BinaryOp::Divide => ops::divide(&left, &right),
                BinaryOp::Remainder => ops::remainder(&left, &right),
                BinaryOp::Range => ops::range(&left, &right),
                BinaryOp::Join => ops::join(&left, &right),
                BinaryOp::Format => ops::format(&left, &right),
                BinaryOp::Eq => compare_values(&left, &right, ops::Comparison::Eq, false),
                BinaryOp::Ne => compare_values(&left, &right, ops::Comparison::Ne, false),
                BinaryOp::Ge => compare_values(&left, &right, ops::Comparison::Ge, false),
                BinaryOp::Gt => compare_values(&left, &right, ops::Comparison::Gt, false),
                BinaryOp::Lt => compare_values(&left, &right, ops::Comparison::Lt, false),
                BinaryOp::Le => compare_values(&left, &right, ops::Comparison::Le, false),
                BinaryOp::CEq => compare_values(&left, &right, ops::Comparison::Eq, true),
                BinaryOp::CNe => compare_values(&left, &right, ops::Comparison::Ne, true),
                BinaryOp::CGe => compare_values(&left, &right, ops::Comparison::Ge, true),
                BinaryOp::CGt => compare_values(&left, &right, ops::Comparison::Gt, true),
                BinaryOp::CLt => compare_values(&left, &right, ops::Comparison::Lt, true),
                BinaryOp::CLe => compare_values(&left, &right, ops::Comparison::Le, true),
                BinaryOp::Like => compare_values(&left, &right, ops::Comparison::Like, false),
                BinaryOp::NotLike => compare_values(&left, &right, ops::Comparison::NotLike, false),
                BinaryOp::CLike => compare_values(&left, &right, ops::Comparison::Like, true),
                BinaryOp::CNotLike => compare_values(&left, &right, ops::Comparison::NotLike, true),
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
                BinaryOp::Band => bitwise_values(&left, &right, "band"),
                BinaryOp::Bor => bitwise_values(&left, &right, "bor"),
                BinaryOp::Bxor => bitwise_values(&left, &right, "bxor"),
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
    variables: &mut Variables,
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
    if let Some(items) = target.sequence() {
        let at =
            |index: &Value| -> Result<Value, RuntimeError> {
                let index = convert_int32(index)?;
                Ok(array_list::position(index, items.len())
                    .map_or(Value::Null, |at| items[at].clone()))
            };
        return match index.sequence() {
            Some(indices) => Ok(Value::array(
                indices.iter().map(at).collect::<Result<_, _>>()?,
            )),
            None => at(index),
        };
    }
    match (target, index) {
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
            if let Some(table) = environment::table_entries(entries) {
                return Ok(table
                    .iter()
                    .find(|(name, _)| name == &key)
                    .map_or(Value::Null, |(_, v)| v.clone()));
            }
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
            kind: ExprKind::ScriptBlock { statements, .. },
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

fn json_stringify_pretty(value: &Value, indent: usize) -> Result<String, RuntimeError> {
    let pad = " ".repeat(indent);
    let inner = " ".repeat(indent + 2);
    match value {
        Value::Array(items) if !items.is_empty() => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items.iter() {
                parts.push(format!(
                    "{inner}{}",
                    json_stringify_pretty(item, indent + 2)?
                ));
            }
            Ok(format!("[\n{}\n{pad}]", parts.join(",\n")))
        }
        Value::Object(entries) if entries.iter().any(|(key, _)| !is_hidden(key)) => {
            let mut parts = Vec::new();
            for (name, value) in entries.iter() {
                if is_hidden(name) {
                    continue;
                }
                parts.push(format!(
                    "{inner}{}: {}",
                    json_quote(name),
                    json_stringify_pretty(value, indent + 2)?
                ));
            }
            Ok(format!("{{\n{}\n{pad}}}", parts.join(",\n")))
        }
        _ => json_stringify(value),
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

fn mark_json_hashtables(value: Value) -> Value {
    match value {
        Value::Object(entries) => {
            let mut entries: Vec<(String, Value)> = entries
                .as_ref()
                .iter()
                .map(|(name, value)| (name.clone(), mark_json_hashtables(value.clone())))
                .collect();
            entries.push((
                "__TypeName".to_owned(),
                Value::String("System.Management.Automation.OrderedHashtable".into()),
            ));
            Value::object(entries)
        }
        Value::Array(items) => Value::array(
            items
                .as_ref()
                .iter()
                .map(|item| mark_json_hashtables(item.clone()))
                .collect(),
        ),
        other => other,
    }
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

/// The filesystem provider's home directory for `~` and `~/path`.
fn expand_home(path: &str) -> String {
    if path == "~"
        && let Ok(home) = std::env::var("HOME")
    {
        return home;
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    path.to_owned()
}

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

fn object_class(entries: &[(String, Value)]) -> Option<String> {
    entries.iter().find_map(|(key, value)| {
        if key == "__Class" {
            match value {
                Value::String(text) => Some(text.to_string_lossy()),
                _ => None,
            }
        } else {
            None
        }
    })
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
fn mark_format(
    input: &[Value],
    kind: &str,
    properties: Option<Value>,
    hide_headers: bool,
    auto_size: bool,
) -> Vec<Value> {
    input
        .iter()
        .map(|item| match item {
            Value::Object(entries) => {
                let mut entries = entries.as_ref().clone();
                entries.push(("__Format".to_owned(), Value::String(kind.into())));
                if let Some(properties) = &properties {
                    entries.push(("__TableProperty".to_owned(), properties.clone()));
                }
                if auto_size {
                    entries.push(("__AutoSize".to_owned(), Value::Bool(true)));
                }
                if hide_headers {
                    entries.push(("__HideHeaders".to_owned(), Value::Bool(true)));
                }
                Value::object(entries)
            }
            other => other.clone(),
        })
        .collect()
}

fn expand_property_value(input: &[Value], property: Option<Value>) -> Option<Value> {
    let property = property?;
    let names = flatten_value(property.clone());
    if !names
        .iter()
        .any(|name| name.to_string().contains(['*', '?']))
    {
        return Some(property);
    }
    let Some(Value::Object(entries)) = input.first() else {
        return Some(property);
    };
    let visible = visible_entries(entries);
    let mut expanded = Vec::new();
    for name in names {
        let text = name.to_string();
        if text.contains(['*', '?']) {
            for (candidate, _) in &visible {
                if ops::wildcard_match(candidate, &text, false) {
                    expanded.push(Value::String((*candidate).clone().into()));
                }
            }
        } else {
            expanded.push(name);
        }
    }
    if expanded.is_empty() {
        Some(property)
    } else {
        Some(Value::array(expanded))
    }
}

fn format_table_properties(value: &Value) -> Vec<String> {
    let Value::Object(entries) = value else {
        return Vec::new();
    };
    let Some((_, property)) = entries.iter().find(|(key, _)| key == "__TableProperty") else {
        return Vec::new();
    };
    match property {
        Value::Array(values) => values.iter().map(ToString::to_string).collect(),
        other => vec![other.to_string()],
    }
}

fn format_marker(value: &Value) -> Option<String> {
    let Value::Object(entries) = value else {
        return None;
    };
    entries.iter().find_map(|(key, value)| {
        if key == "__Format" {
            match value {
                Value::String(text) => Some(text.to_string_lossy()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn is_command_info(value: &Value) -> bool {
    matches!(
        hidden_type(value).as_deref(),
        Some(aliases::TYPE_NAME | native::APPLICATION_TYPE)
    )
}

fn uses_command_table(first: &Value) -> bool {
    is_command_info(first)
        && format_marker(first).as_deref() != Some("List")
        && format_table_properties(first).is_empty()
}

fn format_command_table(first: &Value, input: &[Value], width: usize) -> String {
    let flag = |name| matches!(first, Value::Object(entries) if entries.iter().any(|(key, value)| key == name && ops::is_true(value)));
    let items: Vec<_> = input
        .iter()
        .filter(|value| !value.is_null_like())
        .map(|value| {
            let Value::Object(entries) = value else {
                return CommandTableItem::Text(value.to_string());
            };
            let property = |name: &str| {
                entries
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.to_string())
                    .unwrap_or_default()
            };
            let kind = property("CommandType");
            let name = if kind == "Alias" {
                property("DisplayName")
            } else {
                property("Name")
            };
            CommandTableItem::Command([kind, name, property("Version"), property("Source")])
        })
        .collect();
    command_table(&items, width, flag("__AutoSize"), flag("__HideHeaders"))
}

fn format_out_string(input: &[Value], width: usize) -> String {
    let Some(first) = input.first() else {
        return String::new();
    };
    if uses_command_table(first) {
        return format_command_table(first, input, width);
    }
    if record_prefix(first).is_some() {
        return input.iter().map(|value| format!("{value}\n")).collect();
    }
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
    let force_list = format_marker(first).as_deref() == Some("List");
    let force_table = format_marker(first).as_deref() == Some("Table");
    let visible = visible_entries(entries);
    if !force_table && (force_list || visible.len() > 4) {
        let selected = format_table_properties(first);
        let rows: Vec<(String, String)> = if force_list && !selected.is_empty() {
            selected
                .into_iter()
                .map(|name| {
                    let value = visible
                        .iter()
                        .find(|(key, _)| key.eq_ignore_ascii_case(&name))
                        .map_or_else(String::new, |(_, value)| value.to_string());
                    (name, value)
                })
                .collect()
        } else {
            visible
                .iter()
                .map(|(name, value)| ((*name).clone(), value.to_string()))
                .collect()
        };
        let width = rows
            .iter()
            .map(|(name, _)| name.chars().count())
            .max()
            .unwrap_or(0);
        let mut text = String::from("\n");
        for (name, value) in &rows {
            text.push_str(&format!("{:<width$} : {}", name, value, width = width));
            text.push('\n');
        }
        text.push('\n');
        return text;
    }
    let selected = format_table_properties(first);
    let columns: Vec<String> = if force_table && !selected.is_empty() {
        selected
    } else {
        visible.iter().map(|(name, _)| (*name).clone()).collect()
    };
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
    let hide_headers = matches!(
        first,
        Value::Object(entries)
            if entries.iter().any(|(key, value)| key == "__HideHeaders" && ops::is_true(value))
    );
    let mut text = String::from("\n");
    if !hide_headers {
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
    }
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

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
    }
}

fn format_datetime_long(fields: [i32; 6]) -> String {
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
    let [year, month, day, hour, minute, second] = fields;
    let (hour12, ampm) = if hour == 0 {
        (12, "AM")
    } else if hour < 12 {
        (hour, "AM")
    } else if hour == 12 {
        (12, "PM")
    } else {
        (hour - 12, "PM")
    };
    let weekday = (weekday(year, month, day).rem_euclid(7)) as usize;
    format!(
        "{}, {} {}, {} {hour12}:{minute:02}:{second:02}\u{202F}{ampm}",
        WEEKDAYS[weekday],
        MONTHS[(month as usize).clamp(1, 12) - 1],
        day,
        year
    )
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
    // ConsoleHost reserves the last column of the reference 80-column
    // buffer when emitting newlines.
    if uses_command_table(first) {
        return format_command_table(first, objects, 79);
    }
    if format_marker(first).is_some() {
        return format_out_string(objects, 79);
    }
    if datetime_fields(first).is_some() {
        let mut text = String::from("\n");
        for object in objects {
            if let Some(fields) = datetime_fields(object) {
                text.push_str(&format_datetime_long(fields));
                text.push('\n');
            }
        }
        text.push('\n');
        return text;
    }
    let columns: Vec<String> = match first {
        Value::Object(entries) => {
            let visible = visible_entries(entries);
            if visible.len() > 4 {
                let width = visible
                    .iter()
                    .map(|(name, _)| name.chars().count())
                    .max()
                    .unwrap_or(0);
                let mut text = String::from("\n");
                for object in objects {
                    let Value::Object(entries) = object else {
                        continue;
                    };
                    for (name, value) in visible_entries(entries) {
                        text.push_str(&format!("{:<width$} : {}", name, value, width = width));
                        text.push('\n');
                    }
                    text.push('\n');
                }
                return text;
            }
            visible.iter().map(|(name, _)| (*name).clone()).collect()
        }
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

fn comparison_for(name: &str) -> Option<ops::Comparison> {
    Some(match name.to_ascii_lowercase().as_str() {
        "eq" | "ieq" => ops::Comparison::Eq,
        "ne" | "ine" => ops::Comparison::Ne,
        "gt" | "igt" => ops::Comparison::Gt,
        "ge" | "ige" => ops::Comparison::Ge,
        "lt" | "ilt" => ops::Comparison::Lt,
        "le" | "ile" => ops::Comparison::Le,
        "like" | "ilike" => ops::Comparison::Like,
        "notlike" | "inotlike" => ops::Comparison::NotLike,
        _ => return None,
    })
}

fn copy_directory(source: &str, destination: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let target = std::path::Path::new(destination).join(entry.file_name());
        if path.is_dir() {
            copy_directory(&path.to_string_lossy(), &target.to_string_lossy())?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn unix_mode_string(path: &std::path::Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return String::new();
        };
        let kind = if metadata.file_type().is_symlink() {
            'l'
        } else if metadata.is_dir() {
            'd'
        } else {
            '-'
        };
        let mode = metadata.permissions().mode();
        let mut text = String::new();
        text.push(kind);
        for shift in [6, 3, 0] {
            let bits = (mode >> shift) & 0o7;
            text.push(if bits & 0o4 != 0 { 'r' } else { '-' });
            text.push(if bits & 0o2 != 0 { 'w' } else { '-' });
            text.push(if bits & 0o1 != 0 { 'x' } else { '-' });
        }
        text
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        String::new()
    }
}

fn item_value(text: &str) -> Value {
    let target = std::path::Path::new(text);
    let mut entries = vec![
        (
            "__TypeName".to_owned(),
            Value::String(
                if target.is_dir() {
                    "System.IO.DirectoryInfo"
                } else {
                    "System.IO.FileInfo"
                }
                .into(),
            ),
        ),
        ("FullName".to_owned(), Value::String(text.to_owned().into())),
        ("PSIsContainer".to_owned(), Value::Bool(target.is_dir())),
        (
            "Mode".to_owned(),
            Value::String(if target.is_dir() { "d----" } else { "-----" }.into()),
        ),
        (
            "UnixMode".to_owned(),
            Value::String(unix_mode_string(target).into()),
        ),
        (
            "Attributes".to_owned(),
            Value::String(
                if target
                    .metadata()
                    .map(|meta| meta.permissions().readonly())
                    .unwrap_or(false)
                {
                    "ReadOnly"
                } else if target.is_dir() {
                    "Directory"
                } else {
                    "Normal"
                }
                .into(),
            ),
        ),
        (
            "IsReadOnly".to_owned(),
            Value::Bool(
                target
                    .metadata()
                    .map(|meta| meta.permissions().readonly())
                    .unwrap_or(false),
            ),
        ),
        (
            "Name".to_owned(),
            Value::String(
                target
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().to_string())
                    .into(),
            ),
        ),
    ];
    if let Ok(metadata) = std::fs::symlink_metadata(target)
        && metadata.file_type().is_symlink()
    {
        entries.push(("LinkType".to_owned(), Value::String("SymbolicLink".into())));
        entries.push((
            "Target".to_owned(),
            Value::String(
                std::fs::read_link(target)
                    .map_or_else(
                        |_| String::new(),
                        |path| path.to_string_lossy().into_owned(),
                    )
                    .into(),
            ),
        ));
    }
    if target.is_file() {
        entries.push((
            "Length".to_owned(),
            Value::Int64(target.metadata().map_or(0, |meta| meta.len()) as i64),
        ));
        entries.push((
            "BaseName".to_owned(),
            Value::String(
                target
                    .file_stem()
                    .map_or_else(String::new, |value| value.to_string_lossy().to_string())
                    .into(),
            ),
        ));
        entries.push((
            "Extension".to_owned(),
            Value::String(
                target
                    .extension()
                    .map_or_else(String::new, |value| format!(".{}", value.to_string_lossy()))
                    .into(),
            ),
        ));
    }
    Value::object(entries)
}

fn collect_children(
    path: &str,
    recurse: bool,
    force: bool,
    prefix: &str,
    entries: &mut Vec<(String, bool, u64)>,
) {
    if let Ok(read) = std::fs::read_dir(path) {
        for entry in read.flatten() {
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            let length = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            if let Some(name) = entry.file_name().to_str() {
                if !force && name.starts_with('.') {
                    continue;
                }
                let relative = format!("{prefix}{name}");
                entries.push((relative.clone(), is_dir, length));
                if recurse && is_dir {
                    collect_children(
                        &entry.path().to_string_lossy(),
                        recurse,
                        force,
                        &format!("{relative}/"),
                        entries,
                    );
                }
            }
        }
    }
}

const BUILTIN_COMMANDS: &[&str] = &[
    "Add-Content",
    "ConvertFrom-Json",
    "New-Object",
    "New-TimeSpan",
    "ConvertTo-Json",
    "Copy-Item",
    "ForEach-Object",
    "Get-Alias",
    "Get-ChildItem",
    "Get-Command",
    "Get-ItemProperty",
    "Get-PSDrive",
    "Get-PSProvider",
    "Remove-Module",
    "Set-ItemProperty",
    "Set-Item",
    "Get-Content",
    "Get-Culture",
    "Get-Item",
    "Get-Location",
    "Get-Random",
    "Get-Unique",
    "Get-UICulture",
    "Get-Variable",
    "Import-Module",
    "Invoke-Expression",
    "Join-Path",
    "Measure-Object",
    "Move-Item",
    "New-Item",
    "Out-File",
    "Out-Null",
    "Out-String",
    "Remove-Item",
    "Remove-Variable",
    "Select-Object",
    "Set-Alias",
    "Set-Content",
    "Set-Location",
    "Set-Variable",
    "Sort-Object",
    "Split-Path",
    "Start-Sleep",
    "Test-Path",
    "Where-Object",
    "Write-Debug",
    "Write-Error",
    "Write-Host",
    "Write-Information",
    "Write-Output",
    "Write-Verbose",
    "Write-Warning",
];

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (index, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..80 {
            w[index] = (w[index - 3] ^ w[index - 8] ^ w[index - 14] ^ w[index - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (index, word) in w.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut digest = [0u8; 20];
    for (index, word) in h.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn md5(data: &[u8]) -> [u8; 16] {
    const SHIFTS: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let mut h: [u32; 4] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in message.as_chunks::<64>().0 {
        let mut m = [0u32; 16];
        for (index, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            m[index] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        for (index, shift) in SHIFTS.iter().enumerate() {
            let (f, g) = match index {
                0..=15 => ((b & c) | (!b & d), index),
                16..=31 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let k = ((index + 1) as f64).sin().abs() * 4_294_967_296.0;
            let k = k as u32;
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(k)
                    .wrapping_add(m[g])
                    .rotate_left(*shift),
            );
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
    }
    let mut digest = [0u8; 16];
    for (index, word) in h.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    digest
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (index, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut a = h;
        for index in 0..64 {
            let s1 = a[4].rotate_right(6) ^ a[4].rotate_right(11) ^ a[4].rotate_right(25);
            let ch = (a[4] & a[5]) ^ (!a[4] & a[6]);
            let temp1 = a[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a[0].rotate_right(2) ^ a[0].rotate_right(13) ^ a[0].rotate_right(22);
            let maj = (a[0] & a[1]) ^ (a[0] & a[2]) ^ (a[1] & a[2]);
            let temp2 = s0.wrapping_add(maj);
            a[7] = a[6];
            a[6] = a[5];
            a[5] = a[4];
            a[4] = a[3].wrapping_add(temp1);
            a[3] = a[2];
            a[2] = a[1];
            a[1] = a[0];
            a[0] = temp1.wrapping_add(temp2);
        }
        for index in 0..8 {
            h[index] = h[index].wrapping_add(a[index]);
        }
    }
    let mut digest = [0u8; 32];
    for (index, word) in h.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn csv_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

fn parse_csv_line_with(line: &str, delimiter: char) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if quoted && chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            }
            ch if ch == delimiter && !quoted => cells.push(std::mem::take(&mut cell)),
            other => cell.push(other),
        }
    }
    cells.push(cell);
    cells
}

fn flatten_values(values: Vec<Value>) -> Vec<Value> {
    values.into_iter().flat_map(flatten_value).collect()
}

fn contains_group(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Group(_) => true,
        ExprKind::Member { target, .. } => contains_group(target),
        ExprKind::Index { target, index } => contains_group(target) || contains_group(index),
        ExprKind::Binary { left, right, .. } => contains_group(left) || contains_group(right),
        ExprKind::Unary { operand, .. } => contains_group(operand),
        ExprKind::Paren(inner) | ExprKind::Subexpression(inner) => contains_group(inner),
        ExprKind::Ternary {
            condition,
            when_true,
            when_false,
        } => contains_group(condition) || contains_group(when_true) || contains_group(when_false),
        _ => false,
    }
}

fn flatten_value(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items.as_ref().clone(),
        Value::ArrayList(list) => list.items().to_vec(),
        Value::Null | Value::AutomationNull => Vec::new(),
        value => vec![value],
    }
}

fn scalar_or_array(mut values: std::vec::IntoIter<Value>) -> Value {
    match values.len() {
        0 | 1 => values.next().unwrap_or(Value::Null),
        _ => Value::array(values.collect()),
    }
}

fn native_exit_code(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    status.code().unwrap_or(1)
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

fn lookup(name: &str, variables: &Variables, last_success: bool) -> Option<Value> {
    match name {
        "null" => return Some(Value::Null),
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        "?" => return Some(Value::Bool(last_success)),
        "_" => return variables.get("_").cloned().or(Some(Value::Null)),
        "PSItem" => return variables.get("_").cloned().or(Some(Value::Null)),
        _ => {}
    }
    if let Some(rest) = environment::drive_name(name) {
        return Some(environment::get(variables, rest).unwrap_or(Value::Null));
    }
    let key = strip_scope(name);
    if key.eq_ignore_ascii_case("Error") {
        return Some(variables.error_value(is_global_scope(name)));
    }
    let key = preferences::variable_name(key).unwrap_or(key);
    let key = encoding::variable_name(key).unwrap_or(key);
    if let Some(value) = variables.get(key) {
        return Some(value.clone());
    }
    automatic_variable(key)
}

fn bitwise_values(left: &Value, right: &Value, op: &str) -> Result<Value, RuntimeError> {
    if let Some(bits) = wildcards::typed_options(left) {
        let right = wildcards::typed_options(right).map_or_else(|| convert_int32(right), Ok)?;
        let result = ops::bitwise(&Value::Int32(bits), &Value::Int32(right), op)?;
        if let Value::Int32(bits) = result {
            return Ok(wildcards::options_value(bits));
        }
        return Ok(result);
    }
    if let Some(bits) = wildcards::typed_options(right) {
        return ops::bitwise(left, &Value::Int32(bits), op);
    }
    ops::bitwise(left, right, op)
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
        key if key.eq_ignore_ascii_case("PSHOME") => Some(Value::String(
            environment::ps_home().to_string_lossy().as_ref().into(),
        )),
        key if key.eq_ignore_ascii_case("PSCulture") || key.eq_ignore_ascii_case("PSUICulture") => {
            Some(Value::String(globalization::current_name().into()))
        }
        "PSStyle" => Some(Value::object(vec![(
            "OutputRendering".to_owned(),
            Value::String("PlainText".into()),
        )])),
        "ErrorView" => Some(Value::String("ConciseView".into())),
        "OutputEncoding" => Some(encoding::UTF8.value()),
        "PSApplicationOutputEncoding" => Some(Value::Null),
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
        key if let Some(value) = preferences::default_value(key) => Some(value),
        "PID" => Some(Value::Int32(std::process::id() as i32)),
        "HOME" => std::env::var("HOME")
            .ok()
            .map(|value| Value::String(value.into())),
        _ => None,
    }
}

fn type_full_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "object" => "System.Object".to_owned(),
        "array" => "System.Array".to_owned(),
        "int" | "int32" => "System.Int32".to_owned(),
        "long" | "int64" => "System.Int64".to_owned(),
        "uint" | "uint32" => "System.UInt32".to_owned(),
        "ulong" | "uint64" => "System.UInt64".to_owned(),
        "short" | "int16" => "System.Int16".to_owned(),
        "ushort" | "uint16" => "System.UInt16".to_owned(),
        "byte" => "System.Byte".to_owned(),
        "sbyte" => "System.SByte".to_owned(),
        "string" => "System.String".to_owned(),
        "bool" | "boolean" => "System.Boolean".to_owned(),
        "double" => "System.Double".to_owned(),
        "decimal" => "System.Decimal".to_owned(),
        "biginteger" => "System.Numerics.BigInteger".to_owned(),
        "collections.arraylist" | "system.collections.arraylist" => {
            rpsh_runtime::ArrayList::TYPE.into()
        }
        "collections.ilist" | "system.collections.ilist" => "System.Collections.IList".into(),
        "collections.icollection" | "system.collections.icollection" => {
            "System.Collections.ICollection".into()
        }
        "collections.ienumerable" | "system.collections.ienumerable" => {
            "System.Collections.IEnumerable".into()
        }
        "cultureinfo" | "globalization.cultureinfo" => globalization::CULTURE_TYPE.to_owned(),
        "text.normalizationform" => globalization::FORM_TYPE.to_owned(),
        "management.automation.actionpreference"
        | "system.management.automation.actionpreference" => preferences::TYPE.into(),
        "stringcomparison" => globalization::COMPARISON_TYPE.to_owned(),
        "pscustomobject" => "System.Management.Automation.PSCustomObject".to_owned(),
        _ => name.to_owned(),
    }
}
fn normalize_type(type_name: &str) -> String {
    let lower = type_name.to_ascii_lowercase();
    lower
        .strip_prefix("system.")
        .map_or(lower.clone(), ToOwned::to_owned)
}

fn parse_datetime_text(text: &str) -> Option<(i32, i32, i32, i32, i32, i32)> {
    let text = text.trim();
    let (date, time) = text.split_once(' ').unwrap_or((text, "00:00:00"));
    let mut date_parts = date.split(['-', '/']);
    let year = date_parts.next()?.parse().ok()?;
    let month = date_parts.next()?.parse().ok()?;
    let day = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour = time_parts
        .next()
        .map_or(Some(0), |value| value.parse().ok())?;
    let minute = time_parts
        .next()
        .map_or(Some(0), |value| value.parse().ok())?;
    let second = time_parts
        .next()
        .map_or(Some(0), |value| value.parse().ok())?;
    Some((year, month, day, hour, minute, second))
}

fn format_datetime(
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    format: &str,
) -> String {
    let mut out = String::new();
    let mut chars = format.chars().peekable();
    while let Some(ch) = chars.next() {
        let count = 1 + chars.clone().take_while(|next| *next == ch).count();
        for _ in 1..count {
            chars.next();
        }
        match (ch, count) {
            ('y', 4) => out.push_str(&format!("{year:04}")),
            ('y', 2) => out.push_str(&format!("{:02}", year % 100)),
            ('M', 2) => out.push_str(&format!("{month:02}")),
            ('M', 1) => out.push_str(&month.to_string()),
            ('d', 2) => out.push_str(&format!("{day:02}")),
            ('d', 1) => out.push_str(&day.to_string()),
            ('H', 2) => out.push_str(&format!("{hour:02}")),
            ('H', 1) => out.push_str(&hour.to_string()),
            ('m', 2) => out.push_str(&format!("{minute:02}")),
            ('m', 1) => out.push_str(&minute.to_string()),
            ('s', 2) => out.push_str(&format!("{second:02}")),
            ('s', 1) => out.push_str(&second.to_string()),
            ('d', 4) => {
                const WEEKDAYS: [&str; 7] = [
                    "Sunday",
                    "Monday",
                    "Tuesday",
                    "Wednesday",
                    "Thursday",
                    "Friday",
                    "Saturday",
                ];
                out.push_str(WEEKDAYS[weekday(year, month, day) as usize % 7]);
            }
            ('d', 3) => {
                const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                out.push_str(WEEKDAYS[weekday(year, month, day) as usize % 7]);
            }
            ('M', 4) => {
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
                out.push_str(MONTHS[(month as usize).clamp(1, 12) - 1]);
            }
            ('M', 3) => {
                const MONTHS: [&str; 12] = [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov",
                    "Dec",
                ];
                out.push_str(MONTHS[(month as usize).clamp(1, 12) - 1]);
            }
            ('h', 2) | ('h', 1) => {
                let hour12 = if hour == 0 {
                    12
                } else if hour > 12 {
                    hour - 12
                } else {
                    hour
                };
                if count == 2 {
                    out.push_str(&format!("{hour12:02}"));
                } else {
                    out.push_str(&hour12.to_string());
                }
            }
            ('t', 2) | ('t', 1) => {
                let ampm = if hour < 12 { "AM" } else { "PM" };
                if count == 2 {
                    out.push_str(ampm);
                } else {
                    out.push(ampm.chars().next().unwrap_or('A'));
                }
            }
            (other, _) => {
                for _ in 0..count {
                    out.push(other);
                }
            }
        }
    }
    out
}

fn add_days_to_date(year: i32, month: i32, day: i32, days: i64) -> (i32, i32, i32) {
    let (mut year, mut month, mut day) = (year, month.max(1), day.max(1));
    let mut leftover = days;
    while leftover > 0 {
        let remaining = i64::from(days_in_month(year, month) - day);
        if leftover <= remaining {
            day += leftover as i32;
            break;
        }
        leftover -= remaining + 1;
        day = 1;
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    while leftover < 0 {
        let available = i64::from(day - 1);
        if -leftover <= available {
            day += leftover as i32;
            break;
        }
        leftover += available + 1;
        month -= 1;
        if month < 1 {
            month = 12;
            year -= 1;
        }
        day = days_in_month(year, month);
    }
    (year, month, day)
}

const TICKS_PER_MILLISECOND: i64 = 10_000;
const TICKS_PER_SECOND: i64 = 10_000_000;
const TICKS_PER_MINUTE: i64 = 60 * TICKS_PER_SECOND;
const TICKS_PER_HOUR: i64 = 60 * TICKS_PER_MINUTE;
const TICKS_PER_DAY: i64 = 24 * TICKS_PER_HOUR;

fn timespan_value(ticks: i64) -> Value {
    let total_seconds = ticks / TICKS_PER_SECOND;
    let total_minutes = ticks / TICKS_PER_MINUTE;
    let total_hours = ticks / TICKS_PER_HOUR;
    let days = total_hours / 24;
    Value::object(vec![
        ("Days".to_owned(), Value::Int32(days as i32)),
        ("Hours".to_owned(), Value::Int32((total_hours % 24) as i32)),
        (
            "Minutes".to_owned(),
            Value::Int32((total_minutes % 60) as i32),
        ),
        (
            "Seconds".to_owned(),
            Value::Int32((total_seconds % 60) as i32),
        ),
        (
            "Milliseconds".to_owned(),
            Value::Int32(((ticks / TICKS_PER_MILLISECOND) % 1000) as i32),
        ),
        ("Ticks".to_owned(), Value::Int64(ticks)),
        (
            "TotalDays".to_owned(),
            Value::Double(ticks as f64 / TICKS_PER_DAY as f64),
        ),
        (
            "TotalHours".to_owned(),
            Value::Double(ticks as f64 / TICKS_PER_HOUR as f64),
        ),
        (
            "TotalMinutes".to_owned(),
            Value::Double(ticks as f64 / TICKS_PER_MINUTE as f64),
        ),
        (
            "TotalSeconds".to_owned(),
            Value::Double(ticks as f64 / TICKS_PER_SECOND as f64),
        ),
        (
            "TotalMilliseconds".to_owned(),
            Value::Double(ticks as f64 / TICKS_PER_MILLISECOND as f64),
        ),
        (
            "__TypeName".to_owned(),
            Value::String("System.TimeSpan".into()),
        ),
    ])
}

fn format_timespan(ticks: i64) -> String {
    let negative = ticks < 0;
    let absolute = ticks.unsigned_abs();
    let days = absolute / TICKS_PER_DAY as u64;
    let rest = absolute % TICKS_PER_DAY as u64;
    let hours = rest / TICKS_PER_HOUR as u64;
    let rest = rest % TICKS_PER_HOUR as u64;
    let minutes = rest / TICKS_PER_MINUTE as u64;
    let rest = rest % TICKS_PER_MINUTE as u64;
    let seconds = rest / TICKS_PER_SECOND as u64;
    let fraction = rest % TICKS_PER_SECOND as u64;
    let sign = if negative { "-" } else { "" };
    let mut text = if days > 0 {
        format!("{sign}{days}.{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    };
    if fraction > 0 {
        text.push_str(&format!(".{fraction:07}"));
    }
    text
}

fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn timespan_ticks(value: &Value) -> Option<i64> {
    let Value::Object(entries) = value else {
        return None;
    };
    let is_timespan = entries.iter().any(|(key, value)| {
        key == "__TypeName"
            && matches!(value, Value::String(text) if text.to_string_lossy() == "System.TimeSpan")
    });
    if !is_timespan {
        return None;
    }
    entries
        .iter()
        .find(|(key, _)| key == "Ticks")
        .and_then(|(_, value)| convert_int64(value).ok())
}

fn datetime_fields(value: &Value) -> Option<[i32; 6]> {
    let Value::Object(entries) = value else {
        return None;
    };
    let is_datetime = entries.iter().any(|(key, value)| {
        key == "__TypeName"
            && matches!(value, Value::String(text) if text.to_string_lossy() == "System.DateTime")
    });
    if !is_datetime {
        return None;
    }
    let field = |name: &str| -> i32 {
        entries
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| convert_int32(value).ok())
            .unwrap_or(0)
    };
    Some([
        field("__Year"),
        field("__Month"),
        field("__Day"),
        field("__Hour"),
        field("__Minute"),
        field("__Second"),
    ])
}

fn datetime_ticks(fields: [i32; 6]) -> i64 {
    days_from_civil(fields[0], fields[1], fields[2]) * TICKS_PER_DAY
        + i64::from(fields[3]) * TICKS_PER_HOUR
        + i64::from(fields[4]) * TICKS_PER_MINUTE
        + i64::from(fields[5]) * TICKS_PER_SECOND
}

fn add_ticks_to_datetime(fields: [i32; 6], ticks: i64) -> Value {
    let days = ticks.div_euclid(TICKS_PER_DAY);
    let rest = ticks.rem_euclid(TICKS_PER_DAY);
    let seconds = i64::from(fields[3]) * 3600 + i64::from(fields[4]) * 60 + i64::from(fields[5]);
    let total = seconds + rest / TICKS_PER_SECOND;
    let extra_days = total.div_euclid(86_400);
    let time = total.rem_euclid(86_400);
    let (year, month, day) = add_days_to_date(fields[0], fields[1], fields[2], days + extra_days);
    datetime_value(
        year,
        month,
        day,
        (time / 3600) as i32,
        (time % 3600 / 60) as i32,
        (time % 60) as i32,
    )
}

fn add_values(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    if let (Some(left), Some(right)) = (timespan_ticks(left), timespan_ticks(right)) {
        return Ok(timespan_value(left + right));
    }
    if let (Some(datetime), Some(ticks)) = (datetime_fields(left), timespan_ticks(right)) {
        return Ok(add_ticks_to_datetime(datetime, ticks));
    }
    ops::add(left, right)
}

fn subtract_values(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    if let (Some(left), Some(right)) = (timespan_ticks(left), timespan_ticks(right)) {
        return Ok(timespan_value(left - right));
    }
    if let (Some(datetime), Some(ticks)) = (datetime_fields(left), timespan_ticks(right)) {
        return Ok(add_ticks_to_datetime(datetime, -ticks));
    }
    if let (Some(left), Some(right)) = (datetime_fields(left), datetime_fields(right)) {
        return Ok(timespan_value(datetime_ticks(left) - datetime_ticks(right)));
    }
    ops::subtract(left, right)
}

fn compare_values(
    left: &Value,
    right: &Value,
    comparison: ops::Comparison,
    case_sensitive: bool,
) -> Result<Value, RuntimeError> {
    let ordering =
        if let (Some(left), Some(right)) = (datetime_fields(left), datetime_fields(right)) {
            Some(datetime_ticks(left).cmp(&datetime_ticks(right)))
        } else if let (Some(left), Some(right)) = (timespan_ticks(left), timespan_ticks(right)) {
            Some(left.cmp(&right))
        } else {
            None
        };
    if let Some(ordering) = ordering {
        let result = match comparison {
            ops::Comparison::Eq => Some(ordering == std::cmp::Ordering::Equal),
            ops::Comparison::Ne => Some(ordering != std::cmp::Ordering::Equal),
            ops::Comparison::Gt => Some(ordering == std::cmp::Ordering::Greater),
            ops::Comparison::Ge => Some(ordering != std::cmp::Ordering::Less),
            ops::Comparison::Lt => Some(ordering == std::cmp::Ordering::Less),
            ops::Comparison::Le => Some(ordering != std::cmp::Ordering::Greater),
            _ => None,
        };
        if let Some(result) = result {
            return Ok(Value::Bool(result));
        }
    }
    ops::compare(left, right, comparison, case_sensitive)
}

fn negate_value(value: &Value) -> Result<Value, RuntimeError> {
    if let Some(ticks) = timespan_ticks(value) {
        return Ok(timespan_value(-ticks));
    }
    ops::negate(value)
}

fn datetime_value(year: i32, month: i32, day: i32, hour: i32, minute: i32, second: i32) -> Value {
    let text = format!("{month:02}/{day:02}/{year:04} {hour:02}:{minute:02}:{second:02}");
    Value::object(vec![
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
    ])
}

fn parse_timespan_text(text: &str) -> Option<i64> {
    let text = text.trim();
    let negative = text.starts_with('-');
    let body = text.trim_start_matches('-');
    let (days, rest) = match body.split_once('.') {
        Some((days, rest)) if days.chars().all(|ch| ch.is_ascii_digit()) => {
            (days.parse::<i64>().ok()?, rest)
        }
        _ => (0, body),
    };
    let mut parts = rest.split(':');
    let hours = parts.next()?.parse::<i64>().ok()?;
    let minutes = parts.next()?.parse::<i64>().ok()?;
    let seconds_text = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let (seconds, fraction) = match seconds_text.split_once('.') {
        Some((seconds, fraction)) => (seconds, fraction),
        None => (seconds_text, ""),
    };
    let seconds = seconds.parse::<i64>().ok()?;
    let ticks = if fraction.is_empty() {
        0
    } else {
        let mut fraction_text = fraction.chars().take(7).collect::<String>();
        while fraction_text.len() < 7 {
            fraction_text.push('0');
        }
        fraction_text.parse::<i64>().ok()?
    };
    let total = days * TICKS_PER_DAY
        + hours * TICKS_PER_HOUR
        + minutes * TICKS_PER_MINUTE
        + seconds * TICKS_PER_SECOND
        + ticks;
    Some(if negative { -total } else { total })
}

fn static_member(
    type_name: &str,
    name: &str,
    variables: &Variables,
) -> Result<Value, RuntimeError> {
    let enum_key = format!("__enum::{type_name}::{name}");
    if let Some(value) = variables.get(&enum_key) {
        return Ok(value.clone());
    }
    let kind = normalize_type(type_name);
    if kind == "environmentvariabletarget"
        && let Some(value) = environment::target_member(name)
    {
        return Ok(value);
    }
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
        ("cultureinfo" | "globalization.cultureinfo", "currentculture" | "currentuiculture") => {
            Ok(globalization::current_culture())
        }
        ("cultureinfo" | "globalization.cultureinfo", "invariantculture") => {
            Ok(globalization::invariant_culture())
        }
        ("stringcomparison", mode) => {
            Ok(globalization::comparison_member(mode).unwrap_or(Value::Null))
        }
        ("text.normalizationform", form) => {
            Ok(globalization::form_member(form).unwrap_or(Value::Null))
        }
        ("management.automation.actionpreference", name) => {
            Ok(preferences::member(name).unwrap_or(Value::Null))
        }
        ("datetimekind", "utc") => Ok(Value::Int32(1)),
        ("datetimekind", "local") => Ok(Value::Int32(2)),
        ("datetimekind", "unspecified") => Ok(Value::Int32(0)),
        ("console", "isoutputredirected") => Ok(Value::Bool(true)),
        ("console", "windowwidth") => Ok(Value::Int32(0)),
        ("nullstring" | "management.automation.language.nullstring", "value") => Ok(typed_scalar(
            "System.Management.Automation.Language.NullString",
            "",
        )),
        ("management.automation.wildcardoptions", name) => wildcards::option_member(name),
        ("console", "outputencoding") => Ok(encoding::console().value()),
        ("text.encoding", name) => {
            Encoding::property(name)
                .map(Encoding::value)
                .ok_or(RuntimeError::NotImplemented {
                    feature: "this Encoding property",
                })
        }
        ("double", "maxvalue") => Ok(Value::Double(f64::MAX)),
        ("double", "minvalue") => Ok(Value::Double(f64::MIN)),
        ("double", "epsilon") => Ok(Value::Double(f64::EPSILON)),
        ("string", "empty") => Ok(Value::String("".into())),
        ("timespan", "zero") => Ok(timespan_value(0)),
        ("math", "pi") => Ok(Value::Double(std::f64::consts::PI)),
        ("math", "e") => Ok(Value::Double(std::f64::consts::E)),
        _ => Err(RuntimeError::NotImplemented {
            feature: "this static member",
        }),
    }
}

fn to_radix(value: i64, base: i32) -> String {
    if value == 0 {
        return "0".to_owned();
    }
    let mut number = value.unsigned_abs();
    let base = base as u32;
    let mut digits = Vec::new();
    while number > 0 {
        let digit = (number % base as u64) as u32;
        digits.push(std::char::from_digit(digit, base).unwrap_or('0'));
        number /= base as u64;
    }
    if value < 0 {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

fn is_null_string(value: &Value) -> bool {
    hidden_type(value).as_deref() == Some("System.Management.Automation.Language.NullString")
}

fn dotnet_string_argument(value: &Value) -> Option<PsString> {
    if is_null_string(value) {
        None
    } else {
        Some(value.to_ps_string())
    }
}

fn invoke_static(
    type_name: &str,
    name: &str,
    arguments: &[Value],
    variables: &mut Variables,
) -> Result<Value, RuntimeError> {
    let kind = normalize_type(type_name);
    let name = name.to_ascii_lowercase();
    match (kind.as_str(), name.as_str()) {
        ("collections.arraylist", "new") => array_list::construct(arguments),
        ("cultureinfo" | "globalization.cultureinfo", method @ ("new" | "getcultureinfo")) => {
            globalization::culture_method(method, arguments)
        }
        (
            "environment",
            method @ ("getenvironmentvariable"
            | "setenvironmentvariable"
            | "getenvironmentvariables"),
        ) => environment::invoke(method, arguments, variables),
        ("wildcardpattern" | "management.automation.wildcardpattern", method) => {
            wildcards::static_method(method, arguments)
        }
        ("text.encoding", "getencoding") if arguments.len() == 1 => {
            Encoding::by_name(&arguments[0]).map(Encoding::value).ok_or(
                RuntimeError::NotImplemented {
                    feature: "this encoding name or code page",
                },
            )
        }
        ("text.utf8encoding", "new")
            if arguments.len() <= 2 && !arguments.get(1).is_some_and(ops::is_true) =>
        {
            Ok(Encoding::new_utf8(arguments.first().is_some_and(ops::is_true)).value())
        }

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
        ("timespan", "fromdays")
        | ("timespan", "fromhours")
        | ("timespan", "fromminutes")
        | ("timespan", "fromseconds")
        | ("timespan", "frommilliseconds")
        | ("timespan", "fromticks") => {
            let value = arguments.first().unwrap_or(&Value::Null);
            let ticks = match name.as_str() {
                "fromticks" => convert_int64(value)?,
                "frommilliseconds" => {
                    (convert_double(value)? * TICKS_PER_MILLISECOND as f64).round() as i64
                }
                "fromseconds" => (convert_double(value)? * TICKS_PER_SECOND as f64).round() as i64,
                "fromminutes" => (convert_double(value)? * TICKS_PER_MINUTE as f64).round() as i64,
                "fromhours" => (convert_double(value)? * TICKS_PER_HOUR as f64).round() as i64,
                _ => (convert_double(value)? * TICKS_PER_DAY as f64).round() as i64,
            };
            Ok(timespan_value(ticks))
        }
        ("timespan", "new") => {
            let number = |index: usize| -> Result<i64, RuntimeError> {
                arguments.get(index).map_or(Ok(0), convert_int64)
            };
            let ticks = number(0)? * TICKS_PER_DAY
                + number(1)? * TICKS_PER_HOUR
                + number(2)? * TICKS_PER_MINUTE
                + number(3)? * TICKS_PER_SECOND
                + number(4)? * TICKS_PER_MILLISECOND;
            Ok(timespan_value(ticks))
        }
        ("timespan", "parse") => {
            let text = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            parse_timespan_text(&text)
                .map(timespan_value)
                .ok_or(RuntimeError::NotImplemented {
                    feature: "this TimeSpan input",
                })
        }
        ("version", "parse") => {
            let value = arguments.first().cloned().unwrap_or(Value::Null);
            cast_to("version", value)
        }
        ("datetime", "parse") => {
            let value = arguments.first().cloned().unwrap_or(Value::Null);
            cast_to("datetime", value)
        }
        ("char", "isdigit")
        | ("char", "isletter")
        | ("char", "iswhitespace")
        | ("char", "isupper")
        | ("char", "islower") => {
            let text = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            let ch = text.chars().next().unwrap_or('\0');
            let result = if name == "isdigit" {
                ch.is_ascii_digit()
            } else if name == "isletter" {
                ch.is_alphabetic()
            } else if name == "isupper" {
                ch.is_uppercase()
            } else if name == "islower" {
                ch.is_lowercase()
            } else {
                ch.is_whitespace()
            };
            Ok(Value::Bool(result))
        }
        ("char", "toupper") | ("char", "tolower") => {
            let text = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            let ch = text.chars().next().unwrap_or('\0');
            let result = if name == "toupper" {
                ch.to_uppercase().next().unwrap_or(ch)
            } else {
                ch.to_lowercase().next().unwrap_or(ch)
            };
            Ok(Value::Char(u32::from(result) as u16))
        }
        ("convert", "toint32") | ("convert", "toint64") => {
            let Some(value) = arguments.first() else {
                return Err(RuntimeError::NotImplemented {
                    feature: "Convert without a value",
                });
            };
            let text = value.to_string();
            let parsed = match arguments.get(1) {
                Some(base) => {
                    let base = convert_int32(base)?;
                    i64::from_str_radix(text.trim(), base as u32).ok()
                }
                None => parse_integer_text(text.trim()).and_then(|value| i64::try_from(value).ok()),
            };
            let Some(parsed) = parsed else {
                return Err(RuntimeError::Script {
                    id: "FormatException",
                    category: "NotSpecified: (:) [], MethodInvocationException".to_owned(),
                    exception_type: "System.Management.Automation.MethodInvocationException",
                    message: format!(
                        "Exception calling \"ToInt32\" with \"1\" argument(s): \"The input string '{text}' was not in a correct format.\""
                    ),
                });
            };
            if kind == "convert" && name == "toint32" {
                i32::try_from(parsed)
                    .map(Value::Int32)
                    .map_err(|_| RuntimeError::NotImplemented {
                        feature: "integer conversion overflow",
                    })
            } else {
                Ok(Value::Int64(parsed))
            }
        }
        ("convert", "todouble") => Ok(Value::Double(convert_double(
            arguments.first().unwrap_or(&Value::Null),
        )?)),
        ("convert", "toboolean") => Ok(Value::Bool(convert_bool(
            arguments.first().unwrap_or(&Value::Null),
        ))),
        ("convert", "tostring") => {
            let Some(value) = arguments.first() else {
                return Err(RuntimeError::NotImplemented {
                    feature: "Convert.ToString without a value",
                });
            };
            match arguments.get(1) {
                Some(base) => {
                    let base = convert_int32(base)?;
                    let number = convert_int64(value)?;
                    Ok(Value::String(to_radix(number, base).into()))
                }
                None => Ok(Value::String(value.to_string().into())),
            }
        }
        ("io.path", "combine") => {
            let mut path = std::path::PathBuf::new();
            for argument in arguments {
                path.push(argument.to_string());
            }
            Ok(Value::String(path.to_string_lossy().into_owned().into()))
        }
        ("io.path", "getfilename") => {
            let path = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            Ok(Value::String(
                std::path::Path::new(&path)
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
                    .into(),
            ))
        }
        ("io.path", "getdirectoryname") => {
            let path = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            Ok(Value::String(
                std::path::Path::new(&path)
                    .parent()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
                    .into(),
            ))
        }
        ("io.path", "getextension") => {
            let path = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            Ok(Value::String(
                std::path::Path::new(&path)
                    .extension()
                    .map_or_else(String::new, |name| format!(".{}", name.to_string_lossy()))
                    .into(),
            ))
        }
        ("io.path", "getfilenamewithoutextension") => {
            let path = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            Ok(Value::String(
                std::path::Path::new(&path)
                    .file_stem()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
                    .into(),
            ))
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
        ("math", "sign") => match arguments.first() {
            Some(Value::Int32(value)) => Ok(Value::Int32(value.signum())),
            Some(Value::Int64(value)) => Ok(Value::Int32(value.signum() as i32)),
            Some(value) => {
                let value = convert_double(value)?;
                Ok(Value::Int32(if value > 0.0 {
                    1
                } else if value < 0.0 {
                    -1
                } else {
                    0
                }))
            }
            None => Err(RuntimeError::NotImplemented {
                feature: "Sign with no argument",
            }),
        },
        ("math", "clamp") => {
            let (Some(value), Some(minimum), Some(maximum)) =
                (arguments.first(), arguments.get(1), arguments.get(2))
            else {
                return Err(RuntimeError::NotImplemented {
                    feature: "Clamp with fewer than three arguments",
                });
            };
            if let (Value::Int32(value), Value::Int32(minimum), Value::Int32(maximum)) =
                (value, minimum, maximum)
            {
                Ok(Value::Int32((*value).clamp(*minimum, *maximum)))
            } else {
                Ok(Value::Double(
                    convert_double(value)?
                        .clamp(convert_double(minimum)?, convert_double(maximum)?),
                ))
            }
        }
        ("math", "exp") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.exp(),
        )),
        ("math", "log") => {
            let value = convert_double(arguments.first().unwrap_or(&Value::Null))?;
            let result = match arguments.get(1) {
                Some(base) => value.log(convert_double(base)?),
                None => value.ln(),
            };
            Ok(Value::Double(result))
        }
        ("math", "log10") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.log10(),
        )),
        ("math", "sin") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.sin(),
        )),
        ("math", "cos") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.cos(),
        )),
        ("math", "tan") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.tan(),
        )),
        ("math", "asin") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.asin(),
        )),
        ("math", "acos") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.acos(),
        )),
        ("math", "atan") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.atan(),
        )),
        ("math", "atan2") => {
            let y = convert_double(arguments.first().unwrap_or(&Value::Null))?;
            let x = convert_double(arguments.get(1).unwrap_or(&Value::Null))?;
            Ok(Value::Double(y.atan2(x)))
        }
        ("math", "sinh") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.sinh(),
        )),
        ("math", "cosh") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.cosh(),
        )),
        ("math", "tanh") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.tanh(),
        )),
        ("math", "sqrt") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.sqrt(),
        )),
        ("math", "round") => {
            let value = convert_double(arguments.first().unwrap_or(&Value::Null))?;
            match arguments.get(1) {
                Some(digits) => {
                    let digits = convert_int32(digits)?.clamp(0, 15);
                    let scale = 10f64.powi(digits);
                    Ok(Value::Double((value * scale).round_ties_even() / scale))
                }
                None => Ok(Value::Double(value.round_ties_even())),
            }
        }
        ("math", "floor") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.floor(),
        )),
        ("math", "ceiling") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.ceil(),
        )),
        ("math", "truncate") => Ok(Value::Double(
            convert_double(arguments.first().unwrap_or(&Value::Null))?.trunc(),
        )),
        ("string", "compare") => string_compare::compare(arguments, false),
        ("string", "compareordinal") => string_compare::compare(arguments, true),
        ("string", "equals") => string_compare::static_equals(arguments),
        ("string", "format") => {
            let Some(template) = arguments.first() else {
                return Err(RuntimeError::NotImplemented {
                    feature: "Format with no template",
                });
            };
            let values = Value::array(arguments[1..].to_vec());
            Ok(ops::format(template, &values)?)
        }
        ("math", "pow") => {
            let a = convert_double(arguments.first().unwrap_or(&Value::Null))?;
            let b = convert_double(arguments.get(1).unwrap_or(&Value::Null))?;
            Ok(Value::Double(a.powf(b)))
        }
        ("double", "parse") => {
            let value = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            convert_double(&Value::String(value.into())).map(Value::Double)
        }
        ("bool" | "boolean", "parse") => {
            let value = arguments
                .first()
                .map_or_else(String::new, ToString::to_string);
            Ok(Value::Bool(value.trim().eq_ignore_ascii_case("true")))
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
        ("string", "isnullorwhitespace") => {
            let empty = arguments
                .first()
                .is_none_or(|value| value.is_null_like() || value.to_string().trim().is_empty());
            Ok(Value::Bool(empty))
        }
        ("string", "concat") => Ok(Value::String(
            arguments
                .iter()
                .map(ToString::to_string)
                .collect::<String>()
                .into(),
        )),
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

fn version_value(text: String) -> Value {
    let parts: Vec<i32> = text
        .split('.')
        .map(|part| part.parse::<i32>().unwrap_or(0))
        .collect();
    Value::object(vec![
        (
            "__TypeName".to_owned(),
            Value::String("System.Version".into()),
        ),
        (
            "Major".to_owned(),
            Value::Int32(parts.first().copied().unwrap_or(0)),
        ),
        (
            "Minor".to_owned(),
            Value::Int32(parts.get(1).copied().unwrap_or(0)),
        ),
        (
            "Build".to_owned(),
            Value::Int32(parts.get(2).copied().unwrap_or(-1)),
        ),
        (
            "Revision".to_owned(),
            Value::Int32(parts.get(3).copied().unwrap_or(-1)),
        ),
        ("__Text".to_owned(), Value::String(text.into())),
    ])
}

/// `[type]value`, the subset of `LanguagePrimitives.ConvertTo` the skeleton
/// needs. Failing conversions are refused rather than approximated.
fn cast_to(type_name: &str, value: Value) -> Result<Value, RuntimeError> {
    if type_name.ends_with("[]") {
        return Ok(match value {
            Value::Array(_) => value,
            Value::ArrayList(list) => Value::array(list.items().to_vec()),
            other => Value::array(vec![other]),
        });
    }
    match type_name.to_ascii_lowercase().as_str() {
        "management.automation.actionpreference"
        | "system.management.automation.actionpreference" => preferences::cast(&value),
        "collections.arraylist" | "system.collections.arraylist" => array_list::cast(value),
        "cultureinfo" | "globalization.cultureinfo" | "system.globalization.cultureinfo" => {
            globalization::cast_culture(&value)
        }
        "stringcomparison" | "system.stringcomparison" => globalization::cast_comparison(&value),
        "text.normalizationform" | "system.text.normalizationform" => {
            globalization::cast_form(&value)
        }
        "pscustomobject" | "system.management.automation.pscustomobject" => Ok(value),
        "ordered" | "ordereddictionary" | "system.collections.specialized.ordereddictionary" => Ok(
            mark_hashtable(value, "System.Collections.Specialized.OrderedDictionary"),
        ),
        "hashtable" | "system.collections.hashtable" => {
            Ok(mark_hashtable(value, "System.Collections.Hashtable"))
        }
        "int" | "int32" | "system.int32" => Ok(Value::Int32(convert_int32(&value)?)),
        "long" | "int64" | "system.int64" => Ok(Value::Int64(convert_int64(&value)?)),
        "byte" | "system.byte" => u8::try_from(convert_int64(&value)?)
            .map(Value::Byte)
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "integer conversion overflow",
            }),
        "sbyte" | "system.sbyte" => i8::try_from(convert_int64(&value)?)
            .map(Value::SByte)
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "integer conversion overflow",
            }),
        "int16" | "short" | "system.int16" => i16::try_from(convert_int64(&value)?)
            .map(Value::Int16)
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "integer conversion overflow",
            }),
        "uint16" | "ushort" | "system.uint16" => u16::try_from(convert_int64(&value)?)
            .map(Value::UInt16)
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "integer conversion overflow",
            }),
        "uint32" | "uint" | "system.uint32" => u32::try_from(convert_int64(&value)?)
            .map(Value::UInt32)
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "integer conversion overflow",
            }),
        "uint64" | "ulong" | "system.uint64" => u64::try_from(convert_int64(&value)?)
            .map(Value::UInt64)
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "integer conversion overflow",
            }),
        "double" | "system.double" => Ok(Value::Double(convert_double(&value)?)),
        "string" | "system.string" => Ok(Value::String(value.to_ps_string())),
        "bool" | "boolean" | "system.boolean" => Ok(Value::Bool(convert_bool(&value))),
        "char" | "system.char" => convert_char(&value),
        "datetime" | "system.datetime" => {
            let text = value.to_string();
            let (year, month, day, hour, minute, second) =
                parse_datetime_text(&text).ok_or(RuntimeError::NotImplemented {
                    feature: "this DateTime input",
                })?;
            Ok(datetime_value(year, month, day, hour, minute, second))
        }
        "version" | "system.version" => Ok(version_value(value.to_string())),
        "switch" | "system.management.automation.switchparameter" => {
            Ok(Value::Bool(convert_bool(&value)))
        }
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
        Value::Object(entries) => entries
            .iter()
            .find(|(key, _)| key == "__EnumValue")
            .and_then(|(_, value)| match value {
                Value::Int64(value) => Some(*value),
                Value::Int32(value) => Some(i64::from(*value)),
                _ => None,
            })
            .ok_or(RuntimeError::NotImplemented {
                feature: "conversion to an integer",
            }),
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
        Value::Array(_) | Value::ArrayList(_) => ops::is_true(value),
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
    let left_name: String = match left {
        Value::Object(entries) => entries
            .iter()
            .find(|(key, _)| key == "__TypeName")
            .and_then(|(_, value)| match value {
                Value::String(text) => Some(text.to_string_lossy()),
                _ => None,
            })
            .unwrap_or_else(|| left.type_name().to_owned()),
        _ => left.type_name().to_owned(),
    };
    let mut same = !left.is_null_like()
        && full.as_ascii().is_some_and(|name| {
            name.eq_ignore_ascii_case(&left_name)
                || name == "System.Object" && !left.is_null_like()
                || name == "System.Array" && matches!(left, Value::Array(_))
                || matches!(
                    name,
                    "System.Collections.IList"
                        | "System.Collections.ICollection"
                        | "System.Collections.IEnumerable"
                ) && matches!(left, Value::Array(_) | Value::ArrayList(_))
        });
    if !same
        && let Value::Object(entries) = left
        && let Some((_, Value::Array(bases))) =
            entries.iter().find(|(key, _)| key == "__BaseClasses")
    {
        let wanted = full.to_string_lossy();
        same = bases
            .iter()
            .any(|base| base.to_string().eq_ignore_ascii_case(&wanted));
    }
    Ok(Value::Bool(if negate { !same } else { same }))
}

fn regex_match(
    left: &Value,
    right: &Value,
    case_sensitive: bool,
    negate: bool,
    variables: &mut Variables,
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
    let matched = matches.is_some();
    if matched {
        variables.insert("Matches".to_owned(), Value::object(object));
    }
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
    if let Value::ArrayList(list) = target {
        return array_list::method(list, name, arguments);
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
        "equals" => string_compare::instance_equals(text, arguments),
        "compareto" => string_compare::compare_to(text, arguments),
        "substring" => string_search::substring(text, arguments),
        "indexof" => string_search::index(text, arguments, false),
        "lastindexof" => string_search::index(text, arguments, true),
        "contains" => string_search::contains(text, arguments),
        "toupper" => globalization::case_method(text, "ToUpper", arguments),
        "toupperinvariant" => globalization::case_method(text, "ToUpperInvariant", arguments),
        "tolower" => globalization::case_method(text, "ToLower", arguments),
        "tolowerinvariant" => globalization::case_method(text, "ToLowerInvariant", arguments),
        "normalize" => globalization::normalization(text, "Normalize", arguments),
        "isnormalized" => globalization::normalization(text, "IsNormalized", arguments),
        "split" => {
            let needle = needle_units(arguments.first())?;
            let mut parts = Vec::new();
            let mut start = 0;
            if needle.is_empty() {
                parts.push(PsString::from_units(units.clone()));
            } else {
                while let Some(at) = find_units(&units[start..], &needle) {
                    parts.push(PsString::from_units(units[start..start + at].to_vec()));
                    start += at + needle.len();
                }
                parts.push(PsString::from_units(units[start..].to_vec()));
            }
            Ok(Value::array(parts.into_iter().map(Value::String).collect()))
        }
        "startswith" => string_search::affix(text, arguments, true),
        "endswith" => string_search::affix(text, arguments, false),
        "trim" | "trimstart" | "trimend" => {
            let mut start = 0;
            let mut end = units.len();
            if !name.eq_ignore_ascii_case("trimend") {
                while start < end {
                    let ch = char::from_u32(u32::from(units[start]))
                        .unwrap_or(char::REPLACEMENT_CHARACTER);
                    if !ch.is_whitespace() {
                        break;
                    }
                    start += 1;
                }
            }
            if !name.eq_ignore_ascii_case("trimstart") {
                while end > start {
                    let ch = char::from_u32(u32::from(units[end - 1]))
                        .unwrap_or(char::REPLACEMENT_CHARACTER);
                    if !ch.is_whitespace() {
                        break;
                    }
                    end -= 1;
                }
            }
            Ok(Value::String(PsString::from_units(
                units[start..end].to_vec(),
            )))
        }
        "padleft" | "padright" => {
            let width = int_argument(arguments.first())?;
            let fill = match arguments.get(1) {
                Some(Value::Char(unit)) => *unit,
                Some(Value::String(text)) => text.unit(0).unwrap_or(b' ' as u16),
                None => b' ' as u16,
                _ => {
                    return Err(RuntimeError::NotImplemented {
                        feature: "this PadLeft/PadRight fill",
                    });
                }
            };
            let width = usize::try_from(width).unwrap_or(0);
            if units.len() >= width {
                return Ok(Value::String(PsString::from_units(units)));
            }
            let padding = vec![fill; width - units.len()];
            let mut result = Vec::with_capacity(width);
            if name.eq_ignore_ascii_case("padleft") {
                result.extend(padding);
                result.extend(units);
            } else {
                result.extend(units);
                result.extend(padding);
            }
            Ok(Value::String(PsString::from_units(result)))
        }
        "insert" => {
            let at = arguments.first().map_or(Ok(0), convert_int32)? as usize;
            let insert = needle_units(arguments.get(1))?;
            let at = at.min(units.len());
            let mut result = Vec::with_capacity(units.len() + insert.len());
            result.extend_from_slice(&units[..at]);
            result.extend(insert);
            result.extend_from_slice(&units[at..]);
            Ok(Value::String(PsString::from_units(result)))
        }
        "remove" => {
            let at = arguments.first().map_or(Ok(0), convert_int32)? as usize;
            let count = match arguments.get(1) {
                Some(value) => convert_int32(value)? as usize,
                None => units.len().saturating_sub(at),
            };
            let at = at.min(units.len());
            let end = (at + count).min(units.len());
            let mut result = Vec::with_capacity(units.len() - (end - at));
            result.extend_from_slice(&units[..at]);
            result.extend_from_slice(&units[end..]);
            Ok(Value::String(PsString::from_units(result)))
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
    if arguments.is_empty() && name.eq_ignore_ascii_case("ToString") {
        let text_type = entries.iter().any(|(key,value)| key == "__TypeName" && matches!(value, Value::String(ty) if matches!(ty.as_ascii(), Some(
            globalization::CULTURE_TYPE | globalization::FORM_TYPE | globalization::COMPARISON_TYPE
            | preferences::TYPE | "System.Management.Automation.ErrorRecord" | "System.Management.Automation.ErrorCategoryInfo"
            | "System.Management.Automation.ErrorCategory" | "System.Management.Automation.ScopedItemOptions"
        ))));
        if text_type && let Some((_, text)) = entries.iter().find(|(key, _)| key == "__Text") {
            return Ok(text.clone());
        }
    }
    if let Some(table) = environment::table_entries(entries)
        && let [key] = arguments
        && (name.eq_ignore_ascii_case("ContainsKey") || name.eq_ignore_ascii_case("Contains"))
    {
        let key = key.to_string();
        return Ok(Value::Bool(table.iter().any(|(entry, _)| entry == &key)));
    }
    if let Some(result) = wildcards::pattern_method(entries, name, arguments) {
        return result;
    }
    if let Some(encoding) = Encoding::from_entries(entries) {
        match (name.to_ascii_lowercase().as_str(), arguments) {
            ("getpreamble", []) => {
                return Ok(Value::array(
                    encoding
                        .preamble()
                        .iter()
                        .copied()
                        .map(Value::Byte)
                        .collect(),
                ));
            }
            ("getbytes", [Value::String(text)]) => {
                let mut bytes = Vec::new();
                encoding.encode(&text.to_string_lossy(), &mut bytes);
                return Ok(Value::array(bytes.into_iter().map(Value::Byte).collect()));
            }
            ("tostring", []) => return Ok(Value::String(encoding.value().to_string().into())),
            _ => {}
        }
    }
    match name.to_ascii_lowercase().as_str() {
        "tostring"
            if entries.iter().any(|(key, value)| {
                key == "__TypeName"
                    && matches!(value, Value::String(text) if text.to_string_lossy() == "System.TimeSpan")
            }) =>
        {
            let ticks = entries
                .iter()
                .find(|(key, _)| key == "Ticks")
                .and_then(|(_, value)| convert_int64(value).ok())
                .unwrap_or(0);
            Ok(Value::String(format_timespan(ticks).into()))
        }
        "tostring"
            if entries
                .iter()
                .any(|(key, value)| {
                    key == "__TypeName"
                        && matches!(value, Value::String(text) if text.to_string_lossy() == "System.Version")
                }) =>
        {
            let text = entries
                .iter()
                .find(|(key, _)| key == "__Text")
                .map_or_else(String::new, |(_, value)| value.to_string());
            Ok(Value::String(text.into()))
        }
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
                Some(format) => {
                    format_datetime(year, month, day, hour, minute, second, format)
                }
            };
            Ok(Value::String(text.into()))
        }
        "adddays" if entries.iter().any(|(key, _)| key == "__Year") => {
            let field = |name: &str| -> i32 {
                entries
                    .iter()
                    .find(|(key, _)| key == name)
                    .and_then(|(_, value)| convert_int32(value).ok())
                    .unwrap_or(0)
            };
            let days = int_argument(arguments.first())?;
            let (year, month, day) = add_days_to_date(
                field("__Year"),
                field("__Month"),
                field("__Day"),
                i64::from(days),
            );
            Ok(datetime_value(
                year,
                month,
                day,
                field("__Hour"),
                field("__Minute"),
                field("__Second"),
            ))
        }
        "addhours" | "addminutes" | "addseconds"
            if entries.iter().any(|(key, _)| key == "__Year") =>
        {
            let field = |name: &str| -> i32 {
                entries
                    .iter()
                    .find(|(key, _)| key == name)
                    .and_then(|(_, value)| convert_int32(value).ok())
                    .unwrap_or(0)
            };
            let amount = i64::from(int_argument(arguments.first())?);
            let factor = match name.to_ascii_lowercase().as_str() {
                "addhours" => 3600,
                "addminutes" => 60,
                _ => 1,
            };
            let total = i64::from(field("__Hour")) * 3600
                + i64::from(field("__Minute")) * 60
                + i64::from(field("__Second"))
                + amount * factor;
            let day_delta = total.div_euclid(86_400);
            let seconds = total.rem_euclid(86_400);
            let (year, month, day) = add_days_to_date(
                field("__Year"),
                field("__Month"),
                field("__Day"),
                day_delta,
            );
            Ok(datetime_value(
                year,
                month,
                day,
                (seconds / 3600) as i32,
                (seconds % 3600 / 60) as i32,
                (seconds % 60) as i32,
            ))
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
    if let Some(result) = globalization::culture_property(value, name) {
        return result;
    }
    if let Value::Object(entries) = value
        && let Some(table) = environment::table_entries(entries)
    {
        if let Some((_, value)) = table.iter().find(|(key, _)| key == name) {
            return Ok(value.clone());
        }
        match name.to_ascii_lowercase().as_str() {
            "count" => return Ok(Value::Int32(table.len() as i32)),
            "keys" => {
                return Ok(Value::array(
                    table
                        .iter()
                        .map(|(key, _)| Value::String(key.as_str().into()))
                        .collect(),
                ));
            }
            "values" => return Ok(Value::array(table.iter().map(|(_, v)| v.clone()).collect())),
            _ => {}
        }
    }
    if name.eq_ignore_ascii_case("GetType") {
        return Ok(get_type(value));
    }
    if (name.eq_ignore_ascii_case("Count") || name.eq_ignore_ascii_case("Length"))
        && matches!(value, Value::Null | Value::AutomationNull)
    {
        return Ok(Value::Int32(0));
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
        Value::ArrayList(list) => array_list::property(list, name),
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
            let is_datetime = entries.iter().any(|(key, value)| {
                key == "__TypeName"
                    && matches!(value, Value::String(text) if text.to_string_lossy() == "System.DateTime")
            });
            if is_datetime {
                let field = |suffix: &str| -> i32 {
                    entries
                        .iter()
                        .find(|(key, _)| key.eq_ignore_ascii_case(suffix))
                        .and_then(|(_, value)| convert_int32(value).ok())
                        .unwrap_or(0)
                };
                let (year, month, day) = (field("__Year"), field("__Month"), field("__Day"));
                if name.eq_ignore_ascii_case("DayOfWeek") {
                    const WEEKDAYS: [&str; 7] = [
                        "Sunday",
                        "Monday",
                        "Tuesday",
                        "Wednesday",
                        "Thursday",
                        "Friday",
                        "Saturday",
                    ];
                    let index = (weekday(year, month, day).rem_euclid(7)) as usize;
                    return Ok(Value::String(WEEKDAYS[index].into()));
                }
                if name.eq_ignore_ascii_case("DayOfYear") {
                    let total: i32 = (1..month.max(1))
                        .map(|month| days_in_month(year, month))
                        .sum::<i32>()
                        + day;
                    return Ok(Value::Int32(total));
                }
                if name.eq_ignore_ascii_case("Date") {
                    return Ok(datetime_value(year, month, day, 0, 0, 0));
                }
                if name.eq_ignore_ascii_case("TimeOfDay") {
                    let ticks = i64::from(field("__Hour")) * TICKS_PER_HOUR
                        + i64::from(field("__Minute")) * TICKS_PER_MINUTE
                        + i64::from(field("__Second")) * TICKS_PER_SECOND;
                    return Ok(timespan_value(ticks));
                }
                if let Some((_, value)) = entries
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(&format!("__{name}")))
                {
                    return Ok(value.clone());
                }
            }
            let visible: Vec<&(String, Value)> =
                entries.iter().filter(|(key, _)| !is_hidden(key)).collect();
            if name.eq_ignore_ascii_case("Length") {
                return Ok(Value::Int32(1));
            }
            if name.eq_ignore_ascii_case("Count") {
                let is_dictionary = entries.iter().any(|(key, value)| {
                    key == "__TypeName"
                        && matches!(value, Value::String(text) if {
                            let name = text.to_string_lossy();
                            name.contains("Dictionary") || name.contains("Hashtable")
                        })
                });
                return Ok(Value::Int32(if is_dictionary {
                    visible.len() as i32
                } else {
                    1
                }));
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
        _ if name.eq_ignore_ascii_case("Count") || name.eq_ignore_ascii_case("Length") => {
            Ok(Value::Int32(1))
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
        let (result, out) = run("1\n2\n1 2");
        assert_eq!(
            result.unwrap_err().missing_feature(),
            Some("syntax error reporting")
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
