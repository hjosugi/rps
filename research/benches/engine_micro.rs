//! #11: in-process micro benchmarks for parse, compile and invoke.
//!
//! Upstream keeps three BenchmarkDotNet benchmarks (issue #11):
//! `Engine.Parser.cs` parses a small `using` script, `Engine.Compiler.cs`
//! compiles each function body in `assets/compiler.test.ps1`, and
//! `Engine.ScriptBlock.cs` invokes a pre-created script block.
//!
//! There is no .NET SDK in the reference environment and no BenchmarkDotNet,
//! so the upstream column is produced by the reference pwsh itself: one process,
//! a stopwatch, and nine samples per operation (`Measure-Command` style). It
//! is an estimate, not a BenchmarkDotNet result; the bench says so.
//!
//! rpsh's parser and engine are measured in process. Compile has no rpsh row
//! yet: the register IR is M3, and the engine lowers the AST itself when it
//! runs. The row is printed as not implemented rather than approximated.
//! The parser can only parse the walking skeleton's grammar today, so the
//! same-input parse comparison uses a generated 100 KiB subset script;
//! `compiler.test.ps1` is still parsed by pwsh and refused by name by rpsh.

use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use rpsh_engine::Engine;
use rpsh_research::{Measurement, Report, iterations, measure};
use rpsh_syntax::parse;

const SUBSET_BYTES: usize = 100_000;
const SUBSET_LINE: &str = "1 + 2 * (3 - 4)\n";
const INVOKE_SOURCE: &str = "1 + 2 * 3";

/// Iteration counts the pwsh script uses. They are repeated here so the
/// result file can say what was measured.
const PWSH_PARSE_ITERATIONS: u64 = 20;
const PWSH_COMPILE_ITERATIONS: u64 = 5;
const PWSH_INVOKE_ITERATIONS: u64 = 100_000;
const PWSH_SAMPLES: usize = 9;

fn main() {
    let root = workspace_root();
    let asset_path = root.join("bench/assets/compiler.test.ps1");
    let asset = fs::read_to_string(&asset_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", asset_path.display()));
    let subset = subset();
    let subset_path = root.join("bench/results/engine-micro-subset.ps1");
    if let Some(directory) = subset_path.parent() {
        fs::create_dir_all(directory).unwrap_or_else(|error| {
            panic!("cannot create {}: {error}", directory.display());
        });
    }
    fs::write(&subset_path, &subset)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", subset_path.display()));

    let mut report = Report::new(
        "engine_micro",
        "in-process parse, compile and invoke against the reference pwsh (#11)",
    );
    report.header();
    report.fact("compiler.test.ps1 bytes", asset.len());
    report.fact("subset bytes", subset.len());
    report.fact("subset statements", subset.lines().count());
    report.fact(
        "pwsh method",
        "stopwatch loops in one pwsh process, not BenchmarkDotNet",
    );

    let rpsh_parse = measure("parse subset", "rpsh", iterations(100), || {
        black_box(parse(black_box(&subset)).expect("the subset is valid"));
    });
    report.add(rpsh_parse.clone());
    report.fact(
        "rpsh parse throughput",
        format!("{:.2} MB/s", throughput(subset.len(), rpsh_parse.median_ns)),
    );

    let refusal = parse(&asset).map_or_else(
        |error| {
            format!(
                "not implemented: {} (at offset {})",
                error.feature(),
                error.span().start
            )
        },
        |_| "parsed".to_owned(),
    );
    report.fact("rpsh parse of compiler.test.ps1", refusal.clone());

    let invoke_ast = parse(INVOKE_SOURCE).expect("the invoke source is valid");
    let mut engine = Engine::new();
    let mut sink = io::sink();
    let rpsh_invoke = measure("invoke 1 + 2 * 3", "rpsh", iterations(100_000), || {
        engine
            .run_ast(black_box(&invoke_ast), &mut sink)
            .expect("the invoke source does not fail");
    });
    report.add(rpsh_invoke.clone());

    report.fact(
        "rpsh compile",
        "not implemented: there is no compiler or register IR yet (DESIGN.md §3.4, M3)",
    );

    let pwsh = locate_pwsh(&root);
    let pwsh_label = pwsh.as_deref().and_then(pwsh_version);
    if let Some(label) = &pwsh_label {
        report.fact("pwsh version", label);
    }
    let upstream = pwsh.as_deref().and_then(|pwsh| {
        measure_upstream(pwsh, &asset_path, &subset_path)
            .map_err(|error| eprintln!("warning: {error}"))
            .ok()
    });
    if let Some(upstream) = &upstream {
        report.add(upstream.parse_asset.measurement());
        report.add(upstream.parse_subset.measurement());
        report.add(upstream.compile_asset.measurement());
        report.add(upstream.invoke.measurement());
        report.fact(
            "pwsh parse throughput",
            format!(
                "{:.2} MB/s",
                throughput(subset.len(), upstream.parse_subset.median_ns)
            ),
        );
    } else if pwsh.is_none() {
        eprintln!(
            "warning: no pwsh found; the upstream column is omitted. \
             Run scripts/fetch-pwsh.sh."
        );
    }

    comparison(
        &rpsh_parse,
        &rpsh_invoke,
        upstream.as_ref(),
        &refusal,
        pwsh_label.as_deref().unwrap_or("latest stable"),
    );
    report.save();
}

fn subset() -> String {
    let mut text = String::with_capacity(SUBSET_BYTES + SUBSET_LINE.len());
    while text.len() < SUBSET_BYTES {
        text.push_str(SUBSET_LINE);
    }
    text
}

fn throughput(bytes: usize, median_ns: f64) -> f64 {
    bytes as f64 / median_ns * 1e9 / (1024.0 * 1024.0)
}

/// A measurement reported by the pwsh script: median and minimum ns/op.
struct Upstream {
    name: &'static str,
    iterations: u64,
    median_ns: f64,
    min_ns: f64,
}

impl Upstream {
    fn measurement(&self) -> Measurement {
        Measurement {
            group: self.name.to_owned(),
            candidate: "pwsh".to_owned(),
            iterations: self.iterations,
            median_ns: self.median_ns,
            min_ns: self.min_ns,
            samples_ns: Vec::new(),
        }
    }
}

struct UpstreamMeasurements {
    parse_asset: Upstream,
    parse_subset: Upstream,
    compile_asset: Upstream,
    invoke: Upstream,
}

fn measure_upstream(
    pwsh: &Path,
    asset: &Path,
    subset: &Path,
) -> Result<UpstreamMeasurements, String> {
    let script = upstream_script(asset, subset);
    let output = Command::new(pwsh)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|error| format!("cannot run {}: {error}", pwsh.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            pwsh.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut values = Vec::new();
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 3 {
            continue;
        }
        let Ok(median_ns) = fields[1].parse::<f64>() else {
            continue;
        };
        let Ok(min_ns) = fields[2].parse::<f64>() else {
            continue;
        };
        values.push((fields[0].to_owned(), median_ns, min_ns));
    }
    let find = |name: &str, iterations: u64| {
        values
            .iter()
            .find(|(candidate, _, _)| candidate == name)
            .map(|(_, median, min)| Upstream {
                name: match name {
                    "parse-asset" => "parse compiler.test.ps1",
                    "parse-subset" => "parse subset",
                    "compile-asset" => "compile compiler.test.ps1",
                    "invoke" => "invoke 1 + 2 * 3",
                    _ => "other",
                },
                iterations,
                median_ns: *median,
                min_ns: *min,
            })
            .ok_or_else(|| format!("pwsh did not report {name}"))
    };
    Ok(UpstreamMeasurements {
        parse_asset: find("parse-asset", PWSH_PARSE_ITERATIONS)?,
        parse_subset: find("parse-subset", PWSH_PARSE_ITERATIONS)?,
        compile_asset: find("compile-asset", PWSH_COMPILE_ITERATIONS)?,
        invoke: find("invoke", PWSH_INVOKE_ITERATIONS)?,
    })
}

fn upstream_script(asset: &Path, subset: &Path) -> String {
    let mut script = String::new();
    let _ = writeln!(script, "$ErrorActionPreference = 'Stop'");
    let _ = writeln!(
        script,
        "$asset = [IO.File]::ReadAllText({})",
        ps_string(asset)
    );
    let _ = writeln!(
        script,
        "$subset = [IO.File]::ReadAllText({})",
        ps_string(subset)
    );
    let _ = writeln!(
        script,
        r#"
function Emit([string] $name, $times) {{
    $sorted = @($times | Sort-Object)
    $median = $sorted[[int]($sorted.Count / 2)]
    $min = $sorted[0]
    "{{0}}`t{{1}}`t{{2}}" -f $name,
        $median.ToString('R', [Globalization.CultureInfo]::InvariantCulture),
        $min.ToString('R', [Globalization.CultureInfo]::InvariantCulture)
}}

$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseInput($asset, [ref] $tokens, [ref] $errors)
$times = @()
for ($s = 0; $s -lt {samples}; $s++) {{
    $sw = [Diagnostics.Stopwatch]::StartNew()
    for ($i = 0; $i -lt {parse}; $i++) {{
        $null = [System.Management.Automation.Language.Parser]::ParseInput($asset, [ref] $tokens, [ref] $errors)
    }}
    $sw.Stop()
    $times += $sw.Elapsed.TotalMilliseconds * 1e6 / {parse}
}}
Emit 'parse-asset' $times

$null = [System.Management.Automation.Language.Parser]::ParseInput($subset, [ref] $tokens, [ref] $errors)
$times = @()
for ($s = 0; $s -lt {samples}; $s++) {{
    $sw = [Diagnostics.Stopwatch]::StartNew()
    for ($i = 0; $i -lt {parse}; $i++) {{
        $null = [System.Management.Automation.Language.Parser]::ParseInput($subset, [ref] $tokens, [ref] $errors)
    }}
    $sw.Stop()
    $times += $sw.Elapsed.TotalMilliseconds * 1e6 / {parse}
}}
Emit 'parse-subset' $times

$compilerType = [System.Management.Automation.PSObject].Assembly.GetType('System.Management.Automation.CompiledScriptBlockData')
$flags = [Reflection.BindingFlags]'Instance,NonPublic,Public'
$ctor = $compilerType.GetConstructor($flags, $null, [Type[]]@([System.Management.Automation.Language.ScriptBlockAst], [bool]), $null)
$compileMethod = $compilerType.GetMethod('Compile', $flags)
$bodies = @($ast.FindAll({{ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }}, $false) | ForEach-Object {{ $_.Body }})
function Compile-All {{
    foreach ($body in $bodies) {{
        $data = $ctor.Invoke(@($body, $false))
        $null = $compileMethod.Invoke($data, @($true))
    }}
}}
Compile-All
$times = @()
for ($s = 0; $s -lt {samples}; $s++) {{
    $sw = [Diagnostics.Stopwatch]::StartNew()
    for ($i = 0; $i -lt {compile}; $i++) {{ Compile-All }}
    $sw.Stop()
    $times += $sw.Elapsed.TotalMilliseconds * 1e6 / {compile}
}}
Emit 'compile-asset' $times

$sb = [scriptblock]::Create('{invoke_source}')
$null = $sb.Invoke()
$times = @()
for ($s = 0; $s -lt {samples}; $s++) {{
    $sw = [Diagnostics.Stopwatch]::StartNew()
    for ($i = 0; $i -lt {invoke}; $i++) {{ $null = $sb.Invoke() }}
    $sw.Stop()
    $times += $sw.Elapsed.TotalMilliseconds * 1e6 / {invoke}
}}
Emit 'invoke' $times
"#,
        samples = PWSH_SAMPLES,
        parse = PWSH_PARSE_ITERATIONS,
        compile = PWSH_COMPILE_ITERATIONS,
        invoke = PWSH_INVOKE_ITERATIONS,
        invoke_source = INVOKE_SOURCE,
    );
    script
}

fn ps_string(path: &Path) -> String {
    let text = path.display().to_string().replace('\'', "''");
    format!("'{text}'")
}

fn locate_pwsh(root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RPSH_PWSH").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    let cache = root.join(".cache/pwsh");
    if let Ok(text) = fs::read_to_string(cache.join("current")) {
        let version = text.trim().trim_start_matches('v');
        let cached = cache.join(version).join("pwsh");
        if cached.is_file() {
            return Some(cached);
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
    best.map(|(_, path)| path).or_else(|| {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|directory| directory.join("pwsh"))
                .find(|candidate| candidate.is_file())
        })
    })
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

fn pwsh_version(pwsh: &Path) -> Option<String> {
    let output = Command::new(pwsh)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PSVersionTable.PSVersion.ToString()",
        ])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn comparison(
    rpsh_parse: &Measurement,
    rpsh_invoke: &Measurement,
    upstream: Option<&UpstreamMeasurements>,
    refusal: &str,
    pwsh_label: &str,
) {
    let parse_asset = upstream.map(|values| values.parse_asset.median_ns);
    let parse_subset = upstream.map(|values| values.parse_subset.median_ns);
    let compile_asset = upstream.map(|values| values.compile_asset.median_ns);
    let invoke = upstream.map(|values| values.invoke.median_ns);

    println!(
        "\n{:<28} {:>16} {:>16} {:>10}",
        "operation",
        "rpsh",
        format!("pwsh {pwsh_label}"),
        "speedup"
    );
    print_row("parse compiler.test.ps1", None, parse_asset);
    print_row(
        "parse subset (100 KiB)",
        Some(rpsh_parse.median_ns),
        parse_subset,
    );
    print_row("compile compiler.test.ps1", None, compile_asset);
    print_row("invoke 1 + 2 * 3", Some(rpsh_invoke.median_ns), invoke);
    println!("\nnotes:");
    println!("  rpsh parse of compiler.test.ps1: {refusal}");
    println!("  rpsh compile: not implemented (no compiler or register IR yet, M3)");
    println!("  pwsh: stopwatch loops in one process, not BenchmarkDotNet");
}

fn print_row(operation: &str, rpsh: Option<f64>, pwsh: Option<f64>) {
    let rpsh_text = rpsh.map_or("—".to_owned(), format_ns);
    let pwsh_text = pwsh.map_or("—".to_owned(), format_ns);
    let speedup = match (rpsh, pwsh) {
        (Some(rpsh), Some(pwsh)) if rpsh > 0.0 => format!("{:.2}x", pwsh / rpsh),
        _ => "—".to_owned(),
    };
    println!("{operation:<28} {rpsh_text:>16} {pwsh_text:>16} {speedup:>10}");
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

fn workspace_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    root.canonicalize().unwrap_or(root)
}
