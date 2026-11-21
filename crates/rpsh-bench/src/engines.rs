//! Running a script under pwsh or rpsh, and judging rpsh's result.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rpsh_host::{EXIT_NOT_IMPLEMENTED, NOT_IMPLEMENTED_MARKER};

use crate::rusage::{Measurer, Metrics};

pub(crate) struct Engine {
    pub(crate) program: PathBuf,
    /// `$HOME` for every run: `.cache/home` in the checkout, never the
    /// user's own. pwsh reads modules, `powershell.config.json` and its
    /// startup JIT profile from below `$HOME`.
    home: PathBuf,
}

/// `PATH` for every run, so that a case names the same programs on every
/// machine (docs/01_compat_contract.md, "Reference environment").
const REFERENCE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

pub(crate) struct Run {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    /// `None` when the process was killed by a signal.
    pub(crate) exit_code: Option<i32>,
    pub(crate) elapsed: Duration,
    /// `None` for a plain run; `Some` when the run went through `wait4`.
    pub(crate) metrics: Option<Metrics>,
}

impl Engine {
    fn new(root: &Path, program: PathBuf) -> Result<Self, String> {
        let home = root.join(".cache/home");
        fs::create_dir_all(&home)
            .map_err(|error| format!("cannot create {}: {error}", home.display()))?;
        Ok(Self { program, home })
    }

    /// Runs `script` the same way under either engine: no profile, no
    /// prompts, the reference environment, the script's directory as the
    /// working directory.
    pub(crate) fn run_file(&self, script: &Path) -> Result<Run, String> {
        let command = self.file_command(script);
        self.run(command)
    }

    /// Like [`Engine::run_file`], but records CPU time and peak RSS through
    /// `wait4` (`scripts/spawn-measure.c`).
    pub(crate) fn run_file_measured(
        &self,
        script: &Path,
        measurer: &Measurer,
    ) -> Result<Run, String> {
        let command = self.file_command(script);
        let measured = measurer.run(command)?;
        Ok(Run {
            stdout: measured.stdout,
            stderr: measured.stderr,
            exit_code: measured.exit_code,
            elapsed: measured.elapsed,
            metrics: Some(measured.metrics),
        })
    }

    fn file_command(&self, script: &Path) -> Command {
        let mut command = self.command();
        command.args(["-NoProfile", "-NonInteractive", "-File"]);
        command.arg(script);
        if let Some(directory) = script.parent() {
            command.current_dir(directory);
        }
        command
    }

    /// The reference environment of docs/01_compat_contract.md. Nothing is
    /// inherited from the caller: `TERM` alone changes `$Host.UI.RawUI` and
    /// `[Console]::WindowWidth`, and `PSModulePath` or `XDG_*` change what
    /// pwsh loads.
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .stdin(Stdio::null())
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", REFERENCE_PATH)
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
            metrics: None,
        })
    }
}

/// What pwsh did with a script: the answer rpsh has to reproduce.
pub(crate) struct Reference {
    pub(crate) stdout: Vec<u8>,
    /// `Some` when the comparison includes stderr (conformance level C2).
    pub(crate) stderr: Option<Vec<u8>>,
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

fn first_difference(ours: &[u8], theirs: &[u8]) -> usize {
    ours.iter()
        .zip(theirs)
        .position(|(ours, theirs)| ours != theirs)
        .unwrap_or_else(|| ours.len().min(theirs.len()))
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
        let at = first_difference(&run.stdout, &reference.stdout);
        return Verdict::Mismatch(format!("stdout differs from pwsh at byte {at}"));
    }
    if let Some(expected) = &reference.stderr
        && run.stderr != *expected
    {
        let at = first_difference(&run.stderr, expected);
        return Verdict::Mismatch(format!("stderr differs from pwsh at byte {at}"));
    }
    Verdict::Match
}

/// Finds the upstream pwsh to measure against.
///
/// The reference is the latest stable release that `scripts/fetch-pwsh.sh`
/// installed. `.cache/pwsh/current` names it; if the pointer is missing, the
/// newest versioned directory in `.cache/pwsh/` is used.
pub(crate) fn locate_pwsh(root: &Path, explicit: Option<String>) -> Result<Engine, String> {
    let from_env = env::var_os("RPSH_PWSH").map(PathBuf::from);
    let cached = installed_pwsh(root);
    let on_path = env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join("pwsh"))
            .find(|candidate| candidate.is_file())
    });
    let program = match explicit.map(PathBuf::from).or(from_env) {
        Some(chosen) => chosen,
        None => cached.or(on_path).ok_or_else(|| {
            "no pwsh found. Run scripts/fetch-pwsh.sh to install the latest stable \
             PowerShell into .cache/, or pass --pwsh PATH."
                .to_owned()
        })?,
    };
    Engine::new(root, program)
}

fn installed_pwsh(root: &Path) -> Option<PathBuf> {
    let cache = root.join(".cache/pwsh");
    if let Ok(text) = fs::read_to_string(cache.join("current")) {
        let version = text.trim().trim_start_matches('v');
        let candidate = cache.join(version).join("pwsh");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for entry in fs::read_dir(&cache).ok()? {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let Some(key) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(version_key)
        else {
            continue;
        };
        let candidate = path.join("pwsh");
        if candidate.is_file() && best.as_ref().is_none_or(|(best, _)| key > *best) {
            best = Some((key, candidate));
        }
    }
    best.map(|(_, path)| path)
}

fn version_key(name: &str) -> Option<Vec<u64>> {
    let name = name.strip_prefix('v').unwrap_or(name);
    let parts: Vec<u64> = name
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (!parts.is_empty()).then_some(parts)
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

/// What the goldens depend on besides the pwsh version: the runtime, the
/// system ICU that supplies culture data, and the host's idea of the
/// console. Written to `tests/conformance/reference.txt` by `record`.
pub(crate) fn pwsh_reference(pwsh: &Engine) -> Result<String, String> {
    const PROBE: &str = r#"
$icu = Get-Content "/proc/$PID/maps" -ErrorAction Ignore |
    ForEach-Object { if ($_ -match 'libicuuc\.so\.([0-9.]+)') { $Matches[1] } } |
    Select-Object -First 1
$size = $Host.UI.RawUI.WindowSize
"powershell: $($PSVersionTable.PSVersion)"
"dotnet: $([Environment]::Version)"
"os: $($PSVersionTable.OS)"
"architecture: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)"
"icu: $(if ($icu) { $icu } else { 'not loaded' })"
"culture: $((Get-Culture).Name) / $((Get-UICulture).Name)"
"time zone: $([TimeZoneInfo]::Local.Id)"
"window: $($size.Width)x$($size.Height)"
"#;
    let mut command = pwsh.command();
    command.args(["-NoProfile", "-NonInteractive", "-Command", PROBE]);
    let run = pwsh.run(command)?;
    if run.exit_code != Some(0) {
        return Err(format!(
            "{} failed to describe its environment: {}",
            pwsh.program.display(),
            String::from_utf8_lossy(&run.stderr)
        ));
    }
    String::from_utf8(run.stdout).map_err(|_| "pwsh described its environment in non-UTF-8".into())
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
    Engine::new(root, target.join("release").join("rpsh"))
}

/// `--rpsh PATH` if given, otherwise a fresh release build.
pub(crate) fn rpsh(root: &Path, explicit: Option<String>) -> Result<Engine, String> {
    match explicit {
        Some(path) => Engine::new(root, PathBuf::from(path)),
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
            metrics: None,
        }
    }

    fn reference(exit_code: i32, stdout: &str) -> Reference {
        Reference {
            stdout: stdout.as_bytes().to_vec(),
            stderr: None,
            exit_code,
        }
    }

    #[test]
    fn version_directories_sort_numerically() {
        assert!(version_key("7.10.0") > version_key("7.9.0"));
        assert_eq!(version_key("v7.7.0"), Some(vec![7, 7, 0]));
        assert_eq!(version_key("current"), None);
        assert_eq!(version_key("7.7.0-preview.1"), None);
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
