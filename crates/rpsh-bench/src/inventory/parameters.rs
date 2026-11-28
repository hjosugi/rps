//! Compare the running shell's binder declarations with the separately generated pwsh TSVs.
use rpsh_commands::signature::{SIGNATURES, Signature};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Default)]
pub(super) struct Options {
    pub details: bool,
    pub check: bool,
    pub registered_only: bool,
    pub command: Option<String>,
    pub platform: Option<String>,
}

#[derive(Debug)]
struct ReferenceCommand<'a> {
    name: &'a str,
    module: &'a str,
    params: BTreeMap<String, ReferenceParameter<'a>>,
}

#[derive(Debug)]
struct ReferenceParameter<'a> {
    name: &'a str,
    ty: &'a str,
    sets: &'a str,
    aliases: &'a str,
}

type Commands<'a> = BTreeMap<String, ReferenceCommand<'a>>;
type Platforms<'a> = BTreeMap<&'a str, Commands<'a>>;

fn table<'a>(
    text: &'a str,
    file: &str,
    header: &[&str],
    mut row: impl FnMut(&[&'a str]) -> Result<(), String>,
) -> Result<(), String> {
    let mut lines = text.lines();
    if lines.next() != Some(header.join("\t").as_str()) {
        return Err(format!("{file}: expected header {}", header.join("\t")));
    }
    for (index, line) in lines.enumerate() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != header.len() {
            return Err(format!(
                "{file}:{}: expected {} columns",
                index + 2,
                header.len()
            ));
        }
        row(&fields).map_err(|e| format!("{file}:{}: {e}", index + 2))?;
    }
    Ok(())
}

fn reference<'a>(commands: &'a str, parameters: &'a str) -> Result<Platforms<'a>, String> {
    let mut platforms = Platforms::new();
    table(
        commands,
        "commands.tsv",
        &["module", "name", "kind", "platform", "status"],
        |fields| {
            let [module, name, kind, platform, _] = *fields else {
                unreachable!()
            };
            if kind == "Function" {
                return Ok(());
            }
            if kind != "Cmdlet" || module.is_empty() || name.is_empty() || platform.is_empty() {
                return Err("invalid command identity".into());
            }
            let command = ReferenceCommand {
                name,
                module,
                params: BTreeMap::new(),
            };
            if platforms
                .entry(platform)
                .or_default()
                .insert(name.to_ascii_lowercase(), command)
                .is_some()
            {
                return Err(format!("duplicate {platform}/{name}"));
            }
            Ok(())
        },
    )?;
    table(
        parameters,
        "parameters.tsv",
        &[
            "command",
            "parameter",
            "type",
            "parameter_sets",
            "aliases",
            "platform",
            "status",
        ],
        |fields| {
            let [command, name, ty, sets, aliases, platform, _] = *fields else {
                unreachable!()
            };
            let Some(entry) = platforms
                .get_mut(platform)
                .and_then(|p| p.get_mut(&command.to_ascii_lowercase()))
            else {
                return Err(format!("unknown cmdlet {platform}/{command}"));
            };
            if name.is_empty() || ty.is_empty() || sets.is_empty() {
                return Err(format!("incomplete parameter for {command}"));
            }
            if entry
                .params
                .insert(
                    name.to_ascii_lowercase(),
                    ReferenceParameter {
                        name,
                        ty,
                        sets,
                        aliases,
                    },
                )
                .is_some()
            {
                return Err(format!("duplicate {platform}/{command} -{name}"));
            }
            Ok(())
        },
    )?;
    if platforms.is_empty() {
        return Err("commands.tsv contains no cmdlets".into());
    }
    Ok(platforms)
}

#[derive(Default, Debug)]
struct Audit {
    expected: usize,
    matched: usize,
    differences: Vec<String>,
}

fn list(text: &str) -> BTreeSet<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn compare(expected: Option<&ReferenceCommand<'_>>, declared: Option<&Signature>) -> Audit {
    let mut result = Audit::default();
    if let Some(command) = expected {
        result.expected = command.params.len();
        match declared {
            None => result.differences.push("missing cmdlet declaration".into()),
            Some(s) if !s.module.eq_ignore_ascii_case(command.module) => result.differences.push(
                format!("module: expected {}, declared {}", command.module, s.module),
            ),
            _ => (),
        }
        for p in command.params.values() {
            let Some(actual) = declared.and_then(|s| {
                s.parameters
                    .iter()
                    .find(|a| a.name.eq_ignore_ascii_case(p.name))
            }) else {
                result.differences.push(format!("-{}: missing", p.name));
                continue;
            };
            let before = result.differences.len();
            let sets = if actual.sets.is_empty() {
                "*".to_owned()
            } else {
                actual.sets.join(",")
            };
            let aliases = actual.aliases.join(",");
            for (label, expected, actual, same) in [
                (
                    "type",
                    p.ty,
                    actual.ty.full_name(),
                    p.ty.eq_ignore_ascii_case(actual.ty.full_name()),
                ),
                ("sets", p.sets, sets.as_str(), list(p.sets) == list(&sets)),
                (
                    "aliases",
                    p.aliases,
                    aliases.as_str(),
                    list(p.aliases) == list(&aliases),
                ),
            ] {
                if !same {
                    result.differences.push(format!(
                        "-{} {label}: expected {expected:?}, declared {actual:?}",
                        p.name
                    ));
                }
            }
            if result.differences.len() == before {
                result.matched += 1;
            }
        }
    } else {
        result
            .differences
            .push("unexpected cmdlet declaration".into());
    }
    if let Some(signature) = declared {
        let mut names = BTreeSet::new();
        for actual in signature.parameters {
            let key = actual.name.to_ascii_lowercase();
            if !names.insert(key.clone()) {
                result
                    .differences
                    .push(format!("-{}: duplicate declaration", actual.name));
            } else if expected.is_none_or(|command| !command.params.contains_key(&key)) {
                result
                    .differences
                    .push(format!("-{}: unexpected", actual.name));
            }
        }
    }
    result
}

pub(super) fn run(directory: &Path, options: &Options) -> Result<bool, String> {
    let read =
        |file: &str| fs::read_to_string(directory.join(file)).map_err(|e| format!("{file}: {e}"));
    let commands = read("commands.tsv")?;
    let parameters = read("parameters.tsv")?;
    let platforms = reference(&commands, &parameters)?;
    let mut selected = false;
    let mut success = true;
    if let Some(platform) = &options.platform
        && !platforms.contains_key(platform.as_str())
    {
        return Err(format!("unknown inventory platform {platform:?}"));
    }
    for (platform, reference) in platforms {
        if options.platform.as_deref().is_some_and(|p| p != platform) {
            continue;
        }
        let mut declared = BTreeMap::new();
        for &signature in SIGNATURES.iter().filter(|s| s.available_on(platform)) {
            if declared
                .insert(signature.name.to_ascii_lowercase(), signature)
                .is_some()
            {
                return Err(format!("duplicate declaration for {}", signature.name));
            }
        }
        let names: BTreeSet<_> = reference.keys().chain(declared.keys()).collect();
        let (mut checked, mut complete, mut matched, mut expected) = (0, 0, 0, 0);
        for name in names {
            let actual = declared.get(name).copied();
            if options
                .command
                .as_ref()
                .is_some_and(|q| !q.eq_ignore_ascii_case(name))
                || (options.registered_only && actual.is_none())
            {
                continue;
            }
            selected = true;
            let command = reference.get(name);
            let result = compare(command, actual);
            let ok = result.differences.is_empty();
            success &= ok;
            checked += 1;
            complete += usize::from(ok);
            matched += result.matched;
            expected += result.expected;
            if options.details || options.check || options.command.is_some() {
                let display = command
                    .map(|c| c.name)
                    .or_else(|| actual.map(|s| s.name))
                    .unwrap_or(name);
                println!(
                    "{platform} {display}: {}/{} parameter declarations match ({})",
                    result.matched,
                    result.expected,
                    if ok { "ok" } else { "incomplete" }
                );
                for difference in result.differences {
                    println!("  {difference}");
                }
            }
        }
        println!(
            "{platform}: {matched}/{expected} parameter declarations match; {complete}/{checked} cmdlet signatures complete"
        );
    }
    if !selected {
        return Err("no matching cmdlet in the selected parameter inventory".into());
    }
    Ok(!options.check || success)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpsh_commands::signature::SWITCH_PROCESS;
    const COMMANDS: &str = "module\tname\tkind\tplatform\tstatus\nMicrosoft.PowerShell.Core\tSwitch-Process\tCmdlet\tlinux-x64\tdone\nM\tZero\tCmdlet\tlinux-x64\tdone";
    const HEADER: &str = "command\tparameter\ttype\tparameter_sets\taliases\tplatform\tstatus\n";

    #[test]
    fn mismatches_include_fields_missing_extra_and_empty_commands() {
        let rows = format!(
            "{HEADER}Switch-Process\tWithCommand\tSystem.String\tOther\tCmd\tlinux-x64\tdone\nSwitch-Process\tMissing\tSystem.String\t*\t\tlinux-x64\tdone"
        );
        let input = reference(COMMANDS, &rows).unwrap();
        let commands = &input["linux-x64"];
        let failure = compare(Some(&commands["switch-process"]), Some(&SWITCH_PROCESS));
        assert_eq!((failure.matched, failure.expected), (0, 2));
        assert_eq!(failure.differences.len(), 4);
        for label in ["type:", "sets:", "aliases:", "-Missing: missing"] {
            assert!(failure.differences.iter().any(|d| d.contains(label)));
        }
        assert_eq!(
            compare(Some(&commands["zero"]), None).differences,
            ["missing cmdlet declaration"]
        );
        let extra = compare(None, Some(&SWITCH_PROCESS));
        assert!(
            extra
                .differences
                .iter()
                .any(|d| d.contains("-WithCommand: unexpected"))
        );
    }

    #[test]
    fn status_is_independent_of_metadata_and_duplicate_rows_are_rejected() {
        let row = "switch-process\twithcommand\tSystem.String[]\t*\t\tlinux-x64\ttodo\n";
        let text = format!("{HEADER}{row}");
        let input = reference(COMMANDS, &text).unwrap();
        let audit = compare(
            Some(&input["linux-x64"]["switch-process"]),
            Some(&SWITCH_PROCESS),
        );
        assert_eq!((audit.matched, audit.expected), (1, 1));
        assert!(audit.differences.is_empty());
        assert!(
            reference(COMMANDS, &format!("{text}{row}"))
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(
            reference(COMMANDS, &text.replace("switch-process", "typo"))
                .unwrap_err()
                .contains("unknown cmdlet")
        );
        assert_eq!(list("First,Second"), list("second,FIRST"));
    }
}
