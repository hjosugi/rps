//! The cmdlets shipped with PowerShell.
//!
//! Upstream: `Microsoft.PowerShell.Core` (inside
//! `System.Management.Automation`), `Microsoft.PowerShell.Commands.Utility`,
//! `Microsoft.PowerShell.Commands.Management`, `Microsoft.PowerShell.Security`
//! and the `Microsoft.PowerShell.Host` module. Nushell analogues:
//! `nu-command`, `nu-cmd-lang`.
//!
//! `inventory/commands.tsv` and `inventory/parameters.tsv` are the checklist:
//! every command the reference pwsh exposes on Linux, and every parameter of each,
//! with a `status` column this crate's progress is recorded in.
//!
//! One module per upstream module (`core`, `utility`, `management`,
//! `security`, `host`). A cmdlet is registered by name and built on first
//! invocation, so the size of this crate does not show up in startup time
//! (DESIGN.md §4).
//!
//! # Status
//!
//! The Management and Utility export catalogs are compiled from the pinned
//! inventory. A name lookup can run before either module is loaded.

#![forbid(unsafe_code)]

pub mod framework;
pub mod signature;

/// Exports of a built-in PowerShell module, available before it is loaded.
#[derive(Debug)]
pub struct BuiltinModule {
    pub name: &'static str,
    pub version: &'static str,
    pub aliases: &'static [(&'static str, &'static str)],
    cmdlets: &'static [&'static str],
}

impl BuiltinModule {
    /// The dispatcher has already normalized the name for case-insensitive lookup.
    pub fn exports_cmdlet(&self, lowercase: &str) -> bool {
        self.cmdlets.binary_search(&lowercase).is_ok()
    }

    pub fn exports_alias(&self, name: &str) -> bool {
        self.aliases
            .iter()
            .any(|(alias, _)| alias.eq_ignore_ascii_case(name))
    }
}

include!(concat!(env!("OUT_DIR"), "/modules.rs"));
