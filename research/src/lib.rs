//! Measurement helpers shared by the research benches.
//!
//! The benches under `research/benches` are prototypes built to settle a
//! design decision, not parts of rpsh. Each one compares candidate designs on
//! the same workload and writes its numbers to
//! `bench/results/research-<name>.json`; a document under `docs/` cites the
//! copy kept in `bench/baselines/`.
//!
//! Method: one untimed pass, then `SAMPLES` timed samples of `iterations`
//! calls each. The median sample, divided by `iterations`, is reported in
//! nanoseconds per operation; the minimum is kept beside it.

use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

const SAMPLES: usize = 21;
const QUICK_SAMPLES: usize = 3;

/// `RPSH_RESEARCH_QUICK=1`: few samples and fewer iterations, to check that a
/// bench runs. Its numbers are not to be cited.
pub fn quick() -> bool {
    std::env::var_os("RPSH_RESEARCH_QUICK").is_some_and(|value| value != "0")
}

/// Scales an iteration count down in quick mode.
pub fn iterations(full: u64) -> u64 {
    if quick() { (full / 100).max(1) } else { full }
}

/// One measured operation.
#[derive(Debug, Clone)]
pub struct Measurement {
    pub group: String,
    pub candidate: String,
    pub iterations: u64,
    pub median_ns: f64,
    pub min_ns: f64,
    pub samples_ns: Vec<f64>,
}

/// Times `operation`, which is called `iterations` times per sample. The
/// result of each call goes through `black_box` so that it is not optimized
/// away.
pub fn measure<R>(
    group: &str,
    candidate: &str,
    iterations: u64,
    mut operation: impl FnMut() -> R,
) -> Measurement {
    let iterations = iterations.max(1);
    for _ in 0..iterations {
        black_box(operation());
    }
    let samples = if quick() { QUICK_SAMPLES } else { SAMPLES };
    let mut samples_ns = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        samples_ns.push(start.elapsed().as_nanos() as f64 / iterations as f64);
    }
    let mut sorted = samples_ns.clone();
    sorted.sort_by(f64::total_cmp);
    Measurement {
        group: group.to_owned(),
        candidate: candidate.to_owned(),
        iterations,
        median_ns: sorted[sorted.len() / 2],
        min_ns: sorted[0],
        samples_ns,
    }
}

/// The measurements of one bench, printed as a table and saved as JSON.
#[derive(Debug, Default)]
pub struct Report {
    name: String,
    description: String,
    measurements: Vec<Measurement>,
    facts: Vec<(String, String)>,
}

impl Report {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_owned(),
            description: description.to_owned(),
            ..Self::default()
        }
    }

    /// Adds a measurement and prints it at once, so a long bench shows
    /// progress.
    pub fn add(&mut self, measurement: Measurement) {
        println!(
            "{:<28} {:<34} {:>12} {:>12}",
            measurement.group,
            measurement.candidate,
            format_ns(measurement.median_ns),
            format_ns(measurement.min_ns),
        );
        self.measurements.push(measurement);
    }

    /// Records a non-timed observation: a size, a count.
    pub fn fact(&mut self, name: &str, value: impl ToString) {
        let value = value.to_string();
        println!("{name}: {value}");
        self.facts.push((name.to_owned(), value));
    }

    pub fn header(&self) {
        println!("{}: {}", self.name, self.description);
        if quick() {
            println!("QUICK MODE: these numbers are not to be cited");
        }
        println!(
            "{:<28} {:<34} {:>12} {:>12}",
            "operation", "candidate", "median", "min"
        );
    }

    /// Writes `bench/results/research-<name>.json`.
    pub fn save(&self) {
        let root = workspace_root();
        let directory = root.join("bench/results");
        let _ = fs::create_dir_all(&directory);
        let path = directory.join(format!("research-{}.json", self.name));
        let mut json = String::new();
        let _ = writeln!(json, "{{");
        let _ = writeln!(json, "  \"schema\": \"rpsh-research-v1\",");
        let _ = writeln!(json, "  \"name\": {},", quote(&self.name));
        let _ = writeln!(json, "  \"description\": {},", quote(&self.description));
        let _ = writeln!(json, "  \"quick\": {},", quick());
        let _ = writeln!(json, "  \"machine\": {{");
        let machine = machine();
        for (index, (key, value)) in machine.iter().enumerate() {
            let comma = if index + 1 < machine.len() { "," } else { "" };
            let _ = writeln!(json, "    {}: {}{comma}", quote(key), quote(value));
        }
        let _ = writeln!(json, "  }},");
        let _ = writeln!(json, "  \"facts\": {{");
        for (index, (key, value)) in self.facts.iter().enumerate() {
            let comma = if index + 1 < self.facts.len() {
                ","
            } else {
                ""
            };
            let _ = writeln!(json, "    {}: {}{comma}", quote(key), quote(value));
        }
        let _ = writeln!(json, "  }},");
        let _ = writeln!(json, "  \"unit\": \"ns per operation\",");
        let _ = writeln!(json, "  \"measurements\": [");
        for (index, m) in self.measurements.iter().enumerate() {
            let comma = if index + 1 < self.measurements.len() {
                ","
            } else {
                ""
            };
            let samples: Vec<String> = m.samples_ns.iter().map(|ns| format!("{ns:.3}")).collect();
            let _ = writeln!(
                json,
                "    {{\"group\": {}, \"candidate\": {}, \"iterations\": {}, \"median_ns\": {:.3}, \"min_ns\": {:.3}, \"samples_ns\": [{}]}}{comma}",
                quote(&m.group),
                quote(&m.candidate),
                m.iterations,
                m.median_ns,
                m.min_ns,
                samples.join(", ")
            );
        }
        let _ = writeln!(json, "  ]");
        let _ = writeln!(json, "}}");
        match fs::write(&path, json) {
            Ok(()) => println!("wrote {}", path.display()),
            Err(error) => eprintln!("cannot write {}: {error}", path.display()),
        }
    }
}

fn format_ns(ns: f64) -> String {
    if ns >= 1_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.2} us", ns / 1_000.0)
    } else {
        format!("{ns:.2} ns")
    }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn workspace_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    root.canonicalize().unwrap_or(root)
}

fn machine() -> Vec<(String, String)> {
    let read = |path: &str| {
        fs::read_to_string(path)
            .ok()
            .map(|text| text.trim().to_owned())
    };
    let cpu = read("/proc/cpuinfo").and_then(|info| {
        info.lines()
            .find(|line| line.starts_with("model name"))
            .and_then(|line| line.split_once(':'))
            .map(|(_, model)| model.trim().to_owned())
    });
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(workspace_root())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    let rustc = Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    [
        ("cpu", cpu),
        (
            "governor",
            read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        ),
        ("load_average", read("/proc/loadavg")),
        ("kernel", read("/proc/sys/kernel/osrelease")),
        ("commit", commit),
        ("rustc", rustc),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.unwrap_or_default()))
    .collect()
}
