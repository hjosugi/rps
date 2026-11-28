use std::borrow::Cow;
use std::path::{Path, PathBuf};

use rpsh_runtime::{InputEnumerator, RuntimeError, Value};
use rpsh_syntax::{
    BlockKind, Expr, ExprKind, InvocationOperator, PipelineCommand, Redirection, ScriptBlockAst,
    Statement, parse,
};

use super::{
    Engine, EngineError, Flow, FunctionDef, OutputTarget, PreparedArguments, hidden_type, member,
};

pub(super) fn single_clause<'a>(
    block: &'a ScriptBlockAst,
    command: &str,
) -> Result<&'a [Statement], RuntimeError> {
    block.single_clause().ok_or_else(|| RuntimeError::Script {
        id: match command {
            "ForEach-Object" => {
                "InvalidOperation,Microsoft.PowerShell.Commands.ForEachObjectCommand"
            }
            "Where-Object" => "InvalidOperation,Microsoft.PowerShell.Commands.WhereObjectCommand",
            _ => "InvalidOperation",
        },
        category: format!("InvalidOperation: (:) [{command}], PSInvalidOperationException"),
        exception_type: "System.Management.Automation.PSInvalidOperationException",
        message: rpsh_messages::format(
            "AutomationExceptions/ScriptBlockInvokeOnOneClauseOnly",
            &[],
        ),
    })
}

pub(super) enum CallTarget<'a> {
    Name(Cow<'a, str>),
    Cmdlet(String),
    Block(Box<ScriptCall>),
}

pub(super) struct ScriptCall {
    pub function: FunctionDef,
    pub operator: InvocationOperator,
    pub path: Option<PathBuf>,
}

impl ScriptCall {
    pub(super) fn new(function: FunctionDef, operator: InvocationOperator) -> Self {
        Self {
            function,
            operator,
            path: None,
        }
    }

    fn from_file(path: PathBuf, operator: InvocationOperator) -> Result<Self, EngineError> {
        let bytes = std::fs::read(&path)?;
        let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "script files that are not UTF-8",
            })?;
        let mut function = FunctionDef::from_script_block(parse(text)?);
        function.name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Ok(Self {
            function,
            operator,
            path: Some(path),
        })
    }
}

pub(super) fn script_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ps1"))
}

pub(super) const EXTERNAL_SCRIPT: &str = "System.Management.Automation.ExternalScriptInfo";

pub(super) fn script_info(path: PathBuf) -> Value {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let full = Value::String(path.to_string_lossy().as_ref().into());
    Value::object(vec![
        ("Name".into(), Value::String(name.as_ref().into())),
        ("__Text".into(), Value::String(name.as_ref().into())),
        ("__TypeName".into(), Value::String(EXTERNAL_SCRIPT.into())),
        (
            "__BaseClasses".into(),
            Value::array(vec![Value::String(
                "System.Management.Automation.CommandInfo".into(),
            )]),
        ),
        ("CommandType".into(), Value::String("ExternalScript".into())),
        ("Path".into(), full.clone()),
        ("Definition".into(), full.clone()),
        ("Source".into(), full),
        ("ModuleName".into(), Value::String("".into())),
        ("Visibility".into(), Value::String("Public".into())),
    ])
}

pub(super) enum StageCommand<'a> {
    Expression(&'a Expr),
    Call(CallTarget<'a>, PreparedArguments<'a>),
}

pub(super) struct PipelineStage<'a> {
    pub command: StageCommand<'a>,
    pub redirections: &'a [Redirection],
}

impl FunctionDef {
    fn from_script_block(mut block: ScriptBlockAst) -> Self {
        let body = &mut block.statements;
        let parameters = if let Some(Statement::Param { parameters, .. }) = body.first_mut() {
            let parameters = std::mem::take(parameters);
            body.remove(0);
            parameters
        } else {
            Vec::new()
        };
        Self {
            name: String::new(),
            definition: None,
            parameters,
            block: std::rc::Rc::new(block),
            filter: false,
        }
    }
}

impl Engine {
    fn bind_input(&mut self, items: Vec<Value>) -> InputEnumerator {
        let input = InputEnumerator::new(items);
        self.variables
            .insert("input".into(), Value::InputEnumerator(input.clone()));
        input
    }

    pub(super) fn call_definition(
        &mut self,
        call: &ScriptCall,
        arguments: &PreparedArguments<'_>,
        input: Option<Vec<Value>>,
        out: &mut dyn std::io::Write,
    ) -> Result<super::Flow, EngineError> {
        let function = &call.function;
        let (positional, named) = self.function_arguments(arguments, function)?;
        let child = call.operator == InvocationOperator::Ampersand;
        let mut local = if child {
            self.variables.child()
        } else {
            self.variables.clone()
        };
        if let Some(path) = &call.path {
            if child {
                local.start_script_scope();
            }
            local.set_script_path(path);
        }
        self.bind_parameters(function, &positional, &named, &mut local)?;
        let caller = std::mem::replace(&mut self.variables, local);
        let caller_trap = self.trap.take();
        let aliases = child.then(|| {
            self.functions.enter_scope();
            self.aliases.clone()
        });
        let flow = self.execute_definition_body(function, input, out);
        if let Some(aliases) = aliases {
            self.aliases = aliases;
            self.functions.leave_scope();
            self.restore_variables(caller);
        } else {
            self.variables.restore_invocation(caller);
        }
        self.trap = caller_trap;
        match flow? {
            Flow::Exit(code) if call.path.is_some() => {
                self.assign_variable("global:LASTEXITCODE", Value::Int32(code))?;
                self.last_failed = code != 0;
            }
            Flow::Return(value) => {
                if let Some(value) = value {
                    self.write_value(value, out)?;
                }
                self.last_failed = false;
            }
            Flow::Normal => self.last_failed = false,
            Flow::Break(_) | Flow::Continue(_) => {}
            flow => return Ok(flow),
        }
        Ok(Flow::Normal)
    }

    fn execute_definition_body(
        &mut self,
        function: &FunctionDef,
        input: Option<Vec<Value>>,
        out: &mut dyn std::io::Write,
    ) -> Result<Flow, EngineError> {
        if !function.filter && function.block.named_blocks.is_empty() {
            self.bind_input(input.unwrap_or_default());
            return self.exec_block(&function.block.statements, out);
        }
        self.execute_named_blocks(&function.block, function.filter, input.map(Into::into), out)
    }

    pub(super) fn execute_named_blocks(
        &mut self,
        block: &ScriptBlockAst,
        filter: bool,
        input: Option<super::input::Records>,
        out: &mut dyn std::io::Write,
    ) -> Result<Flow, EngineError> {
        let mut started = false;
        let result = self.run_pipeline_clauses(block, filter, input, out, &mut started);
        if started && let Some(clean) = block.clause(BlockKind::Clean) {
            let saved = self.capture.replace(Vec::new());
            let cleanup = self.exec_block(clean, out);
            self.capture = saved;
            match cleanup {
                Err(EngineError::Script(error)) => {
                    use std::io::Write as _;
                    let path = self.variables.get("PSCommandPath").map(ToString::to_string);
                    let source = path
                        .as_deref()
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .unwrap_or_default();
                    writeln!(
                        std::io::stderr().lock(),
                        "{}",
                        error.diagnostic(&source, path.as_deref().map(Path::new))
                    )?;
                    if let Err(EngineError::Script(primary)) = result {
                        return Err(EngineError::Script(primary.rethrow()));
                    }
                }
                Err(err) => return Err(err),
                Ok(_) => {}
            }
        }
        result
    }

    fn run_pipeline_clauses(
        &mut self,
        block: &ScriptBlockAst,
        filter: bool,
        input: Option<super::input::Records>,
        out: &mut dyn std::io::Write,
        started: &mut bool,
    ) -> Result<Flow, EngineError> {
        let begin_input = self.bind_input(Vec::new());
        if let Some(begin) = block.clause(BlockKind::Begin) {
            *started = true;
            let flow = self.run_input_clause(begin, out)?;
            if !matches!(flow, Flow::Normal) {
                return Ok(flow);
            }
        }
        let process = block.clause(BlockKind::Process).or_else(|| {
            (filter && block.named_blocks.is_empty()).then_some(block.statements.as_slice())
        });
        let mut end_input = Vec::new();
        match (process, input) {
            (Some(body), Some(items)) => {
                for item in items {
                    let item = item?;
                    *started = true;
                    begin_input.invalidate();
                    let cursor = self.bind_input(vec![item.clone()]);
                    self.variables.insert("_".into(), item);
                    let flow = self.run_input_clause(body, out)?;
                    if !matches!(flow, Flow::Normal) {
                        return Ok(flow);
                    }
                    cursor.invalidate();
                }
            }
            (Some(body), None) => {
                *started = true;
                self.bind_input(Vec::new());
                self.variables.insert("_".into(), Value::Null);
                let flow = self.run_input_clause(body, out)?;
                if !matches!(flow, Flow::Normal) {
                    return Ok(flow);
                }
            }
            (None, Some(items)) => {
                end_input = items.collect::<std::io::Result<Vec<_>>>()?;
                if !end_input.is_empty() {
                    begin_input.invalidate();
                }
            }
            (None, None) => {}
        }
        if let Some(end) = block.clause(BlockKind::End) {
            *started = true;
            self.bind_input(end_input);
            return self.run_input_clause(end, out);
        }
        Ok(Flow::Normal)
    }

    // A return completes this clause. A new record or the end clause can still run.
    fn run_input_clause(
        &mut self,
        body: &[Statement],
        out: &mut dyn std::io::Write,
    ) -> Result<Flow, EngineError> {
        match self.exec_block(body, out)? {
            Flow::Return(value) => {
                if let Some(value) = value {
                    self.write_value(value, out)?;
                }
                Ok(Flow::Normal)
            }
            flow => Ok(flow),
        }
    }

    pub(super) fn function_output(
        &mut self,
        call: &ScriptCall,
        args: &PreparedArguments<'_>,
        input: Option<Vec<Value>>,
    ) -> Result<Vec<Value>, EngineError> {
        let outer = self.capture.replace(Vec::new());
        let flow = self.call_definition(call, args, input, &mut super::capture_sink());
        let values = std::mem::replace(&mut self.capture, outer).unwrap_or_default();
        match flow? {
            super::Flow::Normal => Ok(values),
            _ => Err(RuntimeError::NotImplemented {
                feature: "exit inside a pipeline function",
            }
            .into()),
        }
    }

    pub(super) fn named_target<'a>(
        &mut self,
        name: Cow<'a, str>,
        operator: InvocationOperator,
    ) -> Result<CallTarget<'a>, EngineError> {
        let alias = self.discover_alias(&name);
        let resolved = alias.as_deref().unwrap_or(&name);
        if self.functions.contains_key(&resolved.to_ascii_lowercase()) {
            return Ok(CallTarget::Block(Box::new(ScriptCall::new(
                self.function(resolved)?,
                operator,
            ))));
        }
        if script_name(resolved) {
            let path = self
                .script_paths(resolved, false)
                .into_iter()
                .next()
                .ok_or_else(|| super::command_not_found(resolved))?;
            return Ok(CallTarget::Block(Box::new(ScriptCall::from_file(
                path, operator,
            )?)));
        }
        Ok(CallTarget::Name(name))
    }

    pub(super) fn script_paths(&mut self, name: &str, all: bool) -> Vec<PathBuf> {
        let name = name.replace('\\', "/");
        let name = name
            .strip_prefix("~/")
            .and_then(|suffix| {
                super::environment::get(&self.variables, "HOME").map(|home| {
                    Path::new(&home.to_string())
                        .join(suffix)
                        .to_string_lossy()
                        .into_owned()
                })
            })
            .unwrap_or(name);
        self.application_paths(&name, all)
    }

    pub(super) fn call_target(
        &mut self,
        target: &Expr,
        operator: InvocationOperator,
    ) -> Result<CallTarget<'static>, EngineError> {
        let block = |statements| {
            CallTarget::Block(Box::new(ScriptCall::new(
                FunctionDef::from_script_block(statements),
                operator,
            )))
        };
        if let ExprKind::ScriptBlock { block: script, .. } = &target.kind {
            return Ok(block(script.as_ref().clone()));
        }
        let value = self.eval_expression(target)?;
        if let Value::ScriptBlock(source) = &value {
            return Ok(block(parse(&source.to_string_lossy())?));
        }
        let name = match hidden_type(&value).as_deref() {
            Some(super::native::APPLICATION_TYPE | EXTERNAL_SCRIPT) => {
                member(&value, "Path")?.to_string()
            }
            Some(super::functions::CMDLET) => {
                return Ok(CallTarget::Cmdlet(member(&value, "Name")?.to_string()));
            }
            Some(super::functions::FUNCTION) => {
                let function = self.function(&member(&value, "Name")?.to_string())?;
                return Ok(CallTarget::Block(Box::new(ScriptCall::new(
                    function, operator,
                ))));
            }
            Some(super::aliases::TYPE_NAME) => {
                let definition = member(&value, "Definition")?;
                if definition.is_null_like() {
                    value.to_string()
                } else {
                    definition.to_string()
                }
            }
            _ => value.to_ps_string().to_string_lossy(),
        };
        if name.is_empty() {
            return Err(RuntimeError::Script {
                id: "BadExpression",
                category: "InvalidOperation: (:) [], RuntimeException".into(),
                exception_type: "System.Management.Automation.RuntimeException",
                message: rpsh_messages::format("ParserStrings/BadExpression", &[operator.symbol()]),
            }
            .into());
        }
        self.named_target(name.into(), operator)
    }

    pub(super) fn pipeline_stage<'a>(
        &mut self,
        stage: &'a PipelineCommand,
    ) -> Result<PipelineStage<'a>, EngineError> {
        let (command, redirections) = match stage {
            PipelineCommand::Redirected {
                command,
                redirections,
            } => (command.as_ref(), redirections.as_slice()),
            command => (command, &[][..]),
        };
        let command = match command {
            PipelineCommand::Expression(expr) => StageCommand::Expression(expr),
            PipelineCommand::Command { name, arguments } => StageCommand::Call(
                self.named_target(name.as_str().into(), InvocationOperator::Ampersand)?,
                self.prepare_arguments(arguments)?,
            ),
            PipelineCommand::Call {
                operator,
                target,
                arguments,
            } => StageCommand::Call(
                self.call_target(target, *operator)?,
                self.prepare_arguments(arguments)?,
            ),
            PipelineCommand::Redirected { .. } => {
                return Err(RuntimeError::NotImplemented {
                    feature: "nested redirections",
                }
                .into());
            }
        };
        Ok(PipelineStage {
            command,
            redirections,
        })
    }

    pub(super) fn run_object_stage(
        &mut self,
        command: &StageCommand<'_>,
        input: Vec<Value>,
        has_upstream: bool,
        output: Option<&OutputTarget>,
    ) -> Result<Vec<Value>, EngineError> {
        match command {
            StageCommand::Expression(expr) => Ok(super::input::pipeline_values(
                self.eval_pipeline_expression(expr)?,
            )?),
            StageCommand::Call(CallTarget::Name(name), arguments) => {
                self.run_command(name, arguments, &input, has_upstream, output)
            }
            StageCommand::Call(CallTarget::Cmdlet(name), arguments) => {
                self.cmdlet_output(name, arguments, &input)
            }
            StageCommand::Call(CallTarget::Block(function), arguments) => {
                self.function_output(function, arguments, has_upstream.then_some(input))
            }
        }
    }

    pub(super) fn cmdlet_output(
        &mut self,
        name: &str,
        arguments: &PreparedArguments<'_>,
        input: &[Value],
    ) -> Result<Vec<Value>, EngineError> {
        self.invoke_builtin(name, arguments, input)?.ok_or_else(|| {
            RuntimeError::NotImplemented {
                feature: "invocation of this cmdlet",
            }
            .into()
        })
    }
}
