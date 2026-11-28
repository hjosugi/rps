//! `cargo harness bench`: wall-clock time of each script under pwsh and rpsh.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rpsh_host::REFERENCE_POLICY;

use crate::Args;
use crate::cases::{self, Case};
use crate::engines::{self, Engine, Reference, Run, Verdict};
use crate::json::{Json, Value};
use crate::rusage::Measurer;
use crate::stats::{self, Sample, Summary};

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
    invocation: &'static str,
    status: Status,
    /// The exit code every pwsh run produced; the hyperfine export repeats it.
    reference_exit: i32,
    pwsh: Summary,
    rpsh: Option<Summary>,
    bash: Option<Summary>,
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
    let suite = args
        .value("--suite")?
        .unwrap_or_else(|| "scripts".to_owned());
    match suite.as_str() {
        "scripts" => {}
        "shells" => return crate::shells::run(root, args),
        "profiles" => return crate::profiles::run(root, args),
        other => {
            return Err(format!(
                "unknown suite '{other}' (scripts | shells | profiles)"
            ));
        }
    }
    let include_bash = args.flag("--bash");
    let runs = args.number("--runs")?.unwrap_or(10);
    let warmup = args.number("--warmup")?.unwrap_or(2);
    let filter = args.value("--filter")?;
    let out = args.value("--out")?;
    let compare = args.value("--compare")?;
    let hyperfine = args.value("--export-hyperfine")?;
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
    let rpsh = engines::rpsh(root, rpsh_path)?;
    let cases = cases::collect(&root.join("bench/cases"), filter.as_deref())?;
    if cases.is_empty() {
        return Err("no benchmark case matches".to_owned());
    }

    let bash = include_bash.then(|| Engine::bash(root)).transpose()?;
    let settings = Settings {
        runs,
        warmup,
        budget_scale,
    };
    let before = load_average();
    let measurer = Measurer::new(root)?;
    let mut results = Vec::new();
    for case in &cases {
        eprint!("{} ... ", case.id);
        let result = measure(case, &pwsh, &rpsh, bash.as_ref(), &measurer, &settings)?;
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
    if let Ok(metadata) = fs::metadata(&rpsh.program) {
        println!(
            "\nrpsh binary: {:.2} MiB",
            metadata.len() as f64 / (1024.0 * 1024.0)
        );
    }

    let after = load_average();
    let warnings = collect_warnings(before.as_deref(), after.as_deref());
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }

    let mut fields = vec![
        ("schema", Json::text("rpsh-bench-v2")),
        ("generated_at_unix_s", Json::Number(unix_seconds())),
        ("rpsh", rpsh_info(root, &rpsh)),
        (
            "pwsh",
            Json::Object(vec![
                ("version", Json::text(pwsh_version.clone())),
                ("target_version", Json::text(pwsh_version)),
                ("policy", Json::text(REFERENCE_POLICY)),
                ("executable", Json::text(recorded_path(root, &pwsh.program))),
            ]),
        ),
        ("environment", environment(before, after)),
        (
            "warnings",
            Json::Array(warnings.into_iter().map(Json::text).collect()),
        ),
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
    ];
    if let Some(bash) = &bash {
        let version = command_output(
            root,
            bash.program.to_str().ok_or("invalid bash path")?,
            &["--version"],
        )
        .and_then(|text| text.lines().next().map(str::to_owned));
        fields.push((
            "bash",
            Json::Object(vec![
                ("executable", Json::text(recorded_path(root, &bash.program))),
                ("version", Json::optional(version, Json::Text)),
            ]),
        ));
    }
    let document = Json::Object(fields);
    let out = match out {
        Some(path) => Path::new(&path).to_path_buf(),
        None => root.join("bench/results/latest.json"),
    };
    write_json(&out, &document)?;
    eprintln!("results written to {}", out.display());

    if let Some(path) = compare {
        let path = Path::new(&path);
        print!("{}", compare_table(path, &results)?);
    }
    if let Some(path) = hyperfine {
        let path = Path::new(&path);
        write_json(path, &hyperfine_json(&results))?;
        eprintln!("hyperfine export written to {}", path.display());
    }

    Ok(!results.iter().any(|result| {
        matches!(result.status, Status::Mismatch(_))
            || (result.budget_ms.is_some() && result.within_budget() != Some(true))
    }))
}

pub(crate) fn write_json(path: &Path, document: &Json) -> Result<(), String> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    }
    fs::write(path, document.pretty())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

struct Settings {
    runs: usize,
    warmup: usize,
    budget_scale: f64,
}

fn measure(
    case: &Case,
    pwsh: &Engine,
    rpsh: &Engine,
    bash: Option<&Engine>,
    measurer: &Measurer,
    settings: &Settings,
) -> Result<CaseResult, String> {
    let source = fs::read_to_string(&case.path)
        .map_err(|error| format!("cannot read {}: {error}", case.path.display()))?;
    let inline = cases::inline_command(&source)?;
    let runs = cases::directive(&source, "runs")
        .unwrap_or(settings.runs)
        .max(1);
    let warmup = cases::directive(&source, "warmup").unwrap_or(settings.warmup);
    let budget_ms =
        cases::directive(&source, "budget-ms").map(|budget| budget as f64 * settings.budget_scale);

    // The first run of each engine establishes the answer and is not timed.
    let first = pwsh.run_measured(&case.path, inline, measurer)?;
    let Some(exit_code) = first.exit_code else {
        return Err(format!("{}: pwsh was killed by a signal", case.id));
    };
    let reference = Reference {
        stdout: first.stdout,
        stderr: None,
        exit_code,
    };
    let bash_script = case.path.with_extension("bash");
    let bash = bash.filter(|_| bash_script.is_file());
    let run_bash = |engine: &Engine| -> Result<Sample, String> {
        let run = engine.run_measured(&bash_script, None, measurer)?;
        match engines::judge(&reference, &run) {
            Verdict::Match => sample_of(&run),
            Verdict::Mismatch(why) | Verdict::NotImplemented(why) => {
                Err(format!("{}: bash {why}", case.id))
            }
        }
    };
    let mut status = match engines::judge(
        &reference,
        &rpsh.run_measured(&case.path, inline, measurer)?,
    ) {
        Verdict::Match => Status::Verified,
        Verdict::NotImplemented(feature) => Status::NotImplemented(feature),
        Verdict::Mismatch(why) => Status::Mismatch(why),
    };

    if let Some(bash) = bash {
        run_bash(bash)?;
    }
    for _ in 0..warmup {
        if let Some(bash) = bash {
            run_bash(bash)?;
        }
        pwsh.run_measured(&case.path, inline, measurer)?;
        if matches!(status, Status::Verified) {
            rpsh.run_measured(&case.path, inline, measurer)?;
        }
    }

    // Alternate the engines so that drift in machine load hits both.
    let mut pwsh_samples = Vec::with_capacity(runs);
    let mut rpsh_samples = Vec::with_capacity(runs);
    let mut bash_samples = Vec::new();
    for _ in 0..runs {
        let run = pwsh.run_measured(&case.path, inline, measurer)?;
        if run.stdout != reference.stdout || run.exit_code != Some(reference.exit_code) {
            return Err(format!(
                "{}: pwsh printed something different on a later run; \
                 benchmark scripts must be deterministic",
                case.id
            ));
        }
        pwsh_samples.push(sample_of(&run)?);

        if matches!(status, Status::Verified) {
            let run = rpsh.run_measured(&case.path, inline, measurer)?;
            match engines::judge(&reference, &run) {
                Verdict::Match => rpsh_samples.push(sample_of(&run)?),
                Verdict::NotImplemented(why) | Verdict::Mismatch(why) => {
                    status = Status::Mismatch(format!("unstable across runs: {why}"));
                }
            }
        }
        if let Some(bash) = bash {
            bash_samples.push(run_bash(bash)?);
        }
    }

    let rpsh = matches!(status, Status::Verified).then(|| stats::summarize(&rpsh_samples));
    Ok(CaseResult {
        id: case.id.clone(),
        invocation: if inline.is_some() {
            "-Command"
        } else {
            "-File"
        },
        status,
        reference_exit: exit_code,
        pwsh: stats::summarize(&pwsh_samples),
        rpsh,
        bash: bash.map(|_| stats::summarize(&bash_samples)),
        budget_ms,
    })
}

fn sample_of(run: &Run) -> Result<Sample, String> {
    let metrics = run
        .metrics
        .ok_or_else(|| "a measured run has no rusage".to_owned())?;
    Ok(Sample {
        elapsed: run.elapsed,
        user: metrics.user,
        system: metrics.system,
        max_rss_kib: metrics.max_rss_kib,
    })
}

fn markdown(results: &[CaseResult], pwsh_version: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## rpsh vs pwsh {pwsh_version}\n");
    let _ = writeln!(
        out,
        "| case | status | pwsh median | rpsh median | speedup | rpsh peak RSS | rpsh CPU | rpsh budget |"
    );
    let _ = writeln!(out, "|---|---|---:|---:|---:|---:|---:|---|");
    for result in results {
        let status = match &result.status {
            Status::Verified => "verified".to_owned(),
            Status::NotImplemented(feature) => format!("not implemented: {feature}"),
            Status::Mismatch(why) => format!("**MISMATCH**: {why}"),
        };
        let rpsh = result.rpsh.as_ref().map_or("—".to_owned(), |summary| {
            format!(
                "{} [{:.2}–{:.2}]",
                millis(summary.median_ms),
                summary.median_ci95_ms[0],
                summary.median_ci95_ms[1]
            )
        });
        let peak_rss = result
            .rpsh
            .as_ref()
            .map_or("—".to_owned(), |summary| mebibytes(summary.max_rss_kib));
        let cpu = result.rpsh.as_ref().map_or("—".to_owned(), |summary| {
            millis(summary.mean_user_ms + summary.mean_system_ms)
        });
        let speedup = result.speedup().map_or("—".to_owned(), ratio);
        let budget = match (result.budget_ms, result.within_budget()) {
            (None, _) => "—".to_owned(),
            (Some(budget), None) => format!("{} (not timed)", millis(budget)),
            (Some(budget), Some(true)) => format!("{} ok", millis(budget)),
            (Some(budget), Some(false)) => format!("{} **OVER**", millis(budget)),
        };
        let _ = writeln!(
            out,
            "| {} | {status} | {} | {rpsh} | {speedup} | {peak_rss} | {cpu} | {budget} |",
            result.id,
            millis(result.pwsh.median_ms),
        );
    }

    if results.iter().any(|row| row.bash.is_some()) {
        let _ = writeln!(
            out,
            "\n| native workload | bash median | rpsh median |\n|---|---:|---:|"
        );
        for row in results {
            if let Some(bash) = &row.bash {
                let _ = writeln!(
                    out,
                    "| {} | {} | {} |",
                    row.id,
                    millis(bash.median_ms),
                    optional_millis(row.rpsh.as_ref().map(|s| s.median_ms))
                );
            }
        }
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
    let outliers: Vec<String> = results
        .iter()
        .filter_map(|result| {
            let pwsh = result.pwsh.outliers_ms.len();
            let rpsh = result.rpsh.as_ref().map_or(0, |s| s.outliers_ms.len());
            (pwsh + rpsh > 0).then(|| format!("{} (pwsh {pwsh}, rpsh {rpsh})", result.id))
        })
        .collect();
    if !outliers.is_empty() {
        let _ = writeln!(out, "outliers: {}", outliers.join(", "));
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

fn mebibytes(kib: u64) -> String {
    format!("{:.1} MiB", kib as f64 / 1024.0)
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
    let mut fields = vec![
        ("id", Json::text(result.id.clone())),
        ("invocation", Json::text(result.invocation)),
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
    ];
    if let Some(bash) = &result.bash {
        fields.push(("bash", summary_to_json(bash)));
    }
    Json::Object(fields)
}

pub(crate) fn summary_to_json(summary: &Summary) -> Json {
    Json::Object(vec![
        ("median_ms", Json::Number(summary.median_ms)),
        (
            "median_ci95_ms",
            Json::Array(vec![
                Json::Number(summary.median_ci95_ms[0]),
                Json::Number(summary.median_ci95_ms[1]),
            ]),
        ),
        (
            "outliers_ms",
            Json::Array(
                summary
                    .outliers_ms
                    .iter()
                    .copied()
                    .map(Json::Number)
                    .collect(),
            ),
        ),
        ("min_ms", Json::Number(summary.min_ms)),
        ("max_ms", Json::Number(summary.max_ms)),
        ("mean_ms", Json::Number(summary.mean_ms)),
        ("stddev_ms", Json::Number(summary.stddev_ms)),
        ("mean_user_ms", Json::Number(summary.mean_user_ms)),
        ("mean_system_ms", Json::Number(summary.mean_system_ms)),
        ("max_rss_kib", Json::Number(summary.max_rss_kib as f64)),
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
    let size_bytes = fs::metadata(&rpsh.program)
        .ok()
        .map(|metadata| metadata.len());
    Json::Object(vec![
        ("version", Json::text(env!("CARGO_PKG_VERSION"))),
        ("commit", Json::optional(commit, Json::Text)),
        ("dirty", Json::optional(dirty, Json::Bool)),
        ("executable", Json::text(recorded_path(root, &rpsh.program))),
        (
            "size_bytes",
            Json::optional(size_bytes, |size| Json::Number(size as f64)),
        ),
    ])
}

/// What a reader needs to judge how far the numbers can be trusted.
pub(crate) fn environment(load_before: Option<Vec<f64>>, load_after: Option<Vec<f64>>) -> Json {
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
        ("load_average_after", load(load_after)),
        (
            "rustc",
            Json::optional(command_output(Path::new("."), "rustc", &["-V"]), Json::Text),
        ),
        ("rusage", Json::text("wait4 via scripts/spawn-measure.c")),
    ])
}

/// Conditions that make the numbers less comparable with a baseline.
fn collect_warnings(before: Option<&[f64]>, after: Option<&[f64]>) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(governor) = read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        && governor == "powersave"
    {
        warnings.push(format!(
            "cpu governor is {governor}; frequency scaling will add variance"
        ));
    }
    if let Some(cpus) = std::thread::available_parallelism().ok().map(|n| n.get()) {
        let one_minute = before
            .into_iter()
            .chain(after)
            .filter_map(|values| values.first())
            .copied()
            .fold(0.0f64, f64::max);
        if one_minute > cpus as f64 {
            warnings.push(format!(
                "1-minute load average {one_minute:.2} exceeds {cpus} CPUs; \
                 close other work before taking numbers worth citing"
            ));
        }
    }
    warnings
}

/// A diff against an earlier result file. Both `rpsh-bench-v1` and the
/// current schema are read through the same fields, so old baselines stay
/// comparable.
fn compare_table(path: &Path, results: &[CaseResult]) -> Result<String, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let baseline = Value::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let schema = baseline
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or("unknown schema");
    let cases = baseline
        .get("cases")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{}: no cases array", path.display()))?;

    let mut before = BTreeMap::new();
    for case in cases {
        let Some(id) = case.get("id").and_then(Value::as_str) else {
            continue;
        };
        let pwsh = case
            .get("pwsh")
            .and_then(|summary| summary.get("median_ms"))
            .and_then(Value::as_f64);
        let rpsh = case
            .get("rpsh")
            .and_then(|summary| summary.get("median_ms"))
            .and_then(Value::as_f64);
        before.insert(id.to_owned(), (pwsh, rpsh));
    }

    let now: BTreeMap<&str, &CaseResult> = results
        .iter()
        .map(|result| (result.id.as_str(), result))
        .collect();
    let mut out = String::new();
    let _ = writeln!(out, "## compared with {} ({schema})\n", path.display());
    let _ = writeln!(
        out,
        "| case | pwsh before | pwsh now | change | rpsh before | rpsh now | change |"
    );
    let _ = writeln!(out, "|---|---:|---:|---:|---:|---:|---:|");
    let mut new_cases = Vec::new();
    for result in results {
        let Some((old_pwsh, old_rpsh)) = before.get(result.id.as_str()) else {
            new_cases.push(result.id.clone());
            continue;
        };
        let pwsh_now = result.pwsh.median_ms;
        let rpsh_now = result.rpsh.as_ref().map(|summary| summary.median_ms);
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            result.id,
            optional_millis(*old_pwsh),
            millis(pwsh_now),
            change(*old_pwsh, Some(pwsh_now)),
            optional_millis(*old_rpsh),
            optional_millis(rpsh_now),
            change(*old_rpsh, rpsh_now),
        );
    }

    let only_before: Vec<&str> = before
        .keys()
        .filter(|id| !now.contains_key(id.as_str()))
        .map(String::as_str)
        .collect();
    let _ = writeln!(
        out,
        "\n{} new case(s), {} case(s) only in the baseline",
        new_cases.len(),
        only_before.len()
    );
    if !new_cases.is_empty() {
        let _ = writeln!(out, "new: {}", new_cases.join(", "));
    }
    if !only_before.is_empty() {
        let _ = writeln!(out, "only in the baseline: {}", only_before.join(", "));
    }
    Ok(out)
}

fn optional_millis(value: Option<f64>) -> String {
    value.map_or("—".to_owned(), millis)
}

/// The relative change from `before` to `after`; negative means faster.
fn change(before: Option<f64>, after: Option<f64>) -> String {
    match (before, after) {
        (Some(before), Some(after)) if before != 0.0 => {
            format!("{:+.1}%", (after - before) / before * 100.0)
        }
        _ => "—".to_owned(),
    }
}

/// Hyperfine's `--export-json` shape: one result per command. Each engine is
/// a command, so the two engines of one case stay side by side.
fn hyperfine_json(results: &[CaseResult]) -> Json {
    let mut entries = Vec::new();
    for result in results {
        entries.push(hyperfine_result(
            &result.id,
            "pwsh",
            &result.pwsh,
            result.reference_exit,
        ));
        if let Some(summary) = &result.rpsh {
            entries.push(hyperfine_result(
                &result.id,
                "rpsh",
                summary,
                result.reference_exit,
            ));
        }
        if let Some(summary) = &result.bash {
            entries.push(hyperfine_result(
                &result.id,
                "bash",
                summary,
                result.reference_exit,
            ));
        }
    }
    Json::Object(vec![("results", Json::Array(entries))])
}

fn hyperfine_result(id: &str, engine: &str, summary: &Summary, exit_code: i32) -> Json {
    let seconds = |milliseconds: f64| Json::Number(milliseconds / 1000.0);
    Json::Object(vec![
        ("command", Json::text(format!("{id} ({engine})"))),
        ("mean", seconds(summary.mean_ms)),
        ("stddev", seconds(summary.stddev_ms)),
        ("median", seconds(summary.median_ms)),
        ("user", seconds(summary.mean_user_ms)),
        ("system", seconds(summary.mean_system_ms)),
        ("min", seconds(summary.min_ms)),
        ("max", seconds(summary.max_ms)),
        (
            "times",
            Json::Array(summary.samples_ms.iter().copied().map(seconds).collect()),
        ),
        (
            "exit_codes",
            Json::Array(
                summary
                    .samples_ms
                    .iter()
                    .map(|_| Json::Number(exit_code as f64))
                    .collect(),
            ),
        ),
        (
            "parameters",
            Json::Object(vec![("engine", Json::text(engine))]),
        ),
    ])
}

pub(crate) fn load_average() -> Option<Vec<f64>> {
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

pub(crate) fn unix_seconds() -> f64 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_is_signed_and_relative() {
        assert_eq!(change(Some(10.0), Some(9.0)), "-10.0%");
        assert_eq!(change(Some(10.0), Some(12.0)), "+20.0%");
        assert_eq!(change(None, Some(12.0)), "—");
        assert_eq!(change(Some(0.0), Some(12.0)), "—");
    }

    #[test]
    fn compare_reads_a_v1_baseline() {
        let baseline = r#"{
          "schema": "rpsh-bench-v1",
          "cases": [
            {"id": "startup/empty", "pwsh": {"median_ms": 100.0}, "rpsh": {"median_ms": 1.0}},
            {"id": "old/case", "pwsh": {"median_ms": 50.0}, "rpsh": null}
          ]
        }"#;
        let parsed = Value::parse(baseline).unwrap();
        assert_eq!(
            parsed.get("cases").and_then(Value::as_array).unwrap().len(),
            2
        );
        assert_eq!(
            parsed.get("cases").and_then(Value::as_array).unwrap()[0]
                .get("pwsh")
                .and_then(|pwsh| pwsh.get("median_ms"))
                .and_then(Value::as_f64),
            Some(100.0)
        );
    }
}
