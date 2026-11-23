//! Console host: the command line, script files and interactive sessions.
//!
//! Upstream: `src/Microsoft.PowerShell.ConsoleHost`. Nushell analogue:
//! `nu-cli`.
//!
//! # Status
//!
//! `-Command` and `-File` run through the engine. There is no interactive
//! session, no profile loading and no formatting beyond `Display`.

mod cli;

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use rpsh_engine::{Engine, EngineError};
use rpsh_messages as messages;

pub use cli::{CliError, Invocation, parse_args};

/// How rpsh chooses the upstream release it is measured against.
///
/// `scripts/fetch-pwsh.sh` resolves the latest stable release; every result
/// file and golden records the version it actually ran with.
pub const REFERENCE_POLICY: &str = "latest stable";

/// Exit code when rpsh stops because it does not implement something.
///
/// A script can exit with this code on its own, so the harness also requires
/// stderr to start with [`NOT_IMPLEMENTED_MARKER`] before it counts a run as
/// "not implemented" rather than as a wrong answer.
pub const EXIT_NOT_IMPLEMENTED: i32 = 99;

/// What stderr starts with when rpsh exits with [`EXIT_NOT_IMPLEMENTED`].
/// The missing feature's label follows.
pub const NOT_IMPLEMENTED_MARKER: &str = "rpsh: not implemented: ";

/// Upstream's `ExitCodeBadCommandLineParameter`.
const EXIT_BAD_COMMAND_LINE: i32 = 64;

/// Upstream `CommandLineParameterParserStrings` messages, by catalog key.
const MISSING_COMMAND_PARAMETER: &str = "CommandLineParameterParserStrings/MissingCommandParameter";
const MISSING_FILE_ARGUMENT: &str = "CommandLineParameterParserStrings/MissingFileArgument";
const ARGUMENT_FILE_DOES_NOT_EXIST: &str =
    "CommandLineParameterParserStrings/ArgumentFileDoesNotExist";

/// Runs rpsh with the arguments that follow the program name and returns the
/// process exit code.
pub fn run(args: &[String]) -> i32 {
    let invocation = match parse_args(args) {
        Ok(invocation) => invocation,
        Err(CliError::NotImplemented(feature)) => return not_implemented(feature),
        Err(CliError::MissingCommand) => {
            eprintln!("{}", messages::text(MISSING_COMMAND_PARAMETER));
            return EXIT_BAD_COMMAND_LINE;
        }
        Err(CliError::MissingFile) => {
            eprintln!("{}", messages::text(MISSING_FILE_ARGUMENT));
            return EXIT_BAD_COMMAND_LINE;
        }
    };
    match invocation {
        Invocation::Version => {
            println!(
                "rpsh {} (targets {REFERENCE_POLICY} PowerShell)",
                env!("CARGO_PKG_VERSION")
            );
            0
        }
        Invocation::Help => {
            println!("{USAGE}");
            0
        }
        Invocation::Command(source) => run_source(&source),
        Invocation::File { path, args } => run_file(&path, &args),
        Invocation::Interactive => not_implemented("interactive sessions"),
    }
}

const USAGE: &str = "\
Usage: rpsh [-NoProfile] [-NoLogo] [-NonInteractive] [[-File] <filePath>]
            [-Command <string>] [-Version] [-Help]

Switches are case-insensitive and may be abbreviated the way pwsh allows.
Anything rpsh does not implement yet ends the run with exit code 99.";

fn not_implemented(feature: &str) -> i32 {
    eprintln!("{NOT_IMPLEMENTED_MARKER}{feature}");
    EXIT_NOT_IMPLEMENTED
}

fn run_file(path: &Path, args: &[String]) -> i32 {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => {
            let path = path.display().to_string();
            eprintln!(
                "{}",
                messages::format(ARGUMENT_FILE_DOES_NOT_EXIST, &[&path])
            );
            return EXIT_BAD_COMMAND_LINE;
        }
    };
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    match std::str::from_utf8(bytes) {
        Ok(source) => run_source_with(source, args, Some(path)),
        Err(_) => not_implemented("script files that are not UTF-8"),
    }
}

fn run_source(source: &str) -> i32 {
    run_source_with(source, &[], None)
}

fn run_source_with(source: &str, args: &[String], path: Option<&Path>) -> i32 {
    let mut out = BufWriter::new(io::stdout().lock());
    let result = Engine::new().run_script_with(source, args, path, &mut out);
    let flushed = out.flush();
    match (result, flushed) {
        (Ok(code), Ok(())) => code,
        (Err(EngineError::Io(error)), _) | (Ok(_), Err(error)) => {
            eprintln!("rpsh: cannot write output: {error}");
            1
        }
        (Err(error), _) => {
            eprintln!("{NOT_IMPLEMENTED_MARKER}{error}");
            EXIT_NOT_IMPLEMENTED
        }
    }
}
