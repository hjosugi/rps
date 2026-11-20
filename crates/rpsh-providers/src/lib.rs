//! PowerShell providers and path resolution.
//!
//! Upstream: `src/System.Management.Automation/namespaces` (`ProviderBase.cs`
//! up to `NavigationProviderBase.cs`, `FileSystemProvider.cs`,
//! `EnvironmentProvider.cs`, `VariableProvider.cs`, `FunctionProvider.cs`,
//! `AliasProvider.cs`, `LocationGlobber.cs`) and the `SessionState*` path and
//! drive APIs in `engine`. Nushell analogues: `nu-path`, `nu-glob`.
//!
//! The item cmdlets (`Get-ChildItem`, `Get-Content`, `Copy-Item`, ...) are
//! thin layers over a provider, so path semantics live here: drives,
//! provider-qualified paths, wildcard expansion, `-LiteralPath`, and the
//! `PSPath` / `PSParentPath` / `PSChildName` properties attached to every
//! item. `inventory/providers.tsv` lists what pwsh ships on Linux.
//!
//! # Status
//!
//! Empty.

#![forbid(unsafe_code)]
