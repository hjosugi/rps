//! Formatting and output.
//!
//! Upstream: `src/System.Management.Automation/FormatAndOutput` — the format
//! view definitions for the built-in types, the `Format-Table`, `Format-List`,
//! `Format-Wide` and `Format-Custom` engines, and the `Out-*` sinks. Nushell
//! analogue: `nu-table`.
//!
//! Text output is where most differential tests observe rpsh, so the default
//! views have to match byte for byte: column widths, truncation, the rule
//! that picks a table for up to four properties and a list above that,
//! `$FormatEnumerationLimit`, and `$PSStyle` rendering.
//! `inventory/format-views.tsv` lists the views pwsh ships.
//!
//! Format data is a prime candidate for lazy construction (DESIGN.md §4): a
//! script that prints two integers must not build a single view.
//!
//! # Status
//!
//! Empty. The host writes values with `Display`.

#![forbid(unsafe_code)]
