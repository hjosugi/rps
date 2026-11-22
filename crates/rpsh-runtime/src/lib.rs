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
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// `System.Int32`
    Int32(i32),
    /// `System.Int64`
    Int64(i64),
    /// `System.UInt32`
    UInt32(u32),
    /// `System.UInt64`
    UInt64(u64),
    /// `System.Int16`
    Int16(i16),
    /// `System.UInt16`
    UInt16(u16),
    /// `System.Byte`
    Byte(u8),
    /// `System.SByte`
    SByte(i8),
    /// `System.Decimal`, with its scale preserved as text.
    Decimal(String),
    /// `System.Double`
    Double(f64),
    /// `System.Numerics.BigInteger`
    BigInteger(String),
    /// A PowerShell object/hashtable with ordered keys.
    Object(Vec<(String, Value)>),
    /// The result of `.GetType()`, holding the full .NET type name.
    Type(String),
    /// `System.String`
    String(String),
}

impl Value {
    /// The .NET type name scripts see through `.GetType().FullName`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int32(_) => "System.Int32",
            Value::Int64(_) => "System.Int64",
            Value::UInt32(_) => "System.UInt32",
            Value::UInt64(_) => "System.UInt64",
            Value::Int16(_) => "System.Int16",
            Value::UInt16(_) => "System.UInt16",
            Value::Byte(_) => "System.Byte",
            Value::SByte(_) => "System.SByte",
            Value::Decimal(_) => "System.Decimal",
            Value::Double(_) => "System.Double",
            Value::BigInteger(_) => "System.Numerics.BigInteger",
            Value::Object(_) => "System.Management.Automation.PSCustomObject",
            Value::Type(_) => "System.RuntimeType",
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
            Value::Int64(value) => write!(f, "{value}"),
            Value::UInt32(value) => write!(f, "{value}"),
            Value::UInt64(value) => write!(f, "{value}"),
            Value::Int16(value) => write!(f, "{value}"),
            Value::UInt16(value) => write!(f, "{value}"),
            Value::Byte(value) => write!(f, "{value}"),
            Value::SByte(value) => write!(f, "{value}"),
            Value::Decimal(value) | Value::BigInteger(value) | Value::Type(value) => {
                f.write_str(value)
            }
            Value::Double(value) => write!(f, "{value}"),
            Value::Object(entries) => {
                f.write_str("@{")?;
                for (index, (name, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{name}={value}")?;
                }
                f.write_str("}")
            }
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
