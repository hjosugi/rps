//! `cargo harness conform`: differential tests against recorded pwsh output.
//!
//! Each case is a script under `tests/conformance/cases`. `record` runs it
//! under the pinned pwsh and stores the exit code and stdout beside it as
//! `<name>.golden`. `check` runs it under rpsh and compares, without needing
//! pwsh, so it runs anywhere the workspace builds.
//!
//! `tests/conformance/passing.txt` lists the cases rpsh passes. `check` fails
//! when the list and reality disagree in either direction: a listed case that
//! no longer passes is a regression, and a case that newly passes has to be
//! added (`--bless`) so that it cannot regress unnoticed later.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rpsh_host::TARGET_PWSH_VERSION;

use crate::Args;
use crate::cases::{self, Case};
use crate::engines::{self, Reference, Verdict};

pub(crate) fn run(root: &Path, args: &[String]) -> Result<bool, String> {
    let mut args = Args::new(args);
    match args.positional().as_deref() {
        Some("record") => record(root, args),
        Some("check") => check(root, args),
        _ => Err("expected `conform record` or `conform check`".to_owned()),
    }
}

fn cases_directory(root: &Path) -> PathBuf {
    root.join("tests/conformance/cases")
}

fn golden_path(case: &Case) -> PathBuf {
    case.path.with_extension("golden")
}

/// A golden file is `exit=<code>`, a line feed, then stdout byte for byte.
fn encode_golden(reference: &Reference) -> Vec<u8> {
    let mut bytes = format!("exit={}\n", reference.exit_code).into_bytes();
    bytes.extend_from_slice(&reference.stdout);
    bytes
}

fn decode_golden(bytes: &[u8]) -> Option<Reference> {
    let line_end = bytes.iter().position(|&byte| byte == b'\n')?;
    let header = std::str::from_utf8(&bytes[..line_end]).ok()?;
    let exit_code = header.strip_prefix("exit=")?.parse().ok()?;
    Some(Reference {
        stdout: bytes[line_end + 1..].to_vec(),
        exit_code,
    })
}

fn record(root: &Path, mut args: Args) -> Result<bool, String> {
    let filter = args.value("--filter")?;
    let pwsh_path = args.value("--pwsh")?;
    args.finish()?;

    let pwsh = engines::locate_pwsh(root, pwsh_path)?;
    let version = engines::pwsh_version(&pwsh)?;
    if version != TARGET_PWSH_VERSION {
        return Err(format!(
            "goldens must come from PowerShell {TARGET_PWSH_VERSION}, found {version}. \
             Run scripts/fetch-pwsh.sh."
        ));
    }

    let cases = cases::collect(&cases_directory(root), filter.as_deref())?;
    let mut changed = 0;
    for case in &cases {
        let run = pwsh.run_file(&case.path)?;
        let Some(exit_code) = run.exit_code else {
            return Err(format!("{}: pwsh was killed by a signal", case.id));
        };
        let golden = encode_golden(&Reference {
            stdout: run.stdout,
            exit_code,
        });
        let path = golden_path(case);
        if fs::read(&path).ok().as_deref() != Some(golden.as_slice()) {
            fs::write(&path, golden)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            println!("recorded {}", case.id);
            changed += 1;
        }
    }
    println!(
        "{} cases recorded from pwsh {version}, {changed} changed",
        cases.len()
    );
    Ok(true)
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
        let path = golden_path(case);
        let golden = fs::read(&path).map_err(|_| {
            format!(
                "{} has no golden; run `cargo harness conform record`",
                case.id
            )
        })?;
        let reference = decode_golden(&golden)
            .ok_or_else(|| format!("{} is not a golden file", path.display()))?;
        match engines::judge(&reference, &rpsh.run_file(&case.path)?) {
            Verdict::Match => {
                passing.insert(case.id.clone());
            }
            Verdict::NotImplemented(feature) => *missing.entry(feature).or_default() += 1,
            Verdict::Mismatch(why) => mismatches.push(format!("{}: {why}", case.id)),
        }
    }

    let refused: usize = missing.values().sum();
    println!(
        "conformance against pwsh {TARGET_PWSH_VERSION}: {} pass, {refused} not implemented, \
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
            exit_code: 255,
        };
        let decoded = decode_golden(&encode_golden(&reference)).unwrap();
        assert_eq!(decoded.exit_code, 255);
        assert_eq!(decoded.stdout, reference.stdout);
    }

    #[test]
    fn an_empty_stdout_is_distinct_from_a_blank_line() {
        assert_eq!(decode_golden(b"exit=0\n").unwrap().stdout, b"");
        assert_eq!(decode_golden(b"exit=0\n\n").unwrap().stdout, b"\n");
    }

    #[test]
    fn rejects_files_without_the_header() {
        assert!(decode_golden(b"").is_none());
        assert!(decode_golden(b"2\n").is_none());
        assert!(decode_golden(b"exit=zero\n").is_none());
    }
}
