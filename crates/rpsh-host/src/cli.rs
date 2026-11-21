//! Command-line parsing, following upstream's `CommandLineParameterParser.cs`.

use std::path::PathBuf;

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// `-Version`
    Version,
    /// `-Help`, `-?`
    Help,
    /// `-Command <text>`: the rest of the command line, joined with spaces.
    Command(String),
    /// `-File <path>`, or the first argument that is not a switch.
    File(PathBuf),
    /// No script given: an interactive session.
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// A switch or mode upstream has and rpsh does not handle yet.
    NotImplemented(&'static str),
    /// `-Command` with nothing after it. Upstream exits with 64.
    MissingCommand,
    /// `-File` with nothing after it. Upstream exits with 64.
    MissingFile,
}

/// Every switch upstream accepts, in the order it tests them, as
/// `(name, shortest accepted prefix)`. A switch matches when it is a prefix of
/// `name` at least as long as the second string; the first match wins.
/// Source: `MatchSwitch` calls in `CommandLineParameterParser.cs`, the reference release.
const SWITCHES: &[(&str, &str)] = &[
    ("version", "v"),
    ("help", "h"),
    ("login", "l"),
    ("noexit", "noe"),
    ("noprofile", "nop"),
    ("nologo", "nol"),
    ("noninteractive", "noni"),
    ("socketservermode", "so"),
    ("servermode", "s"),
    ("namedpipeservermode", "nam"),
    ("sshservermode", "sshs"),
    ("noprofileloadtime", "noprofileloadtime"),
    ("interactive", "i"),
    ("configurationfile", "configurationfile"),
    ("configurationname", "config"),
    ("custompipename", "cus"),
    ("commandwithargs", "commandwithargs"),
    ("cwa", "cwa"),
    ("command", "c"),
    ("windowstyle", "w"),
    ("file", "f"),
    ("outputformat", "o"),
    ("of", "o"),
    ("inputformat", "inp"),
    ("if", "if"),
    ("executionpolicy", "ex"),
    ("ep", "ep"),
    ("encodedcommand", "e"),
    ("ec", "e"),
    ("encodedarguments", "encodeda"),
    ("ea", "ea"),
    ("settingsfile", "settings"),
    ("sta", "sta"),
    ("mta", "mta"),
    ("workingdirectory", "wo"),
    ("wd", "wd"),
    (
        "removeworkingdirectorytrailingcharacter",
        "removeworkingdirectorytrailingcharacter",
    ),
    ("token", "to"),
    ("utctimestamp", "utc"),
];

/// The full name of the switch `key` selects, if any. `key` is lower case and
/// has no leading dashes.
fn match_switch(key: &str) -> Option<&'static str> {
    if key == "?" {
        return Some("help");
    }
    SWITCHES
        .iter()
        .find(|(name, shortest)| key.len() >= shortest.len() && name.starts_with(key))
        .map(|(name, _)| *name)
}

/// Parses the arguments that follow the program name.
pub fn parse_args(args: &[String]) -> Result<Invocation, CliError> {
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        index += 1;
        // Upstream accepts `-switch`, `--switch` and `/switch`. Anything it
        // does not recognize as a switch starts the script file name.
        if arg.starts_with("/-") || arg.starts_with("-/") {
            return Err(CliError::NotImplemented("mixed '-' and '/' switch prefix"));
        }
        let switch = arg
            .strip_prefix('-')
            .map(|rest| rest.strip_prefix('-').unwrap_or(rest))
            .or_else(|| arg.strip_prefix('/'))
            .and_then(|key| match_switch(&key.to_ascii_lowercase()));
        match switch {
            Some("version") => return Ok(Invocation::Version),
            Some("help") => return Ok(Invocation::Help),
            // Nothing to skip yet: there are no profiles, no banner and no
            // prompts. The switches are accepted so that scripts and the
            // harness can pass them as they would to pwsh.
            Some("noprofile" | "nologo" | "noninteractive") => {}
            Some("command") => return command(&args[index..]),
            Some("file") => {
                let Some(path) = args.get(index) else {
                    return Err(CliError::MissingFile);
                };
                return file(path, &args[index + 1..]);
            }
            Some(_) => return Err(CliError::NotImplemented("this command-line switch")),
            None => return file(arg, &args[index..]),
        }
    }
    Ok(Invocation::Interactive)
}

fn command(rest: &[String]) -> Result<Invocation, CliError> {
    match rest {
        [] => Err(CliError::MissingCommand),
        [only] if only == "-" => Err(CliError::NotImplemented("-Command - (script from stdin)")),
        [first, ..] if first == "-" => Err(CliError::NotImplemented(
            "-Command - with trailing arguments",
        )),
        _ => Ok(Invocation::Command(rest.join(" "))),
    }
}

fn file(path: &str, script_args: &[String]) -> Result<Invocation, CliError> {
    if path == "-" {
        return Err(CliError::NotImplemented("-File - (script from stdin)"));
    }
    if !script_args.is_empty() {
        return Err(CliError::NotImplemented("script arguments"));
    }
    Ok(Invocation::File(PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Invocation, CliError> {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        parse_args(&args)
    }

    #[test]
    fn switches_match_by_prefix_and_ignore_case() {
        for spelling in ["-Command", "-c", "-com", "--command", "-COMMAND"] {
            assert_eq!(
                parse(&[spelling, "1"]),
                Ok(Invocation::Command("1".to_owned())),
                "{spelling}"
            );
        }
        assert_eq!(parse(&["-v"]), Ok(Invocation::Version));
        assert_eq!(parse(&["-?"]), Ok(Invocation::Help));
    }

    #[test]
    fn a_prefix_shorter_than_the_shortest_accepted_one_is_a_file_name() {
        // `-no` is ambiguous between -NoProfile, -NoLogo and friends, so
        // upstream does not treat it as a switch at all.
        assert_eq!(parse(&["-no"]), Ok(Invocation::File(PathBuf::from("-no"))));
    }

    #[test]
    fn command_takes_the_rest_of_the_line() {
        assert_eq!(
            parse(&["-NoProfile", "-NonInteractive", "-Command", "1", "+", "1"]),
            Ok(Invocation::Command("1 + 1".to_owned()))
        );
        assert_eq!(parse(&["-c"]), Err(CliError::MissingCommand));
    }

    #[test]
    fn the_first_non_switch_argument_is_the_script() {
        let script = Ok(Invocation::File(PathBuf::from("a.ps1")));
        assert_eq!(parse(&["a.ps1"]), script);
        assert_eq!(parse(&["-nop", "-File", "a.ps1"]), script);
        assert_eq!(parse(&["-f"]), Err(CliError::MissingFile));
    }

    #[test]
    fn no_script_means_interactive() {
        assert_eq!(parse(&[]), Ok(Invocation::Interactive));
        assert_eq!(parse(&["-NoLogo"]), Ok(Invocation::Interactive));
    }

    #[test]
    fn upstream_switches_without_an_implementation_are_refused() {
        for args in [
            &["-NoExit"][..],
            &["-EncodedCommand", "MQ=="],
            &["-wd", "/tmp"],
            &["-Login"],
            &["a.ps1", "arg"],
            &["-c", "-"],
        ] {
            assert!(
                matches!(parse(args), Err(CliError::NotImplemented(_))),
                "{args:?}"
            );
        }
    }
}
