//! `cargo harness bench --suite profiles`: what a typical profile costs pwsh.
//!
//! The fixtures under `bench/profiles` isolate the shapes docs/04 rule 3
//! names: many function definitions, a module import, `tool init |
//! Invoke-Expression`, and alias/completer registration. Each fixture becomes
//! the current user's profile in a private HOME; the deltas against an empty
//! profile are the per-shape cost. rpsh does not read profiles yet, so its row
//! is recorded as not implemented rather than measured against a wrong model.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Args;
use crate::bench;
use crate::engines;
use crate::json::Json;
use crate::rusage::Measurer;
use crate::stats::{self, Sample, Summary};

const REFERENCE_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const PROFILES: &[&str] = &[
    "none",
    "empty",
    "functions",
    "module-import",
    "init-iex",
    "aliases-completion",
    "typical",
];

pub(crate) fn run(root: &Path, mut args: Args) -> Result<bool, String> {
    let runs = args.number("--runs")?.unwrap_or(10);
    let warmup = args.number("--warmup")?.unwrap_or(2);
    let filter = args.value("--filter")?;
    let out = args.value("--out")?;
    let pwsh_path = args.value("--pwsh")?;
    args.finish()?;
    if runs == 0 {
        return Err("--runs must be at least 1".to_owned());
    }

    let pwsh = engines::locate_pwsh(root, pwsh_path)?;
    let pwsh_version = engines::pwsh_version(&pwsh)?;
    let measurer = Measurer::new(root)?;
    let base_home = root.join(".cache/home");
    fs::create_dir_all(&base_home)
        .map_err(|error| format!("cannot create {}: {error}", base_home.display()))?;

    let before = bench::load_average();
    let mut results = Vec::new();
    for name in PROFILES {
        if filter
            .as_deref()
            .is_some_and(|filter| !name.contains(filter))
        {
            continue;
        }
        eprint!("{name} ... ");
        let home = if *name == "none" {
            base_home.clone()
        } else {
            profile_home(root, name)?
        };
        let result = measure(&pwsh.program, name, &home, root, &measurer, runs, warmup);
        match &result.summary {
            Ok(summary) => eprintln!("{}", millis(summary.median_ms)),
            Err(error) => eprintln!("FAILED: {error}"),
        }
        results.push(result);
    }
    if results.is_empty() {
        return Err("no profile matches".to_owned());
    }

    let baseline = results
        .iter()
        .find(|result| result.name == "empty")
        .and_then(|result| result.summary.as_ref().ok())
        .map(|summary| summary.median_ms);

    print!("{}", table(&results, baseline, &pwsh_version));

    let after = bench::load_average();
    let document = Json::Object(vec![
        ("schema", Json::text("rpsh-profiles-v1")),
        ("generated_at_unix_s", Json::Number(bench::unix_seconds())),
        (
            "pwsh",
            Json::Object(vec![
                ("version", Json::text(pwsh_version)),
                ("policy", Json::text(rpsh_host::REFERENCE_POLICY)),
            ]),
        ),
        (
            "rpsh",
            Json::Object(vec![
                ("status", Json::text("not_implemented")),
                (
                    "detail",
                    Json::text("profiles are not implemented; the fixtures are ready for it"),
                ),
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
            "profiles",
            Json::Array(
                results
                    .iter()
                    .map(|result| result_json(result, baseline))
                    .collect(),
            ),
        ),
    ]);
    let out = match out {
        Some(path) => Path::new(&path).to_path_buf(),
        None => root.join("bench/results/profiles-latest.json"),
    };
    bench::write_json(&out, &document)?;
    eprintln!("results written to {}", out.display());
    Ok(true)
}

struct ProfileResult {
    name: &'static str,
    summary: Result<Summary, String>,
}

fn measure(
    pwsh: &Path,
    name: &str,
    home: &Path,
    root: &Path,
    measurer: &Measurer,
    runs: usize,
    warmup: usize,
) -> ProfileResult {
    let mut summary = || -> Result<Summary, String> {
        for _ in 0..warmup {
            run_once(pwsh, name, home, root, measurer)?;
        }
        let mut samples = Vec::with_capacity(runs);
        for _ in 0..runs {
            samples.push(run_once(pwsh, name, home, root, measurer)?);
        }
        Ok(stats::summarize(&samples))
    }();
    if let Err(error) = &mut summary {
        *error = format!("{name}: {error}");
    }
    ProfileResult {
        name: match name {
            "none" => "none",
            "empty" => "empty",
            "functions" => "functions",
            "module-import" => "module-import",
            "init-iex" => "init-iex",
            "aliases-completion" => "aliases-completion",
            "typical" => "typical",
            _ => "other",
        },
        summary,
    }
}

fn run_once(
    pwsh: &Path,
    name: &str,
    home: &Path,
    root: &Path,
    measurer: &Measurer,
) -> Result<Sample, String> {
    let mut command = Command::new(pwsh);
    if name == "none" {
        command.arg("-NoProfile");
    }
    command.args(["-NonInteractive", "-Command", "exit"]);
    command.stdin(Stdio::null());
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", REFERENCE_PATH)
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .env("TZ", "UTC")
        .env("NO_COLOR", "1")
        .env("POWERSHELL_TELEMETRY_OPTOUT", "1")
        .env("POWERSHELL_UPDATECHECK", "Off");
    command.current_dir(root);

    let measured = measurer.run(command)?;
    if measured.exit_code != Some(0) {
        return Err(format!(
            "{} exited with {:?}",
            pwsh.display(),
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

/// Creates a private HOME whose current-user profile dot-sources the fixture.
fn profile_home(root: &Path, name: &str) -> Result<PathBuf, String> {
    let home = root.join(".cache/profiles").join(name).join("home");
    let directory = home.join(".config/powershell");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let fixture = root.join("bench/profiles").join(format!("{name}.ps1"));
    let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let profile = format!(
        "# Generated by `cargo harness bench --suite profiles`.\n. {}\n",
        quote(&fixture.display().to_string())
    );
    fs::write(directory.join("Microsoft.PowerShell_profile.ps1"), profile).map_err(|error| {
        format!(
            "cannot write the profile in {}: {error}",
            directory.display()
        )
    })?;
    Ok(home)
}

fn table(results: &[ProfileResult], baseline: Option<f64>, pwsh_version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## pwsh {pwsh_version} profile cost\n");
    let _ = writeln!(out, "| profile | wall | vs empty | CPU | peak RSS | note |");
    let _ = writeln!(out, "|---|---:|---:|---:|---:|---|");
    for result in results {
        let (wall, cpu, rss, delta, note) = match &result.summary {
            Ok(summary) => {
                let delta = match (baseline, result.name) {
                    (Some(base), name) if name != "empty" && name != "none" && base > 0.0 => {
                        format!("{:+.1}%", (summary.median_ms - base) / base * 100.0)
                    }
                    _ => "—".to_owned(),
                };
                (
                    millis(summary.median_ms),
                    millis(summary.mean_user_ms + summary.mean_system_ms),
                    mebibytes(summary.max_rss_kib),
                    delta,
                    String::new(),
                )
            }
            Err(error) => (
                "—".to_owned(),
                "—".to_owned(),
                "—".to_owned(),
                "—".to_owned(),
                error.clone(),
            ),
        };
        let _ = writeln!(
            out,
            "| {} | {wall} | {delta} | {cpu} | {rss} | {note} |",
            result.name
        );
    }
    let _ = writeln!(
        out,
        "\nrpsh: profiles are not implemented; these fixtures are the cases it will run."
    );
    out
}

fn result_json(result: &ProfileResult, baseline: Option<f64>) -> Json {
    let delta = match (&result.summary, baseline, result.name) {
        (Ok(summary), Some(base), name) if name != "empty" && name != "none" && base > 0.0 => {
            Some((summary.median_ms - base) / base * 100.0)
        }
        _ => None,
    };
    Json::Object(vec![
        ("name", Json::text(result.name)),
        (
            "wall",
            Json::optional(result.summary.as_ref().ok(), bench::summary_to_json),
        ),
        ("delta_percent", Json::optional(delta, Json::Number)),
        (
            "error",
            Json::optional(result.summary.as_ref().err().cloned(), Json::Text),
        ),
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
