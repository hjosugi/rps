mod arguments;
mod error_files;
mod exit_error;
mod glob;
mod path;
mod text_output;
pub(super) use path::LookupPaths;
pub(super) use text_output::error_prefix;

use super::encoding::Encoding;
use super::{encoding, format_out_string, redirection::text_output_width};
use error_files::ErrorFiles;
use std::borrow::Cow;
use std::io::{self, Write};
use std::path::Path;
use std::process::Stdio;

use rpsh_commands::{CORE_CMDLETS, MODULES};
use rpsh_native::{Destination, PipelineError, application_command, run_pipeline_with_input};
use rpsh_runtime::{RuntimeError, Value};
use rpsh_syntax::{Redirection, Stream};

use super::{
    CallTarget, Engine, EngineError, OutputTarget, PipelineStage, PreparedArguments, StageCommand,
    command_not_found, native_exit_code,
};

pub(super) enum NativeOutput<'a> {
    Objects,
    Host(&'a mut dyn Write),
    Redirected(&'a OutputTarget),
}

pub(super) struct NativeStage<'a> {
    name: Cow<'a, str>,
    arguments: &'a PreparedArguments<'a>,
    redirections: &'a [Redirection],
}

impl NativeStage<'_> {
    pub(super) fn pipes_bytes(&self) -> bool {
        !self.redirections.iter().any(|r| {
            matches!(
                r,
                Redirection::File {
                    from: Stream::All | Stream::Output,
                    ..
                } | Redirection::MergeOutput {
                    from: Stream::Error | Stream::All
                }
            )
        })
    }
}

#[derive(Default)]
struct NativeRedirection {
    output: Option<OutputTarget>,
    error: Option<OutputTarget>,
    merge_error: bool,
    text_output: bool,
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

    pub(super) fn native_stage<'a>(&self, stage: &'a PipelineStage<'_>) -> Option<NativeStage<'a>> {
        let StageCommand::Call(CallTarget::Name(name), arguments) = &stage.command else {
            return None;
        };
        let resolved = self
            .aliases
            .definition(name)
            .map_or(Cow::Borrowed(name.as_ref()), Cow::Owned);
        let lower = super::command_name(&resolved).to_ascii_lowercase();
        if super::invocation::script_name(&resolved)
            || self.functions.contains_key(&lower)
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
            redirections: stage.redirections,
        })
    }

    fn native_redirection(
        &mut self,
        redirects: &[Redirection],
    ) -> Result<NativeRedirection, EngineError> {
        let mut result = NativeRedirection::default();
        let mut all = None;
        for redirect in redirects {
            match redirect {
                Redirection::File { from, path, append } => {
                    let target = OutputTarget::open(self.eval_expression(path)?, *append)?;
                    match from {
                        Stream::All => all = Some(target),
                        Stream::Output => result.output = Some(target),
                        Stream::Error => result.error = Some(target),
                        _ => {} // Native processes do not write PowerShell's other streams.
                    }
                }
                Redirection::MergeOutput {
                    from: Stream::All | Stream::Error,
                } => result.merge_error = true,
                Redirection::MergeOutput { .. } => {}
            }
        }
        if let Some(all) = all {
            if result.output.is_none() {
                result.output = Some(all);
                result.text_output = true;
                result.merge_error |= result.error.is_none();
            } else if result.error.is_none() && !result.merge_error {
                result.error = Some(all);
            }
        }
        Ok(result)
    }

    pub(super) fn run_native(
        &mut self,
        name: &str,
        arguments: &PreparedArguments<'_>,
        input: Option<&[Value]>,
        destination: NativeOutput<'_>,
    ) -> Result<Vec<Value>, EngineError> {
        let stage = NativeStage {
            name: Cow::Borrowed(name),
            arguments,
            redirections: &[],
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
        let mut error_files = ErrorFiles::default();
        let mut last_redirection = NativeRedirection::default();
        for (index, stage) in stages.iter().enumerate() {
            let path = self
                .application_paths(&stage.name, false)
                .into_iter()
                .next()
                .ok_or_else(|| command_not_found(&stage.name))?;
            let args = self.native_arguments(stage.arguments, &path);
            let mut command = application_command(
                &path,
                &args,
                super::environment::child_overrides(&self.variables),
            )
            .map_err(|err| spawn_failure(&path, err))?;
            let mut redirection = self.native_redirection(stage.redirections)?;
            if !redirection.merge_error
                && let Some(target) = redirection.error.take()
            {
                error_files.attach(&mut command, index, target)?;
            }
            last_redirection = redirection;
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
        let destination = match (&last_redirection.output, destination) {
            (Some(target), _) | (None, NativeOutput::Redirected(target)) => {
                NativeOutput::Redirected(target)
            }
            (None, NativeOutput::Host(writer)) => NativeOutput::Host(writer),
            (None, NativeOutput::Objects) => NativeOutput::Objects,
        };
        let stdin = (input.is_some() || !self.console_stdio).then_some(bytes.as_slice());
        let policy = crate::preferences::native_action(&self.variables)
            .filter(|action| *action != crate::preferences::Action::Ignore)
            .map(|action| {
                (
                    action,
                    commands
                        .iter()
                        .map(|command| Path::new(command.get_program()).to_owned())
                        .collect(),
                )
            });
        if last_redirection.merge_error || last_redirection.text_output {
            let encoding = self.native_output_encoding();
            let mut sink = text_output::TextOutput::new(destination);
            let merged_stage = last_redirection.merge_error.then_some(stages.len() - 1);
            let run = || {
                text_output::run(
                    commands,
                    stdin,
                    encoding,
                    merged_stage.is_some(),
                    |is_error, line| sink.line(self, is_error, line),
                )
            };
            let status = error_files.run(run, || encoding)?;
            sink.flush()?;
            if let Some(error) =
                self.native_exit_status(status, policy, &error_files, merged_stage)?
            {
                sink.exit_error(self, error)?;
            }
            return sink.finish();
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
        let run = || run_pipeline_with_input(commands, stdin, output).map_err(pipeline_error);
        let status = error_files.run(run, || self.native_output_encoding())?;
        self.native_exit_status(status, policy, &error_files, None)?;
        if captured.is_empty() {
            return Ok(Vec::new());
        }
        Ok(native_lines(
            &self.native_output_encoding().decode(&captured),
        ))
    }

    fn native_output_encoding(&self) -> Encoding {
        self.variables
            .get("PSApplicationOutputEncoding")
            .and_then(Encoding::from_value)
            .unwrap_or_else(encoding::console)
    }
}

fn pipeline_error(error: PipelineError) -> EngineError {
    match error {
        PipelineError::Spawn { program, error } => spawn_failure(Path::new(&program), error),
        PipelineError::Io(error) => EngineError::Io(error),
    }
}

fn spawn_failure(path: &Path, error: io::Error) -> EngineError {
    let file = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let directory = std::env::current_dir().unwrap_or_default();
    let reason = error.to_string();
    let reason = reason.split(" (os error ").next().unwrap_or(&reason);
    RuntimeError::Script {
        id: "NativeCommandFailed",
        category: "ResourceUnavailable: (:) [], ApplicationFailedException".into(),
        exception_type: "System.Management.Automation.ApplicationFailedException",
        message: format!("Program '{file}' failed to run: An error occurred trying to start process '{}' with working directory '{}'. {reason}", path.display(), directory.display()),
    }.into()
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
