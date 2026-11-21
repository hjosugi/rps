//! Peaks and CPU time from `wait4`, through `scripts/spawn-measure.c`.
//!
//! The workspace forbids `unsafe`, and `std` has no portable `wait4`. The
//! helper is already used by `scripts/time-commands.py`; `cargo harness
//! bench` compiles it into `.cache/tools/` on first use, as that script does.
//! It forks and execs the target, so the target's peak RSS is its own and not
//! the harness's.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Resource usage `wait4` reports for one run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Metrics {
    pub(crate) user: Duration,
    pub(crate) system: Duration,
    /// Peak resident set size in KiB (`ru_maxrss` on Linux).
    pub(crate) max_rss_kib: u64,
}

/// What the helper saw while running the target.
pub(crate) struct Measured {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    /// `None` when the target was killed by a signal.
    pub(crate) exit_code: Option<i32>,
    pub(crate) elapsed: Duration,
    pub(crate) metrics: Metrics,
}

pub(crate) struct Measurer {
    helper: PathBuf,
    reports: PathBuf,
    next_report: AtomicU64,
}

impl Measurer {
    /// Compiles `scripts/spawn-measure.c` if it is missing or older than the
    /// source, then prepares a directory for per-run report files.
    pub(crate) fn new(root: &Path) -> Result<Self, String> {
        let source = root.join("scripts/spawn-measure.c");
        let tools = root.join(".cache/tools");
        let helper = tools.join("spawn-measure");
        let stale = match (fs::metadata(&helper), fs::metadata(&source)) {
            (Ok(binary), Ok(source)) => match (binary.modified(), source.modified()) {
                (Ok(binary), Ok(source)) => binary < source,
                _ => true,
            },
            _ => true,
        };
        if stale {
            fs::create_dir_all(&tools)
                .map_err(|error| format!("cannot create {}: {error}", tools.display()))?;
            let compiler = std::env::var_os("CC").unwrap_or_else(|| OsString::from("cc"));
            let output = Command::new(&compiler)
                .arg("-O2")
                .arg("-o")
                .arg(&helper)
                .arg(&source)
                .output()
                .map_err(|error| {
                    format!(
                        "cannot run {}: {error} (the bench needs a C compiler for wait4; set CC)",
                        compiler.to_string_lossy()
                    )
                })?;
            if !output.status.success() {
                return Err(format!(
                    "{} failed to compile {}:\n{}",
                    compiler.to_string_lossy(),
                    source.display(),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
        let reports = tools.join("bench-reports");
        fs::create_dir_all(&reports)
            .map_err(|error| format!("cannot create {}: {error}", reports.display()))?;
        Ok(Self {
            helper,
            reports,
            next_report: AtomicU64::new(0),
        })
    }

    /// Runs `command` as the helper's child and returns what both saw.
    pub(crate) fn run(&self, command: Command) -> Result<Measured, String> {
        let program = resolve_program(command.get_program(), &command)?;
        let report_path = self.reports.join(format!(
            "{}-{}.report",
            std::process::id(),
            self.next_report.fetch_add(1, Ordering::Relaxed)
        ));

        let mut helper = Command::new(&self.helper);
        helper.arg("--path").arg(&report_path).arg(&program);
        helper.args(command.get_args());
        helper.stdin(Stdio::null());
        // The caller's environment is the measured child's environment.
        helper.env_clear();
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => {
                    helper.env(key, value);
                }
                None => {
                    helper.env_remove(key);
                }
            }
        }
        if let Some(directory) = command.get_current_dir() {
            helper.current_dir(directory);
        }

        let output = helper
            .output()
            .map_err(|error| format!("cannot run {}: {error}", self.helper.to_string_lossy()))?;
        if !output.status.success() {
            let _ = fs::remove_file(&report_path);
            return Err(format!(
                "{} failed to measure {}:\n{}",
                self.helper.to_string_lossy(),
                program.to_string_lossy(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let text = fs::read_to_string(&report_path)
            .map_err(|error| format!("cannot read {}: {error}", report_path.display()))?;
        let _ = fs::remove_file(&report_path);
        let parsed = parse_report(&text).ok_or_else(|| {
            format!(
                "spawn-measure wrote an unreadable report for {}: {text:?}",
                program.to_string_lossy()
            )
        })?;

        Ok(Measured {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: parsed.exit_code,
            elapsed: parsed.elapsed,
            metrics: parsed.metrics,
        })
    }
}

struct Parsed {
    elapsed: Duration,
    exit_code: Option<i32>,
    metrics: Metrics,
}

/// `<wall ns> <user us> <sys us> <max rss KiB> <minor faults> <major faults>
/// <wait status>`, as `scripts/spawn-measure.c` writes it.
fn parse_report(text: &str) -> Option<Parsed> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    if fields.len() != 7 {
        return None;
    }
    let wall_ns: u64 = fields[0].parse().ok()?;
    let user_us: u64 = fields[1].parse().ok()?;
    let system_us: u64 = fields[2].parse().ok()?;
    let max_rss_kib: u64 = fields[3].parse().ok()?;
    let _minor_faults: u64 = fields[4].parse().ok()?;
    let _major_faults: u64 = fields[5].parse().ok()?;
    let status: i32 = fields[6].parse().ok()?;
    let exit_code = if status & 0x7f == 0 {
        Some((status >> 8) & 0xff)
    } else {
        None
    };
    Some(Parsed {
        elapsed: Duration::from_nanos(wall_ns),
        exit_code,
        metrics: Metrics {
            user: Duration::from_micros(user_us),
            system: Duration::from_micros(system_us),
            max_rss_kib,
        },
    })
}

/// `execv` does not search `PATH`; resolve a bare program name the way
/// `Command::spawn` would, using the environment the child will get.
fn resolve_program(program: &OsStr, command: &Command) -> Result<OsString, String> {
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() > 1 {
        return Ok(program.to_owned());
    }
    let search = command
        .get_envs()
        .find(|(key, _)| *key == OsStr::new("PATH"))
        .and_then(|(_, value)| value.map(OsStr::to_owned))
        .or_else(|| std::env::var_os("PATH"));
    if let Some(search) = search {
        for directory in std::env::split_paths(&search) {
            let candidate = directory.join(program);
            if candidate.is_file() {
                return Ok(candidate.into_os_string());
            }
        }
    }
    Err(format!(
        "cannot run {}: not found on PATH",
        program.to_string_lossy()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_report() {
        let parsed = parse_report("1500000 700 300 4096 12 0 0\n").unwrap();
        assert_eq!(parsed.elapsed, Duration::from_micros(1500));
        assert_eq!(parsed.exit_code, Some(0));
        assert_eq!(parsed.metrics.user, Duration::from_micros(700));
        assert_eq!(parsed.metrics.system, Duration::from_micros(300));
        assert_eq!(parsed.metrics.max_rss_kib, 4096);
    }

    #[test]
    fn decodes_exit_codes_and_signals() {
        // Exit 42: status = 42 << 8.
        assert_eq!(
            parse_report("0 0 0 0 0 0 10752").unwrap().exit_code,
            Some(42)
        );
        // SIGKILL (9) in the low bits.
        assert_eq!(parse_report("0 0 0 0 0 0 9").unwrap().exit_code, None);
    }

    #[test]
    fn rejects_a_truncated_report() {
        assert!(parse_report("1 2 3").is_none());
        assert!(parse_report("x 2 3 4 5 6 7").is_none());
    }
}
