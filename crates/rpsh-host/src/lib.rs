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

pub use cli::{CliError, Invocation, parse_args};

/// The upstream release rpsh is built and measured against.
pub const TARGET_PWSH_VERSION: &str = "7.7.0";

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

/// Runs rpsh with the arguments that follow the program name and returns the
/// process exit code.
pub fn run(args: &[String]) -> i32 {
    let invocation = match parse_args(args) {
        Ok(invocation) => invocation,
        Err(CliError::NotImplemented(feature)) => return not_implemented(feature),
        Err(CliError::MissingCommand) => {
            eprintln!(
                "Cannot process the command because of a missing parameter. \
                 A command must follow -Command."
            );
            return EXIT_BAD_COMMAND_LINE;
        }
        Err(CliError::MissingFile) => {
            eprintln!(
                "The command cannot be run because the File parameter requires a file \
                 path. Supply a path for the File parameter and then try the command again."
            );
            return EXIT_BAD_COMMAND_LINE;
        }
    };
    match invocation {
        Invocation::Version => {
            println!(
                "rpsh {} (targets PowerShell {TARGET_PWSH_VERSION})",
                env!("CARGO_PKG_VERSION")
            );
            0
        }
        Invocation::Help => {
            println!("{USAGE}");
            0
        }
        Invocation::Command(source) => run_source(&source),
        Invocation::File(path) => run_file(&path),
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

fn run_file(path: &Path) -> i32 {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => {
            eprintln!(
                "The argument '{}' is not recognized as the name of a script file. \
                 Check the spelling of the name, or if a path was included, verify \
                 that the path is correct and try again.",
                path.display()
            );
            return EXIT_BAD_COMMAND_LINE;
        }
    };
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    match std::str::from_utf8(bytes) {
        Ok(source) => run_source(source),
        Err(_) => not_implemented("script files that are not UTF-8"),
    }
}

fn run_source(source: &str) -> i32 {
    let mut out = BufWriter::new(io::stdout().lock());
    let result = Engine::new().run_script(source, &mut out);
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
