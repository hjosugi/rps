use super::{Engine, EngineError, ErrorFiles, OutputTarget, native_exit_code};
use crate::{ScriptException, errors::record_with_exception, preferences::Action};
use rpsh_native::{FailedStage, PipelineStatus};
use rpsh_runtime::{RuntimeError, Value};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

const EXCEPTION: &str = "System.Management.Automation.NativeCommandExitException";

fn record(path: &Path, stage: &FailedStage) -> Value {
    let code = native_exit_code(stage.status);
    let program = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let message = rpsh_messages::format(
        "CommandBaseStrings/ProgramExitedWithNonZeroCode",
        &[&program, &code.to_string(), &format!("0x{code:02X}")],
    );
    let target = Value::String(path.to_string_lossy().as_ref().into());
    let exception = Value::object(vec![
        ("__TypeName".into(), Value::String(EXCEPTION.into())),
        ("Message".into(), Value::String(message.as_str().into())),
        ("ExitCode".into(), Value::Int32(code)),
        ("ProcessId".into(), Value::Int32(stage.pid as i32)),
        ("Path".into(), target.clone()),
        ("InnerException".into(), Value::Null),
    ]);
    record_with_exception(
        "ProgramExitedWithNonZeroCode",
        &format!("NotSpecified: ({target}:String) [], NativeCommandExitException"),
        exception,
        &message,
        target,
    )
}

impl Engine {
    pub(super) fn native_exit_status(
        &mut self,
        status: PipelineStatus,
        policy: Option<(Action, Vec<PathBuf>)>,
        error_files: &ErrorFiles,
    ) -> Result<(), EngineError> {
        self.last_failed = !status.all_succeeded;
        if let Some((action, paths)) = policy {
            for stage in &status.failures {
                self.assign_variable(
                    "global:LASTEXITCODE",
                    Value::Int32(native_exit_code(stage.status)),
                )?;
                let path = &paths[stage.index];
                let error = record(path, stage);
                match action {
                    Action::Stop => return Err(EngineError::Script(ScriptException::new(error))),
                    Action::Inquire => self.inquire_native(path, &error)?,
                    Action::Silent | Action::Continue | Action::Break => {}
                    Action::Ignore | Action::Suspend => {
                        unreachable!("disabled or unassignable preference")
                    }
                }
                self.remember_error(&error);
                if action != Action::Silent {
                    match error_files.target(stage.index) {
                        Some(OutputTarget::Discard) => {}
                        Some(OutputTarget::File(file)) => {
                            writeln!(&*file, "NativeCommandExitException: {error}")?
                        }
                        None => {
                            writeln!(io::stderr().lock(), "NativeCommandExitException: {error}")?
                        }
                    }
                }
            }
        }
        self.assign_variable(
            "global:LASTEXITCODE",
            Value::Int32(native_exit_code(status.last)),
        )
    }

    fn inquire_native(&self, path: &Path, record: &Value) -> Result<(), EngineError> {
        let failure = |why: &str| {
            let program = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            RuntimeError::Script {
                id: "NativeCommandFailed",
                exception_type: "System.Management.Automation.ApplicationFailedException",
                category: "ResourceUnavailable: (:) [], ApplicationFailedException".into(),
                message: format!("Program '{program}' failed to run: {why}"),
            }
        };
        if self.non_interactive || !self.console_stdio {
            return Err(failure("PowerShell is in NonInteractive mode. Read and Prompt functionality is not available.").into());
        }
        let stdin = io::stdin();
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "\nConfirm\n{record}")?;
        loop {
            write!(
                stdout,
                "[Y] Yes  [A] Yes to All  [H] Halt Command  [S] Suspend  [?] Help \n(default is \"Y\"):"
            )?;
            stdout.flush()?;
            let mut answer = String::new();
            if stdin.read_line(&mut answer)? == 0 {
                return Err(
                    failure("Object reference not set to an instance of an object.").into(),
                );
            }
            let answer = answer.trim_end_matches(['\r', '\n']);
            if !stdin.is_terminal() {
                writeln!(stdout, "{answer}")?;
            }
            match answer.trim().to_ascii_lowercase().as_str() {
                "" | "y" | "a" => return Ok(()),
                "h" => return Err(RuntimeError::Script {
                    id: "ActionPreferenceStop",
                    category: "OperationStopped: (:) [], ActionPreferenceStopException".into(),
                    exception_type: "System.Management.Automation.ActionPreferenceStopException",
                    message:
                        "The running command stopped because the user selected the Stop option."
                            .into(),
                }
                .into()),
                "s" => {
                    return Err(RuntimeError::NotImplemented {
                        feature: "nested console prompts",
                    }
                    .into());
                }
                "?" => writeln!(
                    stdout,
                    "Y - Continue with only the next step of the operation.\nA - Continue with all the steps of the operation.\nH - Stop this command.\nS - Pause the current pipeline and return to the command prompt. Type \"exit\" to\n resume the pipeline."
                )?,
                _ => {}
            }
        }
    }
}
