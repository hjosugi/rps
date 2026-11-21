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
    source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("# rpsh-bench:"))
        .flat_map(str::split_whitespace)
        .filter_map(|setting| setting.split_once('='))
        .find(|(name, _)| *name == key)
        .and_then(|(_, value)| value.parse().ok())
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
