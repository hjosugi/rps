//! `cargo harness conform pty`: interactive cases on a pseudo-terminal.
//!
//! Specs live in `tests/conformance/pty/*.json`; `scripts/pty-run.py` forks a
//! PTY, feeds input on a schedule, interprets the ANSI output into a final
//! screen and prints JSON. `record` runs the case with pwsh, `check` with
//! rpsh, and both compare the same golden. Native cases such as `less` name
//! their program directly and run it in either mode.
//!
//! This is deliberately outside `scripts/check.sh`: rpsh does not implement
//! interactive sessions yet, so the prompt and Ctrl-C cases are expected to
//! fail `check` until the host lands. The harness is ready for them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Args;
use crate::bench;
use crate::engines;
use crate::json::{Json, Value};

const REFERENCE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

pub(crate) fn run(root: &Path, mut args: Args) -> Result<bool, String> {
    let mode = args
        .positional()
        .ok_or_else(|| "expected `conform pty record` or `conform pty check`".to_owned())?;
    let filter = args.value("--filter")?;
    let pwsh_path = args.value("--pwsh")?;
    let rpsh_path = args.value("--rpsh")?;
    let repeat = args.number("--repeat")?.unwrap_or(3).max(1);
    args.finish()?;

    let directory = root.join("tests/conformance/pty");
    let mut specs: Vec<PathBuf> = fs::read_dir(&directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    specs.sort();
    if let Some(filter) = filter {
        specs.retain(|path| {
            path.file_stem()
                .is_some_and(|name| name.to_string_lossy().contains(&filter))
        });
    }
    if specs.is_empty() {
        return Err("no PTY case matches".to_owned());
    }

    let pwsh = engines::locate_pwsh(root, pwsh_path)?;
    let rpsh = engines::rpsh(root, rpsh_path)?;
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    let script = root.join("scripts/pty-run.py");
    let home = root.join(".cache/home");
    fs::create_dir_all(&home)
        .map_err(|error| format!("cannot create {}: {error}", home.display()))?;

    let engine = if mode == "record" { "pwsh" } else { "rpsh" };
    let mut passed = 0;
    let mut failed = 0;
    for spec in &specs {
        let name = spec.file_stem().unwrap_or_default().to_string_lossy();
        let golden_path = spec.with_extension("golden");
        if mode == "record" {
            let mut runs = Vec::with_capacity(repeat);
            for _ in 0..repeat {
                runs.push(run_case(
                    &python,
                    &script,
                    engine,
                    &pwsh.program,
                    &rpsh.program,
                    spec,
                    root,
                )?);
            }
            if runs.windows(2).any(|pair| pair[0] != pair[1]) {
                println!("UNSTABLE {name}: repeated PTY runs disagree");
                failed += 1;
                continue;
            }
            let document = outcome_json(&runs[0]);
            let text = document.pretty();
            if fs::read_to_string(&golden_path).ok().as_deref() != Some(text.as_str()) {
                bench::write_json(&golden_path, &document)?;
                println!("recorded {name}");
            }
            passed += 1;
        } else {
            let golden = fs::read_to_string(&golden_path).map_err(|_| {
                format!("{name} has no PTY golden; run `cargo harness conform pty record`")
            })?;
            let golden = outcome_from(&Value::parse(&golden)?)?;
            let mut matched = false;
            let mut last = None;
            for _ in 0..repeat {
                let run = run_case(
                    &python,
                    &script,
                    engine,
                    &pwsh.program,
                    &rpsh.program,
                    spec,
                    root,
                )?;
                if run == golden {
                    matched = true;
                    break;
                }
                last = Some(run);
            }
            if matched {
                passed += 1;
            } else {
                failed += 1;
                if let Some(run) = last {
                    println!(
                        "FAIL {name}: expected {:?}, got {:?}",
                        golden.screen, run.screen
                    );
                }
            }
        }
    }
    println!(
        "PTY cases against {}: {passed} pass, {failed} fail",
        if mode == "record" { "pwsh" } else { "rpsh" }
    );
    Ok(failed == 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Outcome {
    exit_code: Option<i32>,
    screen: String,
    timed_out: bool,
}

fn run_case(
    python: &std::ffi::OsStr,
    script: &Path,
    engine: &str,
    pwsh: &Path,
    rpsh: &Path,
    spec: &Path,
    root: &Path,
) -> Result<Outcome, String> {
    let mut command = Command::new(python);
    command
        .arg(script)
        .arg("--engine")
        .arg(engine)
        .arg("--pwsh")
        .arg(pwsh)
        .arg("--rpsh")
        .arg(rpsh)
        .arg(spec)
        .stdin(Stdio::null())
        .env_clear()
        .env("HOME", root.join(".cache/home"))
        .env("PATH", REFERENCE_PATH)
        .env("TERM", "xterm-256color")
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .env("TZ", "UTC")
        .env("NO_COLOR", "1")
        .current_dir(root);
    let output = command
        .output()
        .map_err(|error| format!("cannot run pty-run.py: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "pty-run.py failed for {}: {}",
            spec.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| "pty-run.py printed non-UTF-8".to_owned())?;
    outcome_from(&Value::parse(text.trim())?)
}

fn outcome_from(value: &Value) -> Result<Outcome, String> {
    let screen = value
        .get("screen")
        .and_then(Value::as_str)
        .ok_or_else(|| "PTY result has no screen".to_owned())?;
    let timed_out = match value.get("timed_out") {
        Some(Value::Bool(timed_out)) => *timed_out,
        _ => return Err("PTY result has no timed_out".to_owned()),
    };
    let exit_code = match value.get("exit_code") {
        Some(Value::Null) => None,
        Some(Value::Number(code)) => Some(*code as i32),
        _ => return Err("PTY result has no exit_code".to_owned()),
    };
    Ok(Outcome {
        exit_code,
        screen: screen.to_owned(),
        timed_out,
    })
}

fn outcome_json(outcome: &Outcome) -> Json {
    Json::Object(vec![
        (
            "exit_code",
            Json::optional(outcome.exit_code, |code| Json::Number(code as f64)),
        ),
        ("screen", Json::text(outcome.screen.clone())),
        ("timed_out", Json::Bool(outcome.timed_out)),
    ])
}
