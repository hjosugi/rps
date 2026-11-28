//! A redirected native error stream is decoded into lines the way PowerShell's
//! process reader does. Native stdout can still go directly to a file or pipe.
use super::super::{EngineError, OutputTarget, encoding::Encoding};
use rpsh_native::PipelineStatus;
use std::io::{self, BufWriter, PipeReader, Read, Write};
use std::process::{Command, Stdio};

struct RedirectedError {
    stage: usize,
    target: OutputTarget,
    reader: Option<PipeReader>,
}

#[derive(Default)]
pub(super) struct ErrorFiles(Vec<RedirectedError>);

impl ErrorFiles {
    pub(super) fn attach(
        &mut self,
        command: &mut Command,
        stage: usize,
        target: OutputTarget,
    ) -> io::Result<()> {
        let reader = match &target {
            OutputTarget::Discard => {
                command.stderr(Stdio::null());
                None
            }
            OutputTarget::File(_) => {
                let (reader, writer) = io::pipe()?;
                command.stderr(writer);
                Some(reader)
            }
        };
        self.0.push(RedirectedError {
            stage,
            target,
            reader,
        });
        Ok(())
    }

    pub(super) fn target(&self, stage: usize) -> Option<&OutputTarget> {
        self.0
            .iter()
            .find(|entry| entry.stage == stage)
            .map(|entry| &entry.target)
    }

    pub(super) fn run(
        &mut self,
        start: impl FnOnce() -> Result<PipelineStatus, EngineError>,
        encoding: impl FnOnce() -> Encoding,
    ) -> Result<PipelineStatus, EngineError> {
        if !self.0.iter().any(|entry| entry.reader.is_some()) {
            return start();
        }
        let encoding = encoding();
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for entry in &mut self.0 {
                if let (Some(reader), OutputTarget::File(file)) =
                    (entry.reader.take(), &entry.target)
                {
                    let worker = std::thread::Builder::new().spawn_scoped(scope, move || {
                        let mut file = BufWriter::new(file);
                        read_lines(reader, encoding, |line| writeln!(file, "{line}"))?;
                        file.flush()
                    });
                    match worker {
                        Ok(worker) => workers.push(worker),
                        // The unstarted commands still own the write ends of
                        // previously prepared pipes. Close them before joining.
                        Err(error) => {
                            drop(start);
                            return Err(error.into());
                        }
                    }
                }
            }
            let result = start();
            for worker in workers {
                worker
                    .join()
                    .map_err(|_| io::Error::other("native stderr reader panicked"))??;
            }
            result
        })
    }
}

/// Delimiters are single bytes in UTF-8 and aligned units in UTF-16/32.
/// Keep only the open line and a partial code unit between reads.
pub(super) fn read_lines(
    mut source: impl Read,
    encoding: Encoding,
    mut line: impl FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    let width = encoding.unit_width();
    let mut buffer = [0u8; 8192];
    let mut pending = Vec::new();
    let mut scanned = 0;
    let mut after_cr = false;
    loop {
        let n = match source.read(&mut buffer) {
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..n]);
        let complete = pending.len() / width * width;
        let mut start = 0;
        for at in (scanned..complete).step_by(width) {
            let unit = &pending[at..at + width];
            let delimiter = encoding.newline(unit);
            if after_cr && delimiter == Some(b'\n') {
                after_cr = false;
                start = at + width;
                continue;
            }
            after_cr = delimiter == Some(b'\r');
            if delimiter.is_some() {
                line(&encoding.decode(&pending[start..at]))?;
                start = at + width;
            }
        }
        pending.drain(..start);
        scanned = complete - start;
    }
    if !pending.is_empty() {
        line(&encoding.decode(&pending))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpsh_runtime::Value;

    struct ShortReads<'a> {
        bytes: &'a [u8],
        size: usize,
    }

    impl Read for ShortReads<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let size = self.size.min(out.len());
            self.bytes.read(&mut out[..size])
        }
    }

    #[test]
    fn split_delimiters_and_characters_survive_short_reads() {
        for page in [20127, 28591, 65001, 1200, 1201, 12000, 12001] {
            let enc = Encoding::by_name(&Value::Int32(page)).unwrap();
            let mut bytes = Vec::new();
            enc.encode("A\r\n\rB\n\nC\u{0d00}🙂", &mut bytes);
            let width = enc.unit_width();
            if width > 1 {
                bytes.push(0xff);
            }
            let expected = enc.decode(&bytes[7 * width..]);
            for size in [1, 2, 3, 5, 9] {
                let mut lines = Vec::new();
                read_lines(
                    ShortReads {
                        bytes: &bytes,
                        size,
                    },
                    enc,
                    |s| {
                        lines.push(s.to_owned());
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(
                    lines,
                    ["A", "", "B", "", expected.as_ref()],
                    "{page}, {size}"
                );
            }
        }
    }
}
