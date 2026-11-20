//! The cmdlets shipped with PowerShell.
//!
//! Upstream: `Microsoft.PowerShell.Core` (inside
//! `System.Management.Automation`), `Microsoft.PowerShell.Commands.Utility`,
//! `Microsoft.PowerShell.Commands.Management`, `Microsoft.PowerShell.Security`
//! and the `Microsoft.PowerShell.Host` module. Nushell analogues:
//! `nu-command`, `nu-cmd-lang`.
//!
//! `inventory/commands.tsv` and `inventory/parameters.tsv` are the checklist:
//! every command pwsh 7.7.0 exposes on Linux, and every parameter of each,
//! with a `status` column this crate's progress is recorded in.
//!
//! One module per upstream module (`core`, `utility`, `management`,
//! `security`, `host`). A cmdlet is registered by name and built on first
//! invocation, so the size of this crate does not show up in startup time
//! (DESIGN.md §4).
//!
//! # Status
//!
//! Empty.

#![forbid(unsafe_code)]
