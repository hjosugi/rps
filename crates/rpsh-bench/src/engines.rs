//! Running a script under pwsh or rpsh, and judging rpsh's result.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rpsh_host::{EXIT_NOT_IMPLEMENTED, NOT_IMPLEMENTED_MARKER, TARGET_PWSH_VERSION};

pub(crate) struct Engine {
    pub(crate) program: PathBuf,
}

pub(crate) struct Run {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    /// `None` when the process was killed by a signal.
    pub(crate) exit_code: Option<i32>,
    pub(crate) elapsed: Duration,
}

impl Engine {
    /// Runs `script` the same way under either engine: no profile, no
    /// prompts, a fixed locale and time zone, the script's directory as the
    /// working directory.
    pub(crate) fn run_file(&self, script: &Path) -> Result<Run, String> {
        let mut command = self.command();
        command.args(["-NoProfile", "-NonInteractive", "-File"]);
        command.arg(script);
        if let Some(directory) = script.parent() {
            command.current_dir(directory);
        }
        self.run(command)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .stdin(Stdio::null())
            .env("LANG", "en_US.UTF-8")
            .env("LC_ALL", "en_US.UTF-8")
            .env("TZ", "UTC")
            // Plain text on stdout whatever $TERM is. Colored rendering
            // ($PSStyle) gets cases of its own.
            .env("NO_COLOR", "1")
            // Both would add network round trips to pwsh's run time.
            .env("POWERSHELL_TELEMETRY_OPTOUT", "1")
            .env("POWERSHELL_UPDATECHECK", "Off");
        command
    }

    fn run(&self, mut command: Command) -> Result<Run, String> {
        let start = Instant::now();
        let output = command
            .output()
            .map_err(|error| format!("cannot run {}: {error}", self.program.display()))?;
        let elapsed = start.elapsed();
        Ok(Run {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.status.code(),
            elapsed,
        })
    }
}

/// What pwsh did with a script: the answer rpsh has to reproduce.
pub(crate) struct Reference {
    pub(crate) stdout: Vec<u8>,
    pub(crate) exit_code: i32,
}

pub(crate) enum Verdict {
    /// Same stdout, same exit code.
    Match,
    /// rpsh declined to run the script, naming the missing feature.
    NotImplemented(String),
    /// rpsh ran the script and got a different result: a bug.
    Mismatch(String),
}

pub(crate) fn judge(reference: &Reference, run: &Run) -> Verdict {
    if run.exit_code == Some(EXIT_NOT_IMPLEMENTED) {
        let stderr = String::from_utf8_lossy(&run.stderr);
        if let Some(rest) = stderr.strip_prefix(NOT_IMPLEMENTED_MARKER) {
            let line = rest.lines().next().unwrap_or("");
            // "variables (at offset 12)" and "variables (at offset 40)" are
            // the same missing feature.
            let feature = line.split(" (at offset ").next().unwrap_or(line);
            return Verdict::NotImplemented(feature.to_owned());
        }
    }
    if run.exit_code != Some(reference.exit_code) {
        let actual = match run.exit_code {
            Some(code) => format!("exit code {code}"),
            None => "killed by a signal".to_owned(),
        };
        return Verdict::Mismatch(format!("{actual}, pwsh exits with {}", reference.exit_code));
    }
    if run.stdout != reference.stdout {
        let at = run
            .stdout
            .iter()
            .zip(&reference.stdout)
            .position(|(ours, theirs)| ours != theirs)
            .unwrap_or_else(|| run.stdout.len().min(reference.stdout.len()));
        return Verdict::Mismatch(format!("stdout differs from pwsh at byte {at}"));
    }
    Verdict::Match
}

/// Finds the upstream pwsh to measure against.
pub(crate) fn locate_pwsh(root: &Path, explicit: Option<String>) -> Result<Engine, String> {
    let from_env = env::var_os("RPSH_PWSH").map(PathBuf::from);
    let cached = root
        .join(".cache/pwsh")
        .join(TARGET_PWSH_VERSION)
        .join("pwsh");
    let on_path = env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join("pwsh"))
            .find(|candidate| candidate.is_file())
    });
    let program = match explicit.map(PathBuf::from).or(from_env) {
        Some(chosen) => chosen,
        None if cached.is_file() => cached,
        None => on_path.ok_or_else(|| {
            format!(
                "no pwsh found. Run scripts/fetch-pwsh.sh to install the pinned \
                 PowerShell {TARGET_PWSH_VERSION} into .cache/, or pass --pwsh PATH."
            )
        })?,
    };
    Ok(Engine { program })
}

/// The version pwsh reports for itself.
pub(crate) fn pwsh_version(pwsh: &Engine) -> Result<String, String> {
    let mut command = pwsh.command();
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "$PSVersionTable.PSVersion.ToString()",
    ]);
    let run = pwsh.run(command)?;
    let version = String::from_utf8_lossy(&run.stdout).trim().to_owned();
    if run.exit_code != Some(0) || version.is_empty() {
        return Err(format!(
            "{} did not report a PowerShell version",
            pwsh.program.display()
        ));
    }
    Ok(version)
}

/// Builds the release `rpsh` and returns it.
pub(crate) fn build_rpsh(root: &Path) -> Result<Engine, String> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "--release", "--quiet", "--package", "rpsh-cli"])
        .current_dir(root)
        .status()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !status.success() {
        return Err("building rpsh failed".to_owned());
    }
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    Ok(Engine {
        program: target.join("release").join("rpsh"),
    })
}

/// `--rpsh PATH` if given, otherwise a fresh release build.
pub(crate) fn rpsh(root: &Path, explicit: Option<String>) -> Result<Engine, String> {
    match explicit {
        Some(path) => Ok(Engine {
            program: PathBuf::from(path),
        }),
        None => build_rpsh(root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(exit_code: i32, stdout: &str, stderr: &str) -> Run {
        Run {
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
            exit_code: Some(exit_code),
            elapsed: Duration::ZERO,
        }
    }

    fn reference(exit_code: i32, stdout: &str) -> Reference {
        Reference {
            stdout: stdout.as_bytes().to_vec(),
            exit_code,
        }
    }

    #[test]
    fn same_output_and_exit_code_match() {
        assert!(matches!(
            judge(&reference(3, "2\n"), &run(3, "2\n", "")),
            Verdict::Match
        ));
    }

    #[test]
    fn a_refusal_names_the_feature_without_its_position() {
        let refused = run(99, "", "rpsh: not implemented: variables (at offset 12)\n");
        match judge(&reference(0, "2\n"), &refused) {
            Verdict::NotImplemented(feature) => assert_eq!(feature, "variables"),
            _ => panic!("expected a refusal"),
        }
    }

    #[test]
    fn exit_code_99_without_the_marker_is_an_ordinary_result() {
        // The script itself may `exit 99`.
        assert!(matches!(
            judge(&reference(99, ""), &run(99, "", "")),
            Verdict::Match
        ));
        assert!(matches!(
            judge(&reference(0, ""), &run(99, "", "")),
            Verdict::Mismatch(_)
        ));
    }

    #[test]
    fn different_output_is_a_mismatch() {
        match judge(&reference(0, "12\n"), &run(0, "13\n", "")) {
            Verdict::Mismatch(why) => assert_eq!(why, "stdout differs from pwsh at byte 1"),
            _ => panic!("expected a mismatch"),
        }
        match judge(&reference(0, "12\n"), &run(0, "12\n3\n", "")) {
            Verdict::Mismatch(why) => assert_eq!(why, "stdout differs from pwsh at byte 3"),
            _ => panic!("expected a mismatch"),
        }
    }
}
