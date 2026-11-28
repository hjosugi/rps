use std::borrow::Cow;
use std::io::{self, Write};
use std::process::Command;

use rpsh_commands::{CORE_CMDLETS, MODULES};
use rpsh_native::{Destination, PipelineError, run_pipeline};
use rpsh_runtime::{RuntimeError, Value};
use rpsh_syntax::{CommandArgument, Expr, PipelineCommand, Redirection, Stream};

use super::{
    Engine, EngineError, OutputTarget, command_not_found, evaluate, expand_glob, native_exit_code,
};

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

impl Engine {
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
        input: &[Value],
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
        input: &[Value],
        destination: NativeOutput<'_>,
    ) -> Result<Vec<Value>, EngineError> {
        let mut commands = Vec::with_capacity(stages.len());
        for stage in stages {
            let mut command = Command::new(stage.name.as_ref());
            command.args(self.native_arguments(stage.arguments)?);
            for (key, value) in &self.variables {
                if let Some(name) = key.strip_prefix("env:") {
                    if value.is_null_like() {
                        command.env_remove(name);
                    } else {
                        command.env(name, value.to_string());
                    }
                }
            }
            commands.push(command);
        }
        let mut bytes = Vec::new();
        for value in input {
            writeln!(bytes, "{value}")?;
        }
        let mut captured = Vec::new();
        let output = match destination {
            NativeOutput::Objects => Destination::Writer(&mut captured as &mut dyn Write),
            NativeOutput::Host(writer) => Destination::Writer(writer),
            NativeOutput::Redirected(target) => Destination::Device(target.stdio()?),
        };
        let status = run_pipeline(commands, &bytes, output).map_err(|error| match error {
            PipelineError::Spawn { program, error } if error.kind() == io::ErrorKind::NotFound => {
                EngineError::from(command_not_found(&program.to_string_lossy()))
            }
            PipelineError::Spawn { .. } => RuntimeError::NotImplemented {
                feature: "starting a native command",
            }
            .into(),
            PipelineError::Io(error) => EngineError::Io(error),
        })?;
        self.last_failed = !status.all_succeeded;
        self.assign_variable(
            "global:LASTEXITCODE",
            Value::Int32(native_exit_code(status.last)),
        );
        Ok(native_lines(&captured))
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
                    let value = evaluate(expr, &mut self.variables, !self.last_failed)?;
                    out.push(value.to_string());
                }
                CommandArgument::Parameter { name, value } => {
                    out.push(format!("-{name}"));
                    if let Some(expr) = value {
                        let value = evaluate(expr, &mut self.variables, !self.last_failed)?;
                        out.push(value.to_string());
                    }
                }
            }
        }
        Ok(out)
    }
}

// StreamReader.ReadLine recognizes CR, LF, and CRLF and does not synthesize
// an empty record for a trailing line terminator.
fn native_lines(bytes: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(bytes);
    let mut remaining = text.as_ref();
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
