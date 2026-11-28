//! PowerShell's merged native streams are lines, with an ErrorRecord for
//! each stderr line. Read both descriptors concurrently, including while
//! the child is still running and the terminal or output file is consuming.
use super::{
    Engine, EngineError, NativeOutput, OutputTarget, error_files::read_lines, pipeline_error,
};
use crate::{encoding::Encoding, errors::record_with_exception};
use rpsh_native::{Destination, PipelineStatus, run_pipeline_with_input};
use rpsh_runtime::Value;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::process::Command;
use std::sync::mpsc;
use std::thread;

pub(super) enum TextOutput<'a> {
    Objects {
        values: Vec<Value>,
        first_error: bool,
    },
    File(BufWriter<&'a File>),
    Host(&'a mut dyn Write),
    Discard,
}

impl<'a> TextOutput<'a> {
    pub(super) fn new(output: NativeOutput<'a>) -> Self {
        match output {
            NativeOutput::Objects => Self::Objects {
                values: Vec::new(),
                first_error: true,
            },
            NativeOutput::Host(writer) => Self::Host(writer),
            NativeOutput::Redirected(OutputTarget::File(file)) => Self::File(BufWriter::new(file)),
            NativeOutput::Redirected(OutputTarget::Discard) => Self::Discard,
        }
    }

    pub(super) fn line(
        &mut self,
        engine: &mut Engine,
        is_error: bool,
        line: String,
    ) -> Result<(), EngineError> {
        match self {
            Self::Objects {
                values,
                first_error,
            } => values.push(if is_error {
                stderr_record(&line, std::mem::take(first_error))
            } else {
                Value::String(line.into())
            }),
            Self::File(file) => writeln!(file, "{line}")?,
            Self::Host(writer) => {
                engine.write_host(*writer, format_args!("{line}\n"))?;
                writer.flush()?;
            }
            Self::Discard => {}
        }
        Ok(())
    }

    pub(super) fn exit_error(
        &mut self,
        engine: &mut Engine,
        record: Value,
    ) -> Result<(), EngineError> {
        match self {
            Self::Objects { values, .. } => values.push(record),
            Self::File(file) => writeln!(file, "{EXIT_PREFIX}{record}")?,
            Self::Host(writer) => engine.write_value(record, *writer)?,
            Self::Discard => {}
        }
        Ok(())
    }

    pub(super) fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::File(file) => file.flush(),
            Self::Host(writer) => writer.flush(),
            _ => Ok(()),
        }
    }

    pub(super) fn finish(mut self) -> Result<Vec<Value>, EngineError> {
        self.flush()?;
        match self {
            Self::Objects { values, .. } => Ok(values),
            _ => Ok(Vec::new()),
        }
    }
}

const EXIT_PREFIX: &str = "NativeCommandExitException: ";

pub(crate) fn error_prefix(value: &Value) -> Option<&'static str> {
    let Value::Object(fields) = value else {
        return None;
    };
    let field = |key| {
        fields.iter().find_map(|(name, value)| match value {
            Value::String(text) if name == key => text.as_ascii(),
            _ => None,
        })
    };
    if field("__TypeName")? != "System.Management.Automation.ErrorRecord"
        || field("__Format").is_some()
    {
        return None;
    }
    match field("FullyQualifiedErrorId")? {
        "NativeCommandError" | "NativeCommandErrorMessage" => Some(""),
        "ProgramExitedWithNonZeroCode" => Some(EXIT_PREFIX),
        _ => None,
    }
}

fn stderr_record(message: &str, first: bool) -> Value {
    let text = Value::String(message.into());
    let exception = Value::object(vec![
        (
            "__TypeName".into(),
            Value::String("System.Management.Automation.RemoteException".into()),
        ),
        ("Message".into(), text.clone()),
        ("InnerException".into(), Value::Null),
    ]);
    if first {
        record_with_exception(
            "NativeCommandError",
            &format!("NotSpecified: ({message}:String) [], RemoteException"),
            exception,
            message,
            text,
        )
    } else {
        record_with_exception(
            "NativeCommandErrorMessage",
            "NotSpecified: (:) [], RemoteException",
            exception,
            message,
            Value::Null,
        )
    }
}

pub(super) fn run(
    mut commands: Vec<Command>,
    stdin: Option<&[u8]>,
    encoding: Encoding,
    merge_error: bool,
    mut emit: impl FnMut(bool, String) -> Result<(), EngineError>,
) -> Result<PipelineStatus, EngineError> {
    let (stdout, writer) = io::pipe()?;
    let mut streams = vec![(false, stdout)];
    if merge_error {
        let (stderr, writer) = io::pipe()?;
        let command = commands
            .last_mut()
            .ok_or_else(|| io::Error::other("empty native pipeline"))?;
        command.stderr(writer);
        streams.push((true, stderr));
    }
    thread::scope(move |scope| {
        let (send, receive) = mpsc::sync_channel(16);
        let mut readers = Vec::new();
        for (is_error, pipe) in streams {
            let send = send.clone();
            let reader = thread::Builder::new().spawn_scoped(scope, move || {
                read_lines(pipe, encoding, |line| {
                    send.send((is_error, line.to_owned()))
                        .map_err(|_| io::ErrorKind::BrokenPipe.into())
                })
            })?;
            readers.push(reader);
        }
        drop(send);
        let process = thread::Builder::new().spawn_scoped(scope, move || {
            run_pipeline_with_input(
                commands,
                stdin,
                Destination::<io::Sink>::Device(writer.into()),
            )
        })?;
        for (is_error, line) in receive {
            emit(is_error, line)?;
        }
        let status = process
            .join()
            .map_err(|_| io::Error::other("native pipeline panicked"))?
            .map_err(pipeline_error)?;
        for reader in readers {
            reader
                .join()
                .map_err(|_| io::Error::other("native line reader panicked"))??;
        }
        Ok(status)
    })
}
