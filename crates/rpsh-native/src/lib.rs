//! Native command execution: processes, pipes, the terminal and signals.
//!
//! Upstream: `NativeCommandProcessor.cs`, `NativeCommandParameterBinder.cs`
//! and `NativeCommand.cs` in `src/System.Management.Automation/engine`.
//! Nushell analogue: `nu-system` (foreground process groups, job control) and
//! the `ByteStream` half of `nu-protocol`'s `PipelineData`.
//!
//! "Linux commands just work" (DESIGN.md §6) is decided here:
//!
//! * argument passing (`$PSNativeCommandArgumentPassing`), `--%`, and the
//!   glob and `~` expansion pwsh performs for native commands on Unix;
//! * byte-exact pipes between native commands, with decoding to strings only
//!   where a cmdlet is the reader;
//! * handing the terminal to interactive programs (`vim`, `less`, `ssh`,
//!   `sudo`) and taking it back, including `SIGINT`, `SIGTSTP` and
//!   `SIGWINCH`;
//! * `$LASTEXITCODE`, `$?`, and `$PSNativeCommandUseErrorActionPreference`.
//!
//! This crate will need `unsafe` for `libc`; the workspace denies it
//! elsewhere.
//!
//! # Status
//!
//! Connects consecutive native commands with OS pipes. The engine encodes
//! and decodes values only at the ends of a native pipeline.

mod application;
mod pipe;

pub use application::application_command;
pub use pipe::{Destination, PipelineError, PipelineStatus, run_pipeline, run_pipeline_with_input};
