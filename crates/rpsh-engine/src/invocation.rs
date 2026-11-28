use std::borrow::Cow;

use rpsh_runtime::{RuntimeError, Value};
use rpsh_syntax::{Expr, ExprKind, PipelineCommand, Redirection, Statement, parse};

use super::{
    Engine, EngineError, FunctionDef, OutputTarget, PreparedArguments, flatten_value, hidden_type,
    member,
};

pub(super) enum CallTarget<'a> {
    Name(Cow<'a, str>),
    Cmdlet(String),
    Block(Box<FunctionDef>),
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
    fn from_script_block(mut body: Vec<Statement>) -> Self {
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
            begin: Vec::new(),
            process: Vec::new(),
            end: Vec::new(),
            body,
            filter: false,
        }
    }
}

impl Engine {
    pub(super) fn call_target(
        &mut self,
        target: &Expr,
    ) -> Result<CallTarget<'static>, EngineError> {
        if let ExprKind::ScriptBlock { statements, .. } = &target.kind {
            return Ok(CallTarget::Block(Box::new(FunctionDef::from_script_block(
                statements.clone(),
            ))));
        }
        let value = self.eval_expression(target)?;
        let name = match &value {
            Value::ScriptBlock(source) => {
                let script = parse(&source.to_string_lossy())?;
                return Ok(CallTarget::Block(Box::new(FunctionDef::from_script_block(
                    script.statements,
                ))));
            }
            _ => match hidden_type(&value).as_deref() {
                Some(super::native::APPLICATION_TYPE) => member(&value, "Path")?.to_string(),
                Some(super::functions::CMDLET) => {
                    return Ok(CallTarget::Cmdlet(member(&value, "Name")?.to_string()));
                }
                Some(super::functions::FUNCTION) => {
                    return Ok(CallTarget::Block(Box::new(
                        self.function(&member(&value, "Name")?.to_string())?,
                    )));
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
            },
        };
        if name.is_empty() {
            return Err(RuntimeError::Script {
                id: "BadExpression",
                category: "InvalidOperation: (:) [], RuntimeException".into(),
                exception_type: "System.Management.Automation.RuntimeException",
                message: rpsh_messages::format("ParserStrings/BadExpression", &["&"]),
            }
            .into());
        }
        Ok(CallTarget::Name(name.into()))
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
                CallTarget::Name(name.as_str().into()),
                self.prepare_arguments(arguments)?,
            ),
            PipelineCommand::Call { target, arguments } => StageCommand::Call(
                self.call_target(target)?,
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
        output: Option<&OutputTarget>,
    ) -> Result<Vec<Value>, EngineError> {
        match command {
            StageCommand::Expression(expr) => {
                Ok(flatten_value(self.eval_pipeline_expression(expr)?))
            }
            StageCommand::Call(CallTarget::Name(name), arguments) => {
                self.run_command(name, arguments, input, output)
            }
            StageCommand::Call(CallTarget::Cmdlet(name), arguments) => {
                self.cmdlet_output(name, arguments, &input)
            }
            StageCommand::Call(CallTarget::Block(function), arguments) => {
                self.function_output(function, arguments, input)
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
