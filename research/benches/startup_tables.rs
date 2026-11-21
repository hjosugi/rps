//! #5: what it costs to build the command table eagerly at startup, and to
//! look names up in the alternatives that need no construction.
//!
//! The data is upstream's own surface from `inventory/`: 298 commands, 124
//! aliases, 102 type accelerators and 2,076 parameters. Candidates:
//!
//! * eager map: a `HashMap` keyed by the lowercased name, built when the
//!   process starts (what a straightforward port, and Nushell's
//!   `add_shell_command_context`, do);
//! * eager metadata: every command's parameter metadata built as owned
//!   structures, the way a registry of `Box<dyn Command>` with signatures
//!   would be;
//! * static sorted table: names sorted at build time, binary search with an
//!   ASCII case-insensitive comparison; nothing to construct at run time.
//!
//! Construction is timed whole; lookups are timed per name.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::hint::black_box;
use std::path::Path;

use rpsh_research::{Report, iterations, measure};

struct Inventory {
    commands: Vec<String>,
    aliases: Vec<(String, String)>,
    accelerators: Vec<(String, String)>,
    parameters: Vec<ParameterRow>,
}

#[derive(Clone)]
struct ParameterRow {
    command: String,
    name: String,
    type_name: String,
    sets: String,
    aliases: String,
}

fn rows(path: &Path) -> Vec<Vec<String>> {
    let text = fs::read_to_string(path).unwrap_or_else(|error| {
        panic!("cannot read {}: {error}", path.display());
    });
    text.lines()
        .skip(1)
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect()
}

fn load() -> Inventory {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../inventory");
    Inventory {
        commands: rows(&root.join("commands.tsv"))
            .into_iter()
            .map(|row| row[1].clone())
            .collect(),
        aliases: rows(&root.join("aliases.tsv"))
            .into_iter()
            .map(|row| (row[0].clone(), row[1].clone()))
            .collect(),
        accelerators: rows(&root.join("type-accelerators.tsv"))
            .into_iter()
            .map(|row| (row[0].clone(), row[1].clone()))
            .collect(),
        parameters: rows(&root.join("parameters.tsv"))
            .into_iter()
            .map(|row| ParameterRow {
                command: row[0].clone(),
                name: row[1].clone(),
                type_name: row[2].clone(),
                sets: row[3].clone(),
                aliases: row[4].clone(),
            })
            .collect(),
    }
}

/// What an eagerly built command entry holds.
#[allow(dead_code)]
struct Parameter {
    name: String,
    type_name: String,
    sets: Vec<String>,
    aliases: Vec<String>,
}

#[allow(dead_code)]
struct CommandEntry {
    name: String,
    parameters: Vec<Parameter>,
    by_name: HashMap<String, usize>,
}

fn build_names(inventory: &Inventory) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for (index, name) in inventory.commands.iter().enumerate() {
        map.insert(name.to_ascii_lowercase(), index as u32);
    }
    for (index, (alias, _)) in inventory.aliases.iter().enumerate() {
        map.insert(alias.to_ascii_lowercase(), 10_000 + index as u32);
    }
    for (index, (accelerator, _)) in inventory.accelerators.iter().enumerate() {
        map.insert(accelerator.to_ascii_lowercase(), 20_000 + index as u32);
    }
    map
}

fn build_metadata(inventory: &Inventory) -> HashMap<String, CommandEntry> {
    let mut commands: HashMap<String, CommandEntry> = HashMap::new();
    for name in &inventory.commands {
        commands.insert(
            name.to_ascii_lowercase(),
            CommandEntry {
                name: name.clone(),
                parameters: Vec::new(),
                by_name: HashMap::new(),
            },
        );
    }
    for row in &inventory.parameters {
        if let Some(entry) = commands.get_mut(&row.command.to_ascii_lowercase()) {
            entry
                .by_name
                .insert(row.name.to_ascii_lowercase(), entry.parameters.len());
            entry.parameters.push(Parameter {
                name: row.name.clone(),
                type_name: row.type_name.clone(),
                sets: row.sets.split(',').map(str::to_owned).collect(),
                aliases: row
                    .aliases
                    .split(',')
                    .filter(|alias| !alias.is_empty())
                    .map(str::to_owned)
                    .collect(),
            });
        }
    }
    commands
}

fn compare_ignore_ascii_case(a: &str, b: &str) -> Ordering {
    a.bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(b.bytes().map(|byte| byte.to_ascii_lowercase()))
}

fn main() {
    let inventory = load();
    let mut report = Report::new(
        "startup_tables",
        "eager construction of the command table against static lookup (#5)",
    );
    report.header();
    report.fact("commands", inventory.commands.len());
    report.fact("aliases", inventory.aliases.len());
    report.fact("type accelerators", inventory.accelerators.len());
    report.fact("parameters", inventory.parameters.len());

    report.add(measure(
        "construct",
        "eager map of names",
        iterations(2_000),
        || build_names(&inventory),
    ));
    report.add(measure(
        "construct",
        "eager parameter metadata",
        iterations(500),
        || build_metadata(&inventory),
    ));

    // Lookups: every command name and alias once, in mixed case as typed.
    let queries: Vec<String> = inventory
        .commands
        .iter()
        .chain(inventory.aliases.iter().map(|(alias, _)| alias))
        .map(|name| name.to_ascii_uppercase())
        .collect();
    let query_count = queries.len() as u64;

    let names = build_names(&inventory);
    report.add(measure(
        "lookup x422",
        "eager map, lowercased key",
        iterations(5_000),
        || {
            let mut found = 0u32;
            for query in &queries {
                if names.contains_key(&query.to_ascii_lowercase()) {
                    found += 1;
                }
            }
            found
        },
    ));

    let mut sorted: Vec<&str> = inventory
        .commands
        .iter()
        .chain(inventory.aliases.iter().map(|(alias, _)| alias))
        .chain(inventory.accelerators.iter().map(|(name, _)| name))
        .map(String::as_str)
        .collect();
    sorted.sort_by(|a, b| compare_ignore_ascii_case(a, b));
    sorted.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    report.add(measure(
        "lookup x422",
        "static sorted table",
        iterations(5_000),
        || {
            let mut found = 0u32;
            for query in &queries {
                if sorted
                    .binary_search_by(|name| compare_ignore_ascii_case(name, query))
                    .is_ok()
                {
                    found += 1;
                }
            }
            found
        },
    ));
    report.fact("lookups per sample", query_count);
    black_box(&names);
    report.save();
}
