//! `cargo harness bench --suite shells`: startup of other shells.
//!
//! Two measurements per shell: a one-line run (`<shell> -c exit` or its
//! equivalent) through `wait4`, and the time to the first output on a
//! pseudo-terminal, which is how long an interactive prompt takes to appear.
//! A shell that is not installed is recorded as skipped, not omitted.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::Args;
use crate::bench;
use crate::engines;
use crate::json::Json;
use crate::rusage::Measurer;
use crate::stats::{self, Sample, Summary};

const REFERENCE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

pub(crate) fn run(root: &Path, mut args: Args) -> Result<bool, String> {
    let runs = args.number("--runs")?.unwrap_or(10);
    let warmup = args.number("--warmup")?.unwrap_or(2);
    let filter = args.value("--filter")?;
    let out = args.value("--out")?;
    let pwsh_path = args.value("--pwsh")?;
    let rpsh_path = args.value("--rpsh")?;
    args.finish()?;
    if runs == 0 {
        return Err("--runs must be at least 1".to_owned());
    }

    let home = root.join(".cache/home");
    fs::create_dir_all(&home)
        .map_err(|error| format!("cannot create {}: {error}", home.display()))?;
    let measurer = Measurer::new(root)?;
    let pwsh = engines::locate_pwsh(root, pwsh_path)?;
    let pwsh_version = engines::pwsh_version(&pwsh)?;
    let rpsh = engines::rpsh(root, rpsh_path)?;

    let mut candidates = candidates(&pwsh.program, &rpsh.program);
    if let Some(filter) = filter {
        candidates.retain(|candidate| candidate.name.contains(&filter));
    }
    if candidates.is_empty() {
        return Err("no shell matches".to_owned());
    }

    let before = bench::load_average();
    let mut results = Vec::new();
    for candidate in &candidates {
        eprint!("{} ... ", candidate.name);
        let result = measure(candidate, root, &measurer, runs, warmup);
        match (&result.one_line, &result.interactive) {
            (Ok(one), Some(Ok(interactive))) => {
                eprintln!(
                    "{} / {}",
                    millis(one.median_ms),
                    millis(interactive.median_ms)
                );
            }
            (Ok(one), _) => eprintln!("{}", millis(one.median_ms)),
            (Err(error), _) => eprintln!("FAILED: {error}"),
        }
        results.push(result);
    }
    let after = bench::load_average();

    print!("{}", table(&results));
    let document = Json::Object(vec![
        ("schema", Json::text("rpsh-shells-v1")),
        ("generated_at_unix_s", Json::Number(bench::unix_seconds())),
        (
            "pwsh",
            Json::Object(vec![
                ("version", Json::text(pwsh_version)),
                ("policy", Json::text(rpsh_host::REFERENCE_POLICY)),
            ]),
        ),
        ("environment", bench::environment(before, after)),
        (
            "settings",
            Json::Object(vec![
                ("runs", Json::Number(runs as f64)),
                ("warmup", Json::Number(warmup as f64)),
            ]),
        ),
        (
            "shells",
            Json::Array(results.iter().map(result_json).collect()),
        ),
    ]);
    let out = match out {
        Some(path) => Path::new(&path).to_path_buf(),
        None => root.join("bench/results/shells-latest.json"),
    };
    bench::write_json(&out, &document)?;
    eprintln!("results written to {}", out.display());
    Ok(true)
}

struct Candidate {
    name: &'static str,
    program: Option<PathBuf>,
    one_line: &'static [&'static str],
    interactive: Option<&'static [&'static str]>,
    detail: Option<&'static str>,
}

type ShellSpec = (
    &'static str,
    &'static [&'static str],
    Option<&'static [&'static str]>,
);

fn candidates(pwsh: &Path, rpsh: &Path) -> Vec<Candidate> {
    let shells: &[ShellSpec] = &[
        ("bash", &["-c", "exit"], Some(&["-i"])),
        ("dash", &["-c", "exit"], Some(&["-i"])),
        ("zsh", &["-c", "exit"], Some(&["-i"])),
        ("fish", &["-c", "exit"], Some(&["-i"])),
        ("nu", &["-c", "exit"], Some(&["-i"])),
    ];
    let mut candidates: Vec<Candidate> = shells
        .iter()
        .map(|(name, one_line, interactive)| {
            let program = find_program(name);
            Candidate {
                name,
                detail: program.is_none().then_some("not found on PATH"),
                program,
                one_line,
                interactive: *interactive,
            }
        })
        .collect();
    candidates.push(Candidate {
        name: "pwsh",
        program: Some(pwsh.to_path_buf()),
        one_line: &["-NoProfile", "-NonInteractive", "-Command", "exit"],
        interactive: Some(&["-NoLogo"]),
        detail: None,
    });
    candidates.push(Candidate {
        name: "rpsh",
        program: Some(rpsh.to_path_buf()),
        one_line: &["-NoProfile", "-NonInteractive", "-Command", "exit"],
        interactive: None,
        detail: Some("interactive sessions are not implemented"),
    });
    candidates
}

fn find_program(name: &str) -> Option<PathBuf> {
    let mut search = std::env::var_os("PATH")
        .map_or_else(Vec::new, |path| std::env::split_paths(&path).collect());
    search.extend(std::env::split_paths(&std::ffi::OsStr::new(REFERENCE_PATH)));
    search
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

struct ShellResult {
    name: &'static str,
    one_line: Result<Summary, String>,
    interactive: Option<Result<Summary, String>>,
    detail: Option<&'static str>,
}

impl ShellResult {
    fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Err(error) = &self.one_line {
            errors.push(format!("one line: {error}"));
        }
        if let Some(Err(error)) = &self.interactive {
            errors.push(format!("interactive: {error}"));
        }
        errors
    }
}

fn measure(
    candidate: &Candidate,
    root: &Path,
    measurer: &Measurer,
    runs: usize,
    warmup: usize,
) -> ShellResult {
    let Some(program) = &candidate.program else {
        return ShellResult {
            name: candidate.name,
            one_line: Err(candidate.detail.unwrap_or("not installed").to_owned()),
            interactive: None,
            detail: candidate.detail,
        };
    };
    let one_line = measure_one_line(program, candidate.one_line, root, measurer, runs, warmup);
    let interactive = candidate
        .interactive
        .map(|args| measure_interactive(program, args, root, runs, warmup));
    ShellResult {
        name: candidate.name,
        one_line,
        interactive,
        detail: candidate.detail,
    }
}

fn measure_one_line(
    program: &Path,
    args: &[&str],
    root: &Path,
    measurer: &Measurer,
    runs: usize,
    warmup: usize,
) -> Result<Summary, String> {
    for _ in 0..warmup {
        run_shell_once(program, args, root, measurer)?;
    }
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        samples.push(run_shell_once(program, args, root, measurer)?);
    }
    Ok(stats::summarize(&samples))
}

fn run_shell_once(
    program: &Path,
    args: &[&str],
    root: &Path,
    measurer: &Measurer,
) -> Result<Sample, String> {
    let command = shell_command(program, args, root);
    let measured = measurer.run(command)?;
    if measured.exit_code != Some(0) {
        return Err(format!(
            "{} exited with {:?}",
            program.display(),
            measured.exit_code
        ));
    }
    Ok(Sample {
        elapsed: measured.elapsed,
        user: measured.metrics.user,
        system: measured.metrics.system,
        max_rss_kib: measured.metrics.max_rss_kib,
    })
}

fn measure_interactive(
    program: &Path,
    args: &[&str],
    root: &Path,
    runs: usize,
    warmup: usize,
) -> Result<Summary, String> {
    for _ in 0..warmup {
        interactive_once(program, args, root)?;
    }
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let elapsed = interactive_once(program, args, root)?;
        samples.push(Sample {
            elapsed,
            user: Duration::ZERO,
            system: Duration::ZERO,
            max_rss_kib: 0,
        });
    }
    Ok(stats::summarize(&samples))
}

fn interactive_once(program: &Path, args: &[&str], root: &Path) -> Result<Duration, String> {
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    let mut command = Command::new(python);
    command
        .arg(root.join("scripts/pty-startup.py"))
        .arg(program)
        .args(args);
    apply_reference_environment(&mut command, root);
    command.current_dir(root);
    let output = command
        .output()
        .map_err(|error| format!("cannot run pty-startup.py: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "pty-startup.py failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let nanoseconds: u64 = text
        .trim()
        .parse()
        .map_err(|_| format!("pty-startup.py printed {text:?}"))?;
    Ok(Duration::from_nanos(nanoseconds))
}

fn shell_command(program: &Path, args: &[&str], root: &Path) -> Command {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    apply_reference_environment(&mut command, root);
    command.current_dir(root);
    command
}

fn apply_reference_environment(command: &mut Command, root: &Path) {
    command
        .env_clear()
        .env("HOME", root.join(".cache/home"))
        .env("PATH", REFERENCE_PATH)
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .env("TZ", "UTC")
        .env("NO_COLOR", "1")
        .env("POWERSHELL_TELEMETRY_OPTOUT", "1")
        .env("POWERSHELL_UPDATECHECK", "Off");
}

fn table(results: &[ShellResult]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## shell startup\n");
    let _ = writeln!(
        out,
        "| shell | one line | interactive | one-line RSS | note |"
    );
    let _ = writeln!(out, "|---|---:|---:|---:|---|");
    for result in results {
        let one_line = match &result.one_line {
            Ok(summary) => millis(summary.median_ms),
            Err(_) => "—".to_owned(),
        };
        let interactive = match &result.interactive {
            Some(Ok(summary)) => millis(summary.median_ms),
            Some(Err(_)) => "—".to_owned(),
            None => "—".to_owned(),
        };
        let rss = match &result.one_line {
            Ok(summary) => mebibytes(summary.max_rss_kib),
            Err(_) => "—".to_owned(),
        };
        let mut note = result.errors().join("; ");
        if note.is_empty() {
            note = result.detail.unwrap_or("").to_owned();
        }
        let _ = writeln!(
            out,
            "| {} | {one_line} | {interactive} | {rss} | {note} |",
            result.name
        );
    }
    let _ = writeln!(
        out,
        "\ninteractive is the time to the first output on a pseudo-terminal. \
         Shells not on PATH are recorded as skipped; a missing interactive mode \
         is left as —."
    );
    out
}

fn result_json(result: &ShellResult) -> Json {
    Json::Object(vec![
        ("name", Json::text(result.name)),
        (
            "one_line",
            Json::optional(result.one_line.as_ref().ok(), bench::summary_to_json),
        ),
        (
            "interactive",
            Json::optional(
                result
                    .interactive
                    .as_ref()
                    .and_then(|outcome| outcome.as_ref().ok()),
                bench::summary_to_json,
            ),
        ),
        (
            "errors",
            Json::Array(result.errors().into_iter().map(Json::text).collect()),
        ),
        ("detail", Json::optional(result.detail, Json::text)),
    ])
}

fn millis(value: f64) -> String {
    if value < 10.0 {
        format!("{value:.2} ms")
    } else {
        format!("{value:.1} ms")
    }
}

fn mebibytes(kib: u64) -> String {
    format!("{:.1} MiB", kib as f64 / 1024.0)
}
