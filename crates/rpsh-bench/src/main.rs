//! Measurement harness: how fast rpsh is next to upstream pwsh, and whether
//! it gives the same answers. The rules it enforces are in DESIGN.md §7.

mod bench;
mod cases;
mod conform;
mod engines;
mod inventory;
mod json;
mod profiles;
mod pty;
mod rusage;
mod shells;
mod stats;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo harness <command>

  bench [--suite scripts|shells] [--runs N] [--warmup N] [--filter TEXT]
        [--out FILE] [--compare BASELINE.json]
        [--export-hyperfine FILE] [--pwsh PATH] [--rpsh PATH]
        [--budget-scale N] [--bash]
      suite=scripts times every script in bench/cases under pwsh and rpsh.
      A speedup is reported only for cases where rpsh's stdout and exit code
      equal pwsh's. Peak RSS and CPU time come from wait4
      (scripts/spawn-measure.c). --compare prints a diff against an earlier
      result file, and --export-hyperfine writes hyperfine-compatible JSON.
      --bash also times matching .bash fixtures after checking their output.
      suite=shells compares bash, dash, zsh, fish, nu, pwsh and rpsh: a
      one-line run and time to the first prompt on a pseudo-terminal.
      suite=profiles times pwsh with the fixture profiles in bench/profiles:
      an empty profile, 100 functions, a module import, tool init | iex and
      alias/completer registration.

  conform record [--filter TEXT] [--pwsh PATH]
      Run tests/conformance/cases under the reference pwsh and store what it
      printed next to each case as <name>.golden.

  conform check [--bless] [--filter TEXT] [--rpsh PATH]
      Run the cases under rpsh and compare with the goldens. Needs no pwsh.
      --bless rewrites tests/conformance/passing.txt to the cases that pass.

  conform pty record|check [--filter TEXT] [--repeat N]
        [--pwsh PATH] [--rpsh PATH]
      Run the manifests in tests/conformance/pty on a pseudo-terminal.
      record uses pwsh, check uses rpsh; each compares the final screen, exit
      code and timeout. This is separate from scripts/check.sh until rpsh has
      an interactive host.

  inventory [--parameters | --check-parameters] [--command NAME] [--platform PLATFORM] [--registered-only]
      Summarize inventories and compare the binder declarations against pwsh.
      --parameters prints each cmdlet and its differences; --check-parameters also fails on differences.
      --registered-only limits comparison to cmdlets with executable parameter declarations.

The reference pwsh is found through --pwsh, $RPSH_PWSH, .cache/pwsh/ (filled by
scripts/fetch-pwsh.sh), then PATH.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = workspace_root();
    let result = match args.split_first() {
        Some((command, rest)) if command == "bench" => bench::run(&root, rest),
        Some((command, rest)) if command == "conform" => conform::run(&root, rest),
        Some((command, rest)) if command == "inventory" => inventory::run(&root, rest),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            eprintln!("rpsh-bench: {message}");
            ExitCode::from(2)
        }
    }
}

/// The harness is a development tool that lives in the checkout it measures.
fn workspace_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

/// The arguments of one subcommand, consumed option by option.
pub(crate) struct Args(Vec<String>);

impl Args {
    pub(crate) fn new(args: &[String]) -> Self {
        Self(args.to_vec())
    }

    /// Takes the leading argument if it is not an option.
    pub(crate) fn positional(&mut self) -> Option<String> {
        let is_positional = self.0.first().is_some_and(|arg| !arg.starts_with('-'));
        is_positional.then(|| self.0.remove(0))
    }

    pub(crate) fn flag(&mut self, name: &str) -> bool {
        match self.0.iter().position(|arg| arg == name) {
            Some(index) => {
                self.0.remove(index);
                true
            }
            None => false,
        }
    }

    pub(crate) fn value(&mut self, name: &str) -> Result<Option<String>, String> {
        let Some(index) = self.0.iter().position(|arg| arg == name) else {
            return Ok(None);
        };
        if index + 1 >= self.0.len() {
            return Err(format!("{name} needs a value"));
        }
        self.0.remove(index);
        Ok(Some(self.0.remove(index)))
    }

    pub(crate) fn number(&mut self, name: &str) -> Result<Option<usize>, String> {
        match self.value(name)? {
            None => Ok(None),
            Some(text) => match text.parse() {
                Ok(number) => Ok(Some(number)),
                Err(_) => Err(format!("{name} needs a whole number, got '{text}'")),
            },
        }
    }

    /// Fails if anything was left unconsumed.
    pub(crate) fn finish(self) -> Result<(), String> {
        match self.0.first() {
            None => Ok(()),
            Some(extra) => Err(format!("unexpected argument '{extra}'\n\n{USAGE}")),
        }
    }
}
