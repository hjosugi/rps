//! `cargo harness bench`: wall-clock time of each script under pwsh and rpsh.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rpsh_host::TARGET_PWSH_VERSION;

use crate::Args;
use crate::cases::{self, Case};
use crate::engines::{self, Engine, Reference, Verdict};
use crate::json::Json;
use crate::stats::{self, Summary};

enum Status {
    /// rpsh printed what pwsh printed. Its time may be compared.
    Verified,
    /// rpsh declined the script. Only pwsh is timed.
    NotImplemented(String),
    /// rpsh printed something else. A bug; the run fails.
    Mismatch(String),
}

struct CaseResult {
    id: String,
    status: Status,
    pwsh: Summary,
    rpsh: Option<Summary>,
    /// rpsh's allowed median, from `# rpsh-bench: budget-ms=N` times
    /// `--budget-scale` (docs/05_startup.md).
    budget_ms: Option<f64>,
}

impl CaseResult {
    fn speedup(&self) -> Option<f64> {
        let rpsh = self.rpsh.as_ref()?;
        Some(self.pwsh.median_ms / rpsh.median_ms)
    }

    /// `Some(false)` when rpsh was timed and its median exceeds the budget.
    fn within_budget(&self) -> Option<bool> {
        let budget = self.budget_ms?;
        let rpsh = self.rpsh.as_ref()?;
        Some(rpsh.median_ms <= budget)
    }
}

pub(crate) fn run(root: &Path, args: &[String]) -> Result<bool, String> {
    let mut args = Args::new(args);
    let runs = args.number("--runs")?.unwrap_or(10);
    let warmup = args.number("--warmup")?.unwrap_or(2);
    let filter = args.value("--filter")?;
    let out = args.value("--out")?;
    let pwsh_path = args.value("--pwsh")?;
    let rpsh_path = args.value("--rpsh")?;
    let budget_scale = match args.value("--budget-scale")? {
        Some(text) => text
            .parse::<f64>()
            .ok()
            .filter(|scale| *scale > 0.0)
            .ok_or_else(|| format!("--budget-scale expects a positive number, got {text}"))?,
        None => 1.0,
    };
    args.finish()?;
    if runs == 0 {
        return Err("--runs must be at least 1".to_owned());
    }

    let pwsh = engines::locate_pwsh(root, pwsh_path)?;
    let pwsh_version = engines::pwsh_version(&pwsh)?;
    if pwsh_version != TARGET_PWSH_VERSION {
        eprintln!(
            "warning: measuring against PowerShell {pwsh_version}; \
             rpsh targets {TARGET_PWSH_VERSION} (scripts/fetch-pwsh.sh installs it)"
        );
    }
    let rpsh = engines::rpsh(root, rpsh_path)?;
    let cases = cases::collect(&root.join("bench/cases"), filter.as_deref())?;
    if cases.is_empty() {
        return Err("no benchmark case matches".to_owned());
    }

    let before = load_average();
    let mut results = Vec::new();
    for case in &cases {
        eprint!("{} ... ", case.id);
        let result = measure(case, &pwsh, &rpsh, runs, warmup, budget_scale)?;
        match &result.status {
            Status::Verified => eprintln!("verified"),
            Status::NotImplemented(feature) => eprintln!("not implemented: {feature}"),
            Status::Mismatch(why) => eprintln!("MISMATCH: {why}"),
        }
        if result.within_budget() == Some(false) {
            eprintln!("  OVER BUDGET");
        }
        results.push(result);
    }

    print!("{}", markdown(&results, &pwsh_version));

    let document = Json::Object(vec![
        ("schema", Json::text("rpsh-bench-v1")),
        ("generated_at_unix_s", Json::Number(unix_seconds())),
        ("rpsh", rpsh_info(root, &rpsh)),
        (
            "pwsh",
            Json::Object(vec![
                ("version", Json::text(pwsh_version)),
                ("target_version", Json::text(TARGET_PWSH_VERSION)),
                ("executable", Json::text(recorded_path(root, &pwsh.program))),
            ]),
        ),
        ("environment", environment(before)),
        (
            "settings",
            Json::Object(vec![
                ("runs", Json::Number(runs as f64)),
                ("warmup", Json::Number(warmup as f64)),
                ("budget_scale", Json::Number(budget_scale)),
            ]),
        ),
        ("summary", summary_json(&results)),
        (
            "cases",
            Json::Array(results.iter().map(case_json).collect()),
        ),
    ]);
    let out = match out {
        Some(path) => Path::new(&path).to_path_buf(),
        None => root.join("bench/results/latest.json"),
    };
    if let Some(directory) = out.parent() {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    }
    fs::write(&out, document.pretty())
        .map_err(|error| format!("cannot write {}: {error}", out.display()))?;
    eprintln!("results written to {}", out.display());

    Ok(!results.iter().any(|result| {
        matches!(result.status, Status::Mismatch(_)) || result.within_budget() == Some(false)
    }))
}

fn measure(
    case: &Case,
    pwsh: &Engine,
    rpsh: &Engine,
    runs: usize,
    warmup: usize,
    budget_scale: f64,
) -> Result<CaseResult, String> {
    let source = fs::read_to_string(&case.path)
        .map_err(|error| format!("cannot read {}: {error}", case.path.display()))?;
    let runs = cases::directive(&source, "runs").unwrap_or(runs).max(1);
    let warmup = cases::directive(&source, "warmup").unwrap_or(warmup);
    let budget_ms =
        cases::directive(&source, "budget-ms").map(|budget| budget as f64 * budget_scale);

    // The first run of each engine establishes the answer and is not timed.
    let first = pwsh.run_file(&case.path)?;
    let Some(exit_code) = first.exit_code else {
        return Err(format!("{}: pwsh was killed by a signal", case.id));
    };
    let reference = Reference {
        stdout: first.stdout,
        exit_code,
    };
    let mut status = match engines::judge(&reference, &rpsh.run_file(&case.path)?) {
        Verdict::Match => Status::Verified,
        Verdict::NotImplemented(feature) => Status::NotImplemented(feature),
        Verdict::Mismatch(why) => Status::Mismatch(why),
    };

    for _ in 0..warmup {
        pwsh.run_file(&case.path)?;
        if matches!(status, Status::Verified) {
            rpsh.run_file(&case.path)?;
        }
    }

    // Alternate the engines so that drift in machine load hits both.
    let mut pwsh_samples = Vec::with_capacity(runs);
    let mut rpsh_samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let run = pwsh.run_file(&case.path)?;
        if run.stdout != reference.stdout || run.exit_code != Some(reference.exit_code) {
            return Err(format!(
                "{}: pwsh printed something different on a later run; \
                 benchmark scripts must be deterministic",
                case.id
            ));
        }
        pwsh_samples.push(run.elapsed);

        if matches!(status, Status::Verified) {
            let run = rpsh.run_file(&case.path)?;
            match engines::judge(&reference, &run) {
                Verdict::Match => rpsh_samples.push(run.elapsed),
                Verdict::NotImplemented(why) | Verdict::Mismatch(why) => {
                    status = Status::Mismatch(format!("unstable across runs: {why}"));
                }
            }
        }
    }

    let rpsh = matches!(status, Status::Verified).then(|| stats::summarize(&rpsh_samples));
    Ok(CaseResult {
        id: case.id.clone(),
        status,
        pwsh: stats::summarize(&pwsh_samples),
        rpsh,
        budget_ms,
    })
}

fn markdown(results: &[CaseResult], pwsh_version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## rpsh vs pwsh {pwsh_version}\n");
    let _ = writeln!(
        out,
        "| case | status | pwsh median | rpsh median | speedup | rpsh budget |"
    );
    let _ = writeln!(out, "|---|---|---:|---:|---:|---|");
    for result in results {
        let status = match &result.status {
            Status::Verified => "verified".to_owned(),
            Status::NotImplemented(feature) => format!("not implemented: {feature}"),
            Status::Mismatch(why) => format!("**MISMATCH**: {why}"),
        };
        let rpsh = result
            .rpsh
            .as_ref()
            .map_or("—".to_owned(), |summary| millis(summary.median_ms));
        let speedup = result.speedup().map_or("—".to_owned(), ratio);
        let budget = match (result.budget_ms, result.within_budget()) {
            (None, _) => "—".to_owned(),
            (Some(budget), None) => format!("{} (not timed)", millis(budget)),
            (Some(budget), Some(true)) => format!("{} ok", millis(budget)),
            (Some(budget), Some(false)) => format!("{} **OVER**", millis(budget)),
        };
        let _ = writeln!(
            out,
            "| {} | {status} | {} | {rpsh} | {speedup} | {budget} |",
            result.id,
            millis(result.pwsh.median_ms),
        );
    }

    let speedups: Vec<f64> = results.iter().filter_map(CaseResult::speedup).collect();
    let _ = writeln!(
        out,
        "\nverified {} of {} cases. A speedup is shown only where rpsh's stdout \
         and exit code equal pwsh's.",
        speedups.len(),
        results.len()
    );
    if let Some(mean) = stats::geometric_mean(&speedups) {
        let _ = writeln!(
            out,
            "geometric mean speedup over the verified cases: {}",
            ratio(mean)
        );
    }
    let missing = missing_features(results);
    if !missing.is_empty() {
        let list: Vec<String> = missing
            .iter()
            .map(|(feature, count)| format!("{feature} ×{count}"))
            .collect();
        let _ = writeln!(out, "not implemented: {}", list.join(", "));
    }
    out
}

/// Missing features with the number of cases each one blocks, most first.
fn missing_features(results: &[CaseResult]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for result in results {
        if let Status::NotImplemented(feature) = &result.status {
            *counts.entry(feature).or_default() += 1;
        }
    }
    let mut counts: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(feature, count)| (feature.to_owned(), count))
        .collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

fn millis(value: f64) -> String {
    if value < 10.0 {
        format!("{value:.2} ms")
    } else {
        format!("{value:.1} ms")
    }
}

fn ratio(value: f64) -> String {
    if value >= 100.0 {
        format!("{value:.0}x")
    } else if value >= 10.0 {
        format!("{value:.1}x")
    } else {
        format!("{value:.2}x")
    }
}

fn summary_json(results: &[CaseResult]) -> Json {
    let count = |wanted: fn(&Status) -> bool| {
        Json::Number(results.iter().filter(|r| wanted(&r.status)).count() as f64)
    };
    let speedups: Vec<f64> = results.iter().filter_map(CaseResult::speedup).collect();
    let over_budget = results
        .iter()
        .filter(|result| result.within_budget() == Some(false))
        .count();
    Json::Object(vec![
        ("cases", Json::Number(results.len() as f64)),
        ("over_budget", Json::Number(over_budget as f64)),
        ("verified", count(|s| matches!(s, Status::Verified))),
        (
            "not_implemented",
            count(|s| matches!(s, Status::NotImplemented(_))),
        ),
        ("mismatch", count(|s| matches!(s, Status::Mismatch(_)))),
        (
            "geometric_mean_speedup",
            Json::optional(stats::geometric_mean(&speedups), Json::Number),
        ),
    ])
}

fn case_json(result: &CaseResult) -> Json {
    let (status, detail) = match &result.status {
        Status::Verified => ("verified", None),
        Status::NotImplemented(feature) => ("not_implemented", Some(feature.clone())),
        Status::Mismatch(why) => ("mismatch", Some(why.clone())),
    };
    Json::Object(vec![
        ("id", Json::text(result.id.clone())),
        ("status", Json::text(status)),
        ("detail", Json::optional(detail, Json::Text)),
        ("speedup", Json::optional(result.speedup(), Json::Number)),
        (
            "rpsh_budget_ms",
            Json::optional(result.budget_ms, Json::Number),
        ),
        (
            "within_budget",
            Json::optional(result.within_budget(), Json::Bool),
        ),
        ("pwsh", summary_to_json(&result.pwsh)),
        (
            "rpsh",
            Json::optional(result.rpsh.as_ref(), summary_to_json),
        ),
    ])
}

fn summary_to_json(summary: &Summary) -> Json {
    Json::Object(vec![
        ("median_ms", Json::Number(summary.median_ms)),
        ("min_ms", Json::Number(summary.min_ms)),
        ("max_ms", Json::Number(summary.max_ms)),
        ("mean_ms", Json::Number(summary.mean_ms)),
        ("stddev_ms", Json::Number(summary.stddev_ms)),
        (
            "samples_ms",
            Json::Array(
                summary
                    .samples_ms
                    .iter()
                    .copied()
                    .map(Json::Number)
                    .collect(),
            ),
        ),
    ])
}

fn rpsh_info(root: &Path, rpsh: &Engine) -> Json {
    let commit = command_output(root, "git", &["rev-parse", "--short", "HEAD"]);
    let dirty =
        command_output(root, "git", &["status", "--porcelain"]).map(|changes| !changes.is_empty());
    Json::Object(vec![
        ("version", Json::text(env!("CARGO_PKG_VERSION"))),
        ("commit", Json::optional(commit, Json::Text)),
        ("dirty", Json::optional(dirty, Json::Bool)),
        ("executable", Json::text(recorded_path(root, &rpsh.program))),
    ])
}

/// What a reader needs to judge how far the numbers can be trusted.
fn environment(load_before: Option<Vec<f64>>) -> Json {
    let cpu_model = fs::read_to_string("/proc/cpuinfo").ok().and_then(|info| {
        info.lines()
            .find(|line| line.starts_with("model name"))
            .and_then(|line| line.split_once(':'))
            .map(|(_, model)| model.trim().to_owned())
    });
    let cpu_count = std::thread::available_parallelism().ok().map(|n| n.get());
    let load = |values: Option<Vec<f64>>| {
        Json::optional(values, |values| {
            Json::Array(values.into_iter().map(Json::Number).collect())
        })
    };
    Json::Object(vec![
        ("os", Json::text(std::env::consts::OS)),
        ("arch", Json::text(std::env::consts::ARCH)),
        (
            "kernel",
            Json::optional(read_trimmed("/proc/sys/kernel/osrelease"), Json::Text),
        ),
        ("cpu_model", Json::optional(cpu_model, Json::Text)),
        (
            "cpu_count",
            Json::optional(cpu_count, |count| Json::Number(count as f64)),
        ),
        (
            "cpu_governor",
            Json::optional(
                read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
                Json::Text,
            ),
        ),
        ("load_average_before", load(load_before)),
        ("load_average_after", load(load_average())),
        (
            "rustc",
            Json::optional(command_output(Path::new("."), "rustc", &["-V"]), Json::Text),
        ),
    ])
}

fn load_average() -> Option<Vec<f64>> {
    let text = fs::read_to_string("/proc/loadavg").ok()?;
    let values: Vec<f64> = text
        .split_whitespace()
        .take(3)
        .filter_map(|field| field.parse().ok())
        .collect();
    (values.len() == 3).then_some(values)
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

fn command_output(directory: &Path, program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs() as f64)
}

/// How a path is written into a result file: relative to the checkout when
/// inside it, otherwise only the file name. A result file is meant to be
/// committed and should not carry the directory layout of one machine.
fn recorded_path(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(relative) => relative.display().to_string(),
        Err(_) => path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
    }
}
