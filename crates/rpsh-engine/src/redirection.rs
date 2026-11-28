use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::process::Stdio;

use rpsh_runtime::{RuntimeError, Value};
use rpsh_syntax::{PipelineCommand, Redirection, Stream};

use super::{
    Engine, EngineError, error_record, expand_glob, flatten_value, format_out_string, hidden_type,
};

pub(super) enum NativeOutput<'a> {
    Objects,
    Host(&'a mut dyn Write),
    Redirected(&'a OutputTarget),
}

/// Native stdout goes directly to the file. PowerShell values use Out-File's
/// formatting and UTF-8 encoding before they reach the same destination.
pub(super) enum OutputTarget {
    File(File),
    Discard,
}

impl OutputTarget {
    fn open(path: Value, append: bool) -> Result<Self, EngineError> {
        if path.is_null_like() {
            return Ok(Self::Discard);
        }
        let text = path.to_string();
        let mut path = if let Some((provider, tail)) = text.split_once("::")
            && (provider.eq_ignore_ascii_case("FileSystem")
                || provider.eq_ignore_ascii_case("Microsoft.PowerShell.Core\\FileSystem"))
        {
            tail.to_owned()
        } else if let Some((drive, tail)) = text.split_once(':')
            && !drive.contains(['/', '\\'])
            && !(cfg!(windows) && drive.len() == 1)
        {
            if !drive.eq_ignore_ascii_case("Temp") {
                return Err(RuntimeError::NotImplemented {
                    feature: "redirection to this PowerShell drive",
                }
                .into());
            }
            std::env::temp_dir()
                .join(tail.trim_start_matches(['/', '\\']))
                .to_string_lossy()
                .into_owned()
        } else {
            text
        };
        if (path == "~" || path.starts_with("~/"))
            && let Some(home) = std::env::var_os("HOME")
        {
            path = std::path::Path::new(&home)
                .join(path.get(2..).unwrap_or(""))
                .to_string_lossy()
                .into_owned();
        }
        if path.contains(['*', '?', '[']) {
            let matches = expand_glob(&path);
            let [only] = matches.as_slice() else {
                return Err(RuntimeError::NotImplemented {
                    feature: "redirection wildcard resolving to other than one file",
                }
                .into());
            };
            path.clone_from(only);
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(append)
            .truncate(!append)
            .open(path)
            .map_err(|_| RuntimeError::NotImplemented {
                feature: "redirection to an unwritable file",
            })?;
        Ok(Self::File(file))
    }

    pub(super) fn stdio(&self) -> io::Result<Stdio> {
        match self {
            Self::Discard => Ok(Stdio::null()),
            Self::File(file) => Ok(Stdio::from(file.try_clone()?)),
        }
    }

    fn write_values(&self, values: &[Value]) -> io::Result<()> {
        if let Self::File(file) = self
            && !values.is_empty()
        {
            let mut writer = file;
            let width = if std::env::var_os("TERM").is_some_and(|term| !term.is_empty()) {
                80
            } else {
                i32::MAX as usize
            };
            writer.write_all(format_out_string(values, width).as_bytes())?;
        }
        Ok(())
    }
}

impl Engine {
    pub(super) fn run_redirected(
        &mut self,
        command: &PipelineCommand,
        input: Vec<Value>,
        redirections: &[Redirection],
    ) -> Result<Vec<Value>, EngineError> {
        let stream = match command {
            PipelineCommand::Command { name, .. } => {
                let resolved = self.discover_alias(name);
                let name = resolved.as_deref().unwrap_or(name).to_ascii_lowercase();
                if self.functions.contains_key(&name) {
                    None
                } else {
                    match name.as_str() {
                        "write-error" => Some(Stream::Error),
                        "write-warning" => Some(Stream::Warning),
                        "write-verbose" => Some(Stream::Verbose),
                        "write-debug" => Some(Stream::Debug),
                        "write-information" => Some(Stream::Information),
                        _ => None,
                    }
                }
            }
            PipelineCommand::Expression(_) => None,
            PipelineCommand::Redirected { .. } => return Err(unsupported_stream().into()),
        };
        let mut file = None;
        let mut merged = None;
        for redirect in redirections {
            match redirect {
                Redirection::File {
                    from: Stream::Output,
                    append,
                    path,
                } => {
                    file = Some((path, *append));
                }
                Redirection::MergeOutput { from } if Some(*from) == stream => {
                    merged = Some(*from);
                }
                _ => return Err(unsupported_stream().into()),
            }
        }
        if stream.is_some() && merged.is_none() {
            return Err(unsupported_stream().into());
        }
        let output = match file {
            Some((path, append)) => Some(OutputTarget::open(self.eval_expression(path)?, append)?),
            None => None,
        };
        let mut values = match command {
            PipelineCommand::Expression(expr) => {
                flatten_value(self.eval_pipeline_expression(expr)?)
            }
            PipelineCommand::Command { name, arguments } => {
                self.run_command(name, arguments, input, output.as_ref())?
            }
            PipelineCommand::Redirected { .. } => return Err(unsupported_stream().into()),
        };
        if let Some(stream) = merged {
            values = values
                .into_iter()
                .map(|value| stream_record(stream, value))
                .collect();
        }
        if let Some(target) = output {
            target.write_values(&values)?;
            Ok(Vec::new())
        } else {
            Ok(values)
        }
    }
}

fn stream_record(stream: Stream, value: Value) -> Value {
    let kind = match stream {
        Stream::Warning => "WarningRecord",
        Stream::Verbose => "VerboseRecord",
        Stream::Debug => "DebugRecord",
        Stream::Information => "InformationRecord",
        Stream::Error => {
            let text = value.to_string();
            return error_record(
                "Microsoft.PowerShell.Commands.WriteErrorException",
                "NotSpecified: (:) [Write-Error], WriteErrorException",
                "Microsoft.PowerShell.Commands.WriteErrorException",
                &text,
                &text,
            );
        }
        Stream::Output | Stream::All => return value,
    };
    let property = if stream == Stream::Information {
        "MessageData"
    } else {
        "Message"
    };
    Value::object(vec![
        (
            "__TypeName".into(),
            Value::String(format!("System.Management.Automation.{kind}").into()),
        ),
        ("__Text".into(), Value::String(value.to_string().into())),
        (property.into(), value),
    ])
}

pub(super) fn record_prefix(value: &Value) -> Option<&'static str> {
    match hidden_type(value)?.as_str() {
        "System.Management.Automation.WarningRecord" => Some("WARNING: "),
        "System.Management.Automation.VerboseRecord" => Some("VERBOSE: "),
        "System.Management.Automation.DebugRecord" => Some("DEBUG: "),
        "System.Management.Automation.InformationRecord" => Some(""),
        _ => None,
    }
}

fn unsupported_stream() -> RuntimeError {
    RuntimeError::NotImplemented {
        feature: "this combination of command and redirected streams",
    }
}
