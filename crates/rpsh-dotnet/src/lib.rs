//! The .NET surface PowerShell scripts see.
//!
//! PowerShell has no object model of its own: every value is a .NET object,
//! and scripts reach straight into the base class library with `[type]`
//! literals, `::` static members, `New-Object`, `Add-Type` and binary
//! modules. A port has to answer those without paying for a .NET runtime on
//! every start. DESIGN.md §5 splits the answer into tiers, and this crate is
//! where they meet:
//!
//! * **Tier 0 — native shim.** The types scripts use most (`String`, `Math`,
//!   `DateTime`, `IO.File`, `IO.Path`, `Text.StringBuilder`, the collections,
//!   `Regex`, ...) implemented in Rust, with upstream's member names,
//!   overloads and conversion rules. No runtime is loaded.
//! * **Tier 1 — lazy in-process CLR.** The first reference to a type outside
//!   the shim, an `Add-Type`, or a binary module loads CoreCLR through
//!   `hostfxr`, once, at that moment. A script that never asks pays nothing.
//! * **Tier 2 — warm sidecar.** A per-user process that keeps a runtime and
//!   the commonly used assemblies loaded across rpsh invocations.
//!
//! Which tier a script needs is decidable from its AST before it runs, which
//! is also what lets the harness report results per tier.
//!
//! # Status
//!
//! Empty. Nothing in the walking skeleton touches a .NET type.

#![forbid(unsafe_code)]
