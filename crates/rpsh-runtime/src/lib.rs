//! PowerShell values, conversions and operators.
//!
//! Upstream: `LanguagePrimitives.cs`, `MshObject.cs` and `runtime/Operations`
//! under `src/System.Management.Automation/engine`. Nushell analogue:
//! `nu-protocol`.
//!
//! # Status
//!
//! Two value types (`Int32`, `String`) and the operators the walking skeleton
//! needs. An operation whose upstream result this crate cannot reproduce
//! returns [`RuntimeError::NotImplemented`]; it never returns an approximate
//! value. `Int32` overflow, for instance, promotes to `Double` upstream, so
//! here it is refused until `Double` exists.

pub mod ops;

use std::fmt;

/// A PowerShell value.
///
/// The representation is decided in DESIGN.md §3.3: `String` in particular
/// has to answer `.Length` and indexing in UTF-16 code units, which this
/// placeholder does not attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Value {
    /// `System.Int32`
    Int32(i32),
    /// `System.String`
    String(String),
}

impl Value {
    /// The .NET type name scripts see through `.GetType().FullName`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int32(_) => "System.Int32",
            Value::String(_) => "System.String",
        }
    }
}

/// How the value appears when it reaches the end of the pipeline and the
/// host writes it as text.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int32(value) => write!(f, "{value}"),
            Value::String(value) => f.write_str(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// The operation is valid PowerShell whose result rpsh cannot produce yet.
    NotImplemented { feature: &'static str },
}

impl RuntimeError {
    /// The label of the missing feature.
    pub fn feature(&self) -> &'static str {
        match self {
            Self::NotImplemented { feature } => feature,
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.feature())
    }
}

impl std::error::Error for RuntimeError {}
