//! `cargo harness conform`: differential tests against recorded pwsh output.
//!
//! Each case is a script under `tests/conformance/cases`. `record` runs it
//! under the reference pwsh and stores the exit code, stdout and stderr
//! beside it as `<name>.golden` and, when it is not empty, `<name>.stderr`.
//! `check` runs it under rpsh and compares, without needing pwsh, so it runs
//! anywhere the workspace builds.
//!
//! Each case declares how much is compared with
//! `# rpsh-conformance: level=c1|c2`:
//!
//! * `c1`: exit code and stdout, byte for byte.
//! * `c2`: `c1` plus stderr, byte for byte, after the case's directory is
//!   replaced by `<case>` in both.
//!
//! A case whose output differs between two pwsh runs cannot have a golden:
//! `record` runs every case several times and refuses such a case instead of
//! recording one of its answers (docs/01_compat_contract.md, "What does not
//! repeat").
//!
//! `tests/conformance/passing.txt` lists the cases rpsh passes. `check` fails
//! when the list and reality disagree in either direction: a listed case that
//! no longer passes is a regression, and a case that newly passes has to be
//! added (`--bless`) so that it cannot regress unnoticed later.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::Args;
use crate::cases::{self, Case};
use crate::engines::{self, Reference, Run, Verdict};

const GOLDEN_HEADER: &[u8] = b"rpsh-golden-v2\n";

pub(crate) fn run(root: &Path, args: &[String]) -> Result<bool, String> {
    let mut args = Args::new(args);
    match args.positional().as_deref() {
        Some("record") => record(root, args),
        Some("check") => check(root, args),
        Some("pty") => crate::pty::run(root, args),
        _ => Err("expected `conform record`, `conform check` or `conform pty`".to_owned()),
    }
}

/// The pwsh version the goldens were recorded against.
fn recorded_version(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("tests/conformance/reference.txt")).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("powershell: "))
        .map(|version| version.trim().to_owned())
}

fn cases_directory(root: &Path) -> PathBuf {
    root.join("tests/conformance/cases")
}

fn golden_path(case: &Case) -> PathBuf {
    case.path.with_extension("golden")
}

fn stderr_path(case: &Case) -> PathBuf {
    case.path.with_extension("stderr")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    C1,
    C2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaseSettings<'a> {
    level: Level,
    locale: Option<&'a str>,
}

/// Language selection belongs to each engine process, just like the home and
/// working directory. The harness process keeps its own locale.
fn case_settings<'a>(source: &'a str, id: &str) -> Result<CaseSettings<'a>, String> {
    let mut level = None;
    let mut locale = None;
    for line in source.lines() {
        let Some(rest) = line.trim().strip_prefix("# rpsh-conformance:") else {
            continue;
        };
        for setting in rest.split_whitespace() {
            let Some((key, value)) = setting.split_once('=') else {
                continue;
            };
            match key {
                "level" => {
                    level = Some(match value {
                        "c1" => Level::C1,
                        "c2" => Level::C2,
                        other => {
                            return Err(format!(
                                "{id}: unknown conformance level '{other}' (c1 | c2)"
                            ));
                        }
                    })
                }
                "LANG" | "lang" if value.is_empty() => {
                    return Err(format!("{id}: LANG requires a locale name"));
                }
                "LANG" | "lang" => locale = Some(value),
                _ => {}
            }
        }
    }
    let level = level
        .ok_or_else(|| format!("{id}: add `# rpsh-conformance: level=c2` (or c1) to the case"))?;
    Ok(CaseSettings { level, locale })
}

/// A golden v2 is a header line, `exit=<code>`, then stdout byte for byte.
fn encode_golden(reference: &Reference) -> Vec<u8> {
    let mut bytes = GOLDEN_HEADER.to_vec();
    bytes.extend_from_slice(format!("exit={}\n", reference.exit_code).as_bytes());
    bytes.extend_from_slice(&reference.stdout);
    bytes
}

fn decode_golden(bytes: &[u8]) -> Option<Reference> {
    let body = bytes.strip_prefix(GOLDEN_HEADER)?;
    let line_end = body.iter().position(|&byte| byte == b'\n')?;
    let header = std::str::from_utf8(&body[..line_end]).ok()?;
    let exit_code = header.strip_prefix("exit=")?.parse().ok()?;
    Some(Reference {
        stdout: body[line_end + 1..].to_vec(),
        stderr: None,
        exit_code,
    })
}

/// Replaces the case's own directory, which is machine-specific, with
/// `<case>`. Line numbers and the rest of the text are untouched.
fn normalize(bytes: &[u8], directory: &Path) -> Vec<u8> {
    let needle = directory.as_os_str().as_encoded_bytes();
    if needle.is_empty() {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some(position) = rest
        .windows(needle.len())
        .position(|window| window == needle)
    {
        out.extend_from_slice(&rest[..position]);
        out.extend_from_slice(b"<case>");
        rest = &rest[position + needle.len()..];
    }
    out.extend_from_slice(rest);
    out
}

fn record(root: &Path, mut args: Args) -> Result<bool, String> {
    let filter = args.value("--filter")?;
    let pwsh_path = args.value("--pwsh")?;
    let repeat = args.number("--repeat")?.unwrap_or(3).max(2);
    args.finish()?;

    let pwsh = engines::locate_pwsh(root, pwsh_path)?;
    let version = engines::pwsh_version(&pwsh)?;

    let cases = cases::collect(&cases_directory(root), filter.as_deref())?;
    let mut changed = 0;
    let mut unstable = Vec::new();
    for case in &cases {
        let source = fs::read_to_string(&case.path)
            .map_err(|error| format!("cannot read {}: {error}", case.path.display()))?;
        let CaseSettings { level, locale } = case_settings(&source, &case.id)?;
        let directory = case.path.parent().unwrap_or(Path::new("."));
        let mut answers = Vec::with_capacity(repeat);
        for _ in 0..repeat {
            let run = pwsh.run_file(&case.path, locale)?;
            let Some(exit_code) = run.exit_code else {
                return Err(format!("{}: pwsh was killed by a signal", case.id));
            };
            answers.push((
                exit_code,
                normalize(&run.stdout, directory),
                normalize(&run.stderr, directory),
            ));
        }
        if let Some(why) = disagreement(&answers) {
            unstable.push(format!("{}: {why}", case.id));
            continue;
        }
        let (exit_code, stdout, stderr) = answers.swap_remove(0);
        let reference = Reference {
            stdout,
            stderr: None,
            exit_code,
        };
        let path = golden_path(case);
        let golden = encode_golden(&reference);
        if fs::read(&path).ok().as_deref() != Some(golden.as_slice()) {
            fs::write(&path, golden)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            println!("recorded {} ({level:?})", case.id);
            changed += 1;
        }
        let stderr_path = stderr_path(case);
        if stderr.is_empty() {
            if stderr_path.exists() {
                fs::remove_file(&stderr_path)
                    .map_err(|error| format!("cannot remove {}: {error}", stderr_path.display()))?;
                changed += 1;
            }
        } else if fs::read(&stderr_path).ok().as_deref() != Some(stderr.as_slice()) {
            fs::write(&stderr_path, stderr)
                .map_err(|error| format!("cannot write {}: {error}", stderr_path.display()))?;
            changed += 1;
        }
    }

    let reference = engines::pwsh_reference(&pwsh)?;
    let reference_path = root.join("tests/conformance/reference.txt");
    let text = format!(
        "# The environment the goldens were recorded in. Written by \
         `cargo harness conform record`.\n{reference}"
    );
    if fs::read_to_string(&reference_path).ok().as_deref() != Some(text.as_str()) {
        fs::write(&reference_path, text)
            .map_err(|error| format!("cannot write {}: {error}", reference_path.display()))?;
        println!("reference.txt updated:\n{reference}");
    }

    println!(
        "{} cases recorded from pwsh {version} ({repeat} runs each), {changed} changed",
        cases.len() - unstable.len()
    );
    for case in &unstable {
        println!("UNSTABLE {case}");
    }
    if !unstable.is_empty() {
        println!(
            "pwsh's own answer to these cases changes from run to run, so no golden was \
             written. Make the script deterministic (docs/01_compat_contract.md)."
        );
    }
    Ok(unstable.is_empty())
}

/// Why repeated runs of one case do not agree, if they do not.
fn disagreement(answers: &[(i32, Vec<u8>, Vec<u8>)]) -> Option<String> {
    let first = answers.first()?;
    let (index, other) = answers
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, answer)| *answer != first)?;
    let (exit, stdout, stderr) = first;
    if other.0 != *exit {
        return Some(format!(
            "run 1 exits with {exit} and run {} with {}",
            index + 1,
            other.0
        ));
    }
    if other.1 != *stdout {
        let at = first_difference(stdout, &other.1);
        return Some(format!(
            "run 1 and run {} differ at byte {at} of stdout",
            index + 1
        ));
    }
    let at = first_difference(stderr, &other.2);
    Some(format!(
        "run 1 and run {} differ at byte {at} of stderr",
        index + 1
    ))
}

fn first_difference(ours: &[u8], theirs: &[u8]) -> usize {
    ours.iter()
        .zip(theirs)
        .position(|(ours, theirs)| ours != theirs)
        .unwrap_or_else(|| ours.len().min(theirs.len()))
}

fn check(root: &Path, mut args: Args) -> Result<bool, String> {
    let bless = args.flag("--bless");
    let filter = args.value("--filter")?;
    let rpsh_path = args.value("--rpsh")?;
    args.finish()?;

    let rpsh = engines::rpsh(root, rpsh_path)?;
    let cases = cases::collect(&cases_directory(root), filter.as_deref())?;
    if cases.is_empty() {
        return Err("no conformance case matches".to_owned());
    }

    let mut passing = BTreeSet::new();
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    let mut mismatches = Vec::new();
    for case in &cases {
        let source = fs::read_to_string(&case.path)
            .map_err(|error| format!("cannot read {}: {error}", case.path.display()))?;
        let CaseSettings { level, locale } = case_settings(&source, &case.id)?;
        let path = golden_path(case);
        let golden = fs::read(&path).map_err(|_| {
            format!(
                "{} has no golden; run `cargo harness conform record`",
                case.id
            )
        })?;
        let mut reference = decode_golden(&golden).ok_or_else(|| {
            format!(
                "{} is not a v2 golden; run `cargo harness conform record`",
                path.display()
            )
        })?;
        if level == Level::C2 {
            reference.stderr = Some(fs::read(stderr_path(case)).unwrap_or_default());
        }

        let run = rpsh.run_file(&case.path, locale)?;
        let directory = case.path.parent().unwrap_or(Path::new("."));
        let run = Run {
            stdout: normalize(&run.stdout, directory),
            stderr: normalize(&run.stderr, directory),
            exit_code: run.exit_code,
            elapsed: run.elapsed,
            metrics: run.metrics,
        };
        match engines::judge(&reference, &run) {
            Verdict::Match => {
                passing.insert(case.id.clone());
            }
            Verdict::NotImplemented(feature) => *missing.entry(feature).or_default() += 1,
            Verdict::Mismatch(why) => mismatches.push(format!("{}: {why}", case.id)),
        }
    }

    let refused: usize = missing.values().sum();
    let version = recorded_version(root).unwrap_or_else(|| "the recorded reference".to_owned());
    println!(
        "conformance against pwsh {version}: {} pass, {refused} not implemented, \
         {} fail ({} cases)",
        passing.len(),
        mismatches.len(),
        cases.len()
    );
    if !missing.is_empty() {
        let mut missing: Vec<(String, usize)> = missing.into_iter().collect();
        missing.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let list: Vec<String> = missing
            .iter()
            .map(|(feature, count)| format!("{feature} ×{count}"))
            .collect();
        println!("not implemented: {}", list.join(", "));
    }
    for mismatch in &mismatches {
        println!("FAIL {mismatch}");
    }

    // With a filter only part of the suite ran, so the list is left alone.
    let mut ok = mismatches.is_empty();
    if filter.is_none() {
        let list_path = root.join("tests/conformance/passing.txt");
        let listed = read_passing(&list_path)?;
        for regression in listed.difference(&passing) {
            println!("FAIL {regression}: listed in passing.txt but does not pass");
            ok = false;
        }
        let newly: Vec<&String> = passing.difference(&listed).collect();
        if !newly.is_empty() && ok && bless {
            write_passing(&list_path, &passing)?;
            println!("passing.txt updated: {} newly passing", newly.len());
        } else if !newly.is_empty() {
            for case in newly {
                println!("NEW  {case}: passes but is not in passing.txt");
            }
            println!("run `cargo harness conform check --bless` to record them");
            ok = false;
        }
    }
    Ok(ok)
}

fn read_passing(path: &Path) -> Result<BTreeSet<String>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

fn write_passing(path: &Path, passing: &BTreeSet<String>) -> Result<(), String> {
    let mut text = String::from(
        "# Conformance cases rpsh passes. Maintained by `cargo harness conform check --bless`.\n",
    );
    for case in passing {
        text.push_str(case);
        text.push('\n');
    }
    fs::write(path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_golden_round_trips_stdout_byte_for_byte() {
        let reference = Reference {
            stdout: b"a\r\n\nexit=1\n\xff".to_vec(),
            stderr: None,
            exit_code: 255,
        };
        let decoded = decode_golden(&encode_golden(&reference)).unwrap();
        assert_eq!(decoded.exit_code, 255);
        assert_eq!(decoded.stdout, reference.stdout);
    }

    #[test]
    fn an_empty_stdout_is_distinct_from_a_blank_line() {
        assert_eq!(
            decode_golden(b"rpsh-golden-v2\nexit=0\n").unwrap().stdout,
            b""
        );
        assert_eq!(
            decode_golden(b"rpsh-golden-v2\nexit=0\n\n").unwrap().stdout,
            b"\n"
        );
    }

    #[test]
    fn repeated_runs_must_agree() {
        let same = vec![
            (0, b"a\n".to_vec(), Vec::new()),
            (0, b"a\n".to_vec(), Vec::new()),
        ];
        assert_eq!(disagreement(&same), None);
        let order = vec![
            (0, b"alpha,beta\n".to_vec(), Vec::new()),
            (0, b"alpha,beta\n".to_vec(), Vec::new()),
            (0, b"beta,alpha\n".to_vec(), Vec::new()),
        ];
        assert_eq!(
            disagreement(&order).as_deref(),
            Some("run 1 and run 3 differ at byte 0 of stdout")
        );
        let stderr = vec![
            (0, b"a\n".to_vec(), b"warn 1\n".to_vec()),
            (0, b"a\n".to_vec(), b"warn 2\n".to_vec()),
        ];
        assert_eq!(
            disagreement(&stderr).as_deref(),
            Some("run 1 and run 2 differ at byte 5 of stderr")
        );
    }

    #[test]
    fn rejects_files_without_the_v2_header() {
        assert!(decode_golden(b"").is_none());
        assert!(decode_golden(b"exit=0\n").is_none());
        assert!(decode_golden(b"rpsh-golden-v2\n2\n").is_none());
        assert!(decode_golden(b"rpsh-golden-v2\nexit=zero\n").is_none());
    }

    #[test]
    fn normalizes_the_case_directory_and_keeps_line_numbers() {
        let directory = Path::new("/work/tests/conformance/cases/contract");
        let input = b"/work/tests/conformance/cases/contract/foo.ps1:3: oops\n";
        let output = normalize(input, directory);
        assert_eq!(output, b"<case>/foo.ps1:3: oops\n");
    }

    #[test]
    fn reads_the_conformance_level_directive() {
        assert_eq!(
            case_settings("# rpsh-conformance: level=c2\n", "x")
                .unwrap()
                .level,
            Level::C2
        );
        assert_eq!(
            case_settings("# rpsh-conformance: level=c1\n", "x")
                .unwrap()
                .level,
            Level::C1
        );
        assert_eq!(
            case_settings("# rpsh-conformance: LANG=tr_TR.UTF-8 level=c2", "x").unwrap(),
            CaseSettings {
                level: Level::C2,
                locale: Some("tr_TR.UTF-8")
            }
        );
        assert_eq!(
            case_settings("# rpsh-conformance: level=c1", "x")
                .unwrap()
                .locale,
            None
        );
        assert!(case_settings("# rpsh-conformance: level=c2 LANG=", "x").is_err());
        assert!(case_settings("# nothing\n", "x").is_err());
        assert!(case_settings("# rpsh-conformance: level=c9\n", "x").is_err());
    }
}
