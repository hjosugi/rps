//! Finding the `.ps1` cases under a directory.

use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct Case {
    /// Path below the cases directory, without the extension: `startup/empty`.
    pub(crate) id: String,
    pub(crate) path: PathBuf,
}

/// Every `.ps1` file under `directory`, sorted by id. With a filter, only
/// the cases whose id contains it.
pub(crate) fn collect(directory: &Path, filter: Option<&str>) -> Result<Vec<Case>, String> {
    let mut cases = Vec::new();
    walk(directory, directory, &mut cases)?;
    if let Some(filter) = filter {
        cases.retain(|case| case.id.contains(filter));
    }
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cases)
}

fn walk(base: &Path, directory: &Path, cases: &mut Vec<Case>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
            .path();
        if path.is_dir() {
            walk(base, &path, cases)?;
        } else if path.extension().is_some_and(|extension| extension == "ps1") {
            let relative = path.strip_prefix(base).unwrap_or(&path).with_extension("");
            let id = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            cases.push(Case { id, path });
        }
    }
    Ok(())
}

/// The value of `key` in a `# rpsh-bench: key=value ...` line of `source`.
///
/// A benchmark script uses this to ask for fewer runs when one run is slow:
/// `# rpsh-bench: runs=5 warmup=1`.
pub(crate) fn directive(source: &str, key: &str) -> Option<usize> {
    setting(source, key)?.parse().ok()
}

fn setting<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("# rpsh-bench:"))
        .flat_map(str::split_whitespace)
        .filter_map(|setting| setting.split_once('='))
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value)
}

/// A command-mode benchmark passes the script as one argv element. A leading
/// harness directive is metadata, so the exit benchmark executes exactly
/// `rpsh -NoProfile -Command exit`.
pub(crate) fn inline_command(source: &str) -> Result<Option<&str>, String> {
    match setting(source, "mode") {
        None | Some("file") => Ok(None),
        Some("command") => {
            let mut text = source.trim_start();
            while text.starts_with("# rpsh-bench:") {
                text = text.split_once('\n').map_or("", |(_, rest)| rest);
            }
            Ok(Some(text.trim_end_matches(['\r', '\n'])))
        }
        Some(other) => Err(format!("unknown benchmark invocation mode '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_settings_from_a_directive_comment() {
        let source = "# Sums a range.\n# rpsh-bench: runs=5 warmup=1\n$x = 1\n";
        assert_eq!(directive(source, "runs"), Some(5));
        assert_eq!(directive(source, "warmup"), Some(1));
        assert_eq!(directive(source, "other"), None);
        assert_eq!(directive("$x = 1\n", "runs"), None);
    }

    #[test]
    fn command_mode_passes_one_script_argument() {
        assert_eq!(
            inline_command("# rpsh-bench: mode=command runs=30\nexit\n").unwrap(),
            Some("exit")
        );
        assert_eq!(
            inline_command("# rpsh-bench: mode=command\n'one'\n'line two'\n").unwrap(),
            Some("'one'\n'line two'")
        );
        assert_eq!(inline_command("exit").unwrap(), None);
        assert!(inline_command("# rpsh-bench: mode=unknown\nexit").is_err());
    }

    #[test]
    fn startup_cases_carry_the_budget() {
        // DESIGN.md §4 rule 2: the budget is enforced by these cases.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/cases/startup");
        let cases = collect(&root, None).unwrap();
        assert!(!cases.is_empty(), "no startup cases in {}", root.display());
        for case in cases {
            let source = fs::read_to_string(&case.path).unwrap();
            assert_eq!(directive(&source, "budget-ms"), Some(5), "{}", case.id);
        }
    }
}
