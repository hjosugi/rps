mod glob;
mod path;
pub(super) use path::LookupPaths;

use super::encoding::Encoding;
use super::{encoding, format_out_string, redirection::text_output_width};
use std::borrow::Cow;
use std::io::Write;
use std::process::{Command, Stdio};

use rpsh_commands::{CORE_CMDLETS, MODULES};
use rpsh_native::{Destination, PipelineError, run_pipeline_with_input};
use rpsh_runtime::{RuntimeError, Value};
use rpsh_syntax::{
    ArgumentSeparator, CommandArgument, Expr, ExprKind, PipelineCommand, Redirection, Stream,
};

use super::{Engine, EngineError, OutputTarget, command_not_found, native_exit_code};

pub(super) enum NativeOutput<'a> {
    Objects,
    Host(&'a mut dyn Write),
    Redirected(&'a OutputTarget),
}

pub(super) struct NativeStage<'a> {
    name: Cow<'a, str>,
    arguments: &'a [CommandArgument],
    pub(super) file: Option<(&'a Expr, bool)>,
}

pub(super) const APPLICATION_TYPE: &str = "System.Management.Automation.ApplicationInfo";

pub(super) fn application_info(path: std::path::PathBuf) -> Value {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .map_or_else(String::new, |ext| format!(".{}", ext.to_string_lossy()));
    let full = Value::String(path.to_string_lossy().as_ref().into());
    Value::object(vec![
        ("Name".into(), Value::String(name.as_ref().into())),
        ("__TypeName".into(), Value::String(APPLICATION_TYPE.into())),
        ("__Text".into(), Value::String(name.as_ref().into())),
        ("CommandType".into(), Value::String("Application".into())),
        ("Path".into(), full.clone()),
        ("Source".into(), full.clone()),
        ("Definition".into(), full),
        ("Extension".into(), Value::String(extension.into())),
        ("Version".into(), super::version_value("0.0.0.0".into())),
        ("ModuleName".into(), Value::String("".into())),
        ("Visibility".into(), Value::String("Public".into())),
    ])
}

impl Engine {
    pub(super) fn application_paths(&mut self, name: &str, all: bool) -> Vec<std::path::PathBuf> {
        let path = super::environment::search_path(&self.variables);
        self.command_paths.update(&path, || {
            super::environment::get(&self.variables, "HOME").map(|v| v.to_string())
        });
        self.command_paths.find(name, all)
    }

    pub(super) fn native_stage<'a>(&self, stage: &'a PipelineCommand) -> Option<NativeStage<'a>> {
        let (stage, file) = match stage {
            PipelineCommand::Redirected {
                command,
                redirections,
            } => {
                let [
                    Redirection::File {
                        from: Stream::Output,
                        append,
                        path,
                    },
                ] = redirections.as_slice()
                else {
                    return None;
                };
                (command.as_ref(), Some((path, *append)))
            }
            stage => (stage, None),
        };
        let PipelineCommand::Command { name, arguments } = stage else {
            return None;
        };
        let resolved = self
            .aliases
            .definition(name)
            .map_or(Cow::Borrowed(name.as_str()), Cow::Owned);
        let lower = resolved.to_ascii_lowercase();
        if self.functions.contains_key(&lower)
            || CORE_CMDLETS.binary_search(&lower.as_str()).is_ok()
            || MODULES.iter().any(|module| {
                (module.exports_cmdlet(&lower) || module.exports_alias(&resolved))
                    && (self.builtin_loaded(module) || self.autoload_unqualified())
            })
        {
            return None;
        }
        Some(NativeStage {
            name: resolved,
            arguments,
            file,
        })
    }

    pub(super) fn run_native(
        &mut self,
        name: &str,
        arguments: &[CommandArgument],
        input: Option<&[Value]>,
        destination: NativeOutput<'_>,
    ) -> Result<Vec<Value>, EngineError> {
        let stage = NativeStage {
            name: Cow::Borrowed(name),
            arguments,
            file: None,
        };
        self.run_native_stages(&[stage], input, destination)
    }

    pub(super) fn run_native_stages(
        &mut self,
        stages: &[NativeStage<'_>],
        input: Option<&[Value]>,
        destination: NativeOutput<'_>,
    ) -> Result<Vec<Value>, EngineError> {
        let mut commands = Vec::with_capacity(stages.len());
        for stage in stages {
            let path = self
                .application_paths(&stage.name, false)
                .into_iter()
                .next()
                .ok_or_else(|| command_not_found(&stage.name))?;
            let mut command = Command::new(path);
            command.args(self.native_arguments(stage.arguments)?);
            super::environment::configure_child(&mut command, &self.variables);
            commands.push(command);
        }
        let mut bytes = Vec::new();
        if let Some(values) = input {
            let encoding = self
                .variables
                .get("OutputEncoding")
                .and_then(Encoding::from_value)
                .unwrap_or(encoding::UTF8);
            bytes.extend_from_slice(encoding.preamble());
            encoding.encode(&format_out_string(values, text_output_width()), &mut bytes);
        }
        let mut captured = Vec::new();
        let output = match destination {
            NativeOutput::Objects => Destination::Writer(&mut captured as &mut dyn Write),
            NativeOutput::Host(writer) if self.console_stdio => {
                writer.flush()?;
                Destination::Device(Stdio::inherit())
            }
            NativeOutput::Host(writer) => Destination::Writer(writer),
            NativeOutput::Redirected(target) => Destination::Device(target.stdio()?),
        };
        let stdin = (input.is_some() || !self.console_stdio).then_some(bytes.as_slice());
        let status =
            run_pipeline_with_input(commands, stdin, output).map_err(|error| match error {
                PipelineError::Spawn { program, error } => {
                    let path = std::path::Path::new(&program);
                    let file = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                    let directory = std::env::current_dir().unwrap_or_default();
                    let reason = error.to_string();
                    let reason = reason.split(" (os error ").next().unwrap_or(&reason);
                    RuntimeError::Script {
                        id: "NativeCommandFailed",
                        category: "ResourceUnavailable: (:) [], ApplicationFailedException".into(),
                        exception_type: "System.Management.Automation.ApplicationFailedException",
                        message: format!("Program '{file}' failed to run: An error occurred trying to start process '{}' with working directory '{}'. {reason}",path.display(),directory.display()),
                    }.into()
                }
                PipelineError::Io(error) => EngineError::Io(error),
            })?;
        self.last_failed = !status.all_succeeded;
        self.assign_variable(
            "global:LASTEXITCODE",
            Value::Int32(native_exit_code(status.last)),
        )?;
        if captured.is_empty() {
            return Ok(Vec::new());
        }
        let encoding = self
            .variables
            .get("PSApplicationOutputEncoding")
            .and_then(Encoding::from_value)
            .unwrap_or_else(encoding::console);
        Ok(native_lines(&encoding.decode(&captured)))
    }

    fn native_arguments(
        &mut self,
        arguments: &[CommandArgument],
    ) -> Result<Vec<String>, EngineError> {
        let mut out = Vec::new();
        for argument in arguments {
            match argument {
                CommandArgument::Bareword(word) => glob::append(&mut out, word.clone(), false),
                CommandArgument::Value(expr) => self.native_argument_value(expr, &mut out)?,
                CommandArgument::Parameter {
                    name,
                    value,
                    separator,
                } => {
                    if *separator == ArgumentSeparator::Space {
                        out.push(format!("-{name}"));
                        if let Some(expr) = value {
                            self.native_argument_value(expr, &mut out)?;
                        }
                    } else if let Some(expr) = value {
                        let mut values = Vec::new();
                        self.native_argument_value(expr, &mut values)?;
                        for value in values {
                            if *separator == ArgumentSeparator::Colon {
                                out.push(format!("-{name}:{value}"));
                            } else {
                                out.push(format!("-{name}:"));
                                out.push(value);
                            }
                        }
                    } else {
                        out.push(format!("-{name}:"));
                    }
                }
            }
        }
        Ok(out)
    }

    fn native_argument_value(
        &mut self,
        expr: &Expr,
        out: &mut Vec<String>,
    ) -> Result<(), EngineError> {
        let quoted = matches!(
            expr.kind,
            ExprKind::String(_) | ExprKind::ExpandableString(_)
        );
        let value = self.eval_expression(expr)?;
        let items = match &value {
            Value::Array(items) => items.as_slice(),
            value => std::slice::from_ref(value),
        };
        for item in items.iter().filter(|item| !item.is_null_like()) {
            glob::append(out, item.to_string(), quoted);
        }
        Ok(())
    }
}

// StreamReader.ReadLine recognizes CR, LF, and CRLF and does not synthesize
// an empty record for a trailing line terminator.
fn native_lines(mut remaining: &str) -> Vec<Value> {
    let mut values = Vec::new();
    while let Some(index) = remaining.find(['\r', '\n']) {
        values.push(Value::String(remaining[..index].into()));
        let suffix = &remaining[index..];
        remaining = &suffix[if suffix.starts_with("\r\n") { 2 } else { 1 }..];
    }
    if !remaining.is_empty() {
        values.push(Value::String(remaining.into()));
    }
    values
}
