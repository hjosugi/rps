//! `cargo harness inventory`: how much of upstream's surface is implemented.
//!
//! `inventory/*.tsv` is generated from the reference pwsh by
//! `scripts/gen-inventory.ps1`: its commands, their parameters, aliases, type
//! accelerators, AST node types and so on. The last two columns are
//! `platform` and `status`; the status is edited by hand as work lands.

use std::collections::BTreeMap;
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

    let mut counts: BTreeMap<String, Vec<(String, [usize; 4])>> = BTreeMap::new();
    for path in &files {
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        for (platform, row) in count(path)? {
            counts
                .entry(platform)
                .or_default()
                .push((name.to_string(), row));
        }
    }

    println!(
        "{:<14}{:<24}{:>7}{:>7}{:>9}{:>7}{:>6}{:>8}",
        "platform", "inventory", "total", "done", "partial", "todo", "n/a", "done %"
    );
    for (platform, rows) in &counts {
        for (name, counts) in rows {
            print_row(platform, name, *counts);
        }
    }
    Ok(true)
}

fn print_row(platform: &str, name: &str, counts: [usize; 4]) {
    let [done, partial, todo, not_applicable] = counts;
    let total = done + partial + todo + not_applicable;
    let applicable = total - not_applicable;
    let percent = if applicable == 0 {
        100.0
    } else {
        done as f64 * 100.0 / applicable as f64
    };
    println!(
        "{platform:<14}{name:<24}{total:>7}{done:>7}{partial:>9}{todo:>7}{not_applicable:>6}{percent:>7.1}%"
    );
}

/// Counts per platform, in the order [`STATUSES`].
fn count(path: &Path) -> Result<BTreeMap<String, [usize; 4]>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    let columns: Vec<&str> = header.split('\t').collect();
    if columns.last() != Some(&"status") {
        return Err(format!(
            "{}: the last column must be `status`",
            path.display()
        ));
    }
    let platform_column = columns.iter().position(|column| *column == "platform");
    let mut per_platform: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    for (index, line) in lines.enumerate() {
        let fields: Vec<&str> = line.split('\t').collect();
        let status = fields.last().copied().unwrap_or("");
        let Some(slot) = STATUSES.iter().position(|known| *known == status) else {
            return Err(format!(
                "{}:{}: unknown status '{status}' (expected one of {})",
                path.display(),
                index + 2,
                STATUSES.join(", ")
            ));
        };
        let platform = platform_column
            .and_then(|column| fields.get(column))
            .copied()
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown");
        per_platform.entry(platform.to_owned()).or_default()[slot] += 1;
    }
    Ok(per_platform)
}
