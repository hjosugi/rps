use std::ffi::OsString;
use std::io::{self, Write};
use std::process::{Child, Command, ExitStatus, Stdio};

/// A host writer or object decoder reads the last pipe. A file or null
/// device is attached directly to the last process.
#[derive(Debug)]
pub enum Destination<W> {
    Writer(W),
    Device(Stdio),
}

#[derive(Debug)]
pub struct PipelineStatus {
    pub last: ExitStatus,
    pub all_succeeded: bool,
}

#[derive(Debug)]
pub enum PipelineError {
    Spawn { program: OsString, error: io::Error },
    Io(io::Error),
}

impl From<io::Error> for PipelineError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Reap children even if starting a later stage, writing to the host, or
/// waiting for a process fails. Dropping std::process::Child does not reap it.
#[derive(Default)]
struct Children(Vec<Child>);

impl Children {
    fn finish(&mut self) -> io::Result<PipelineStatus> {
        let mut last = None;
        let mut all_succeeded = true;
        for child in &mut self.0 {
            let status = child.wait()?;
            all_succeeded &= status.success();
            last = Some(status);
        }
        self.0.clear();
        Ok(PipelineStatus {
            last: last.ok_or_else(|| io::Error::other("empty native pipeline"))?,
            all_succeeded,
        })
    }

    fn stop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
        }
        for mut child in self.0.drain(..) {
            let _ = child.wait();
        }
    }
}

impl Drop for Children {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start all stages before pumping input or waiting. Only the first process
/// reads object input and only the last process can write into the host.
pub fn run_pipeline<W: Write>(
    commands: Vec<Command>,
    input: &[u8],
    destination: Destination<W>,
) -> Result<PipelineStatus, PipelineError> {
    run_pipeline_with_input(commands, Some(input), destination)
}

/// None preserves the process's stdin. An empty object pipeline supplies
/// EOF and a nonempty one is pumped while the last child's output drains.
pub fn run_pipeline_with_input<W: Write>(
    commands: Vec<Command>,
    input: Option<&[u8]>,
    destination: Destination<W>,
) -> Result<PipelineStatus, PipelineError> {
    let count = commands.len();
    let (mut last_stdout, mut writer) = match destination {
        Destination::Writer(writer) => (Some(Stdio::piped()), Some(writer)),
        Destination::Device(device) => (Some(device), None),
    };
    let mut children = Children::default();
    let mut first_stdin = None;
    let mut reader = None;
    for (index, mut command) in commands.into_iter().enumerate() {
        let stdin = match reader.take() {
            Some(pipe) => Stdio::from(pipe),
            None => match input {
                None => Stdio::inherit(),
                Some([]) => Stdio::null(),
                Some(_) => Stdio::piped(),
            },
        };
        let stdout = if index + 1 == count {
            last_stdout.take().expect("the last stage owns its stdout")
        } else {
            Stdio::piped()
        };
        let mut child = command
            .stdin(stdin)
            .stdout(stdout)
            .spawn()
            .map_err(|error| PipelineError::Spawn {
                program: command.get_program().to_owned(),
                error,
            })?;
        if index == 0 {
            first_stdin = child.stdin.take();
        }
        reader = child.stdout.take();
        children.0.push(child);
    }

    std::thread::scope(|scope| {
        // A child may fill its stdout before accepting the rest of its stdin.
        let feeder = first_stdin
            .zip(input)
            .map(|(mut sink, bytes)| scope.spawn(move || sink.write_all(bytes)));
        let result = (|| {
            if let (Some(mut source), Some(writer)) = (reader, writer.as_mut()) {
                io::copy(&mut source, writer)?;
            }
            children.finish()
        })();
        if result.is_err() {
            children.stop();
        }
        if let Some(feeder) = feeder {
            let written = feeder
                .join()
                .map_err(|_| io::Error::other("native stdin writer panicked"))?;
            // A consumer such as head can exit without reading its whole input.
            if let Err(error) = written
                && error.kind() != io::ErrorKind::BrokenPipe
            {
                return Err(error.into());
            }
        }
        Ok(result?)
    })
}
