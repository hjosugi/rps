//! `cargo harness inventory`: how much of upstream's surface is implemented.
//!
//! `inventory/*.tsv` is generated from the pinned pwsh by
//! `scripts/gen-inventory.ps1`: its commands, their parameters, aliases, type
//! accelerators, AST node types and so on. The last column of every file is
//! `status`, edited by hand as work lands.

use std::fs;
use std::path::Path;

use crate::Args;

const STATUSES: [&str; 4] = ["done", "partial", "todo", "n/a"];

pub(crate) fn run(root: &Path, args: &[String]) -> Result<bool, String> {
    Args::new(args).finish()?;
    let directory = root.join("inventory");
    let mut files: Vec<_> = fs::read_dir(&directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "tsv"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err("inventory/ has no .tsv files; run scripts/gen-inventory.ps1".to_owned());
    }

    println!(
        "{:<24}{:>7}{:>7}{:>9}{:>7}{:>6}{:>8}",
        "inventory", "total", "done", "partial", "todo", "n/a", "done %"
    );
    for path in &files {
        let counts = count(path)?;
        let [done, partial, todo, not_applicable] = counts;
        let total = done + partial + todo + not_applicable;
        let applicable = total - not_applicable;
        let percent = if applicable == 0 {
            100.0
        } else {
            done as f64 * 100.0 / applicable as f64
        };
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        println!(
            "{name:<24}{total:>7}{done:>7}{partial:>9}{todo:>7}{not_applicable:>6}{percent:>7.1}%"
        );
    }
    Ok(true)
}

/// Rows per status, in the order of [`STATUSES`].
fn count(path: &Path) -> Result<[usize; 4], String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    if header.rsplit('\t').next() != Some("status") {
        return Err(format!(
            "{}: the last column must be `status`",
            path.display()
        ));
    }
    let mut counts = [0; 4];
    for (index, line) in lines.enumerate() {
        let status = line.rsplit('\t').next().unwrap_or("");
        let Some(slot) = STATUSES.iter().position(|known| *known == status) else {
            return Err(format!(
                "{}:{}: unknown status '{status}' (expected one of {})",
                path.display(),
                index + 2,
                STATUSES.join(", ")
            ));
        };
        counts[slot] += 1;
    }
    Ok(counts)
}
