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
pub mod regex;
mod string;

use std::fmt;
use std::rc::Rc;

pub use string::PsString;

/// A PowerShell value.
///
/// The representation is decided in DESIGN.md §3.3: `String` in particular
/// has to answer `.Length` and indexing in UTF-16 code units, which this
/// placeholder does not attempt.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// `$null`
    Null,
    /// The value of an empty pipeline, `& {}`. Distinct from [`Value::Null`]:
    /// it does not flow to output, so `@(& {}).Count` is 0 while
    /// `@($null).Count` is 1.
    AutomationNull,
    /// `System.Boolean`
    Bool(bool),
    /// `System.Char`, kept as a UTF-16 code unit so a lone surrogate is exact.
    Char(u16),
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
    Decimal(PsString),
    /// `System.Double`
    Double(f64),
    /// `System.Numerics.BigInteger`
    BigInteger(PsString),
    /// A script block literal.
    ScriptBlock(PsString),
    /// A PowerShell array. The `Rc` keeps `Value` 16 bytes.
    Array(Rc<Vec<Value>>),
    /// A PowerShell object/hashtable with ordered keys. The `Rc` is the
    /// object's identity, so `$o -eq $o` compares the same object, not its
    /// contents.
    Object(Rc<Vec<(String, Value)>>),
    /// The result of `.GetType()`, holding the full .NET type name.
    Type(PsString),
    /// `System.String`
    String(PsString),
}

// DESIGN.md §3.3: a tag and an 8-byte payload.
const _: () = assert!(std::mem::size_of::<Value>() == 16);

impl Value {
    /// The .NET type name scripts see through `.GetType().FullName`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null | Value::AutomationNull => "System.Object",
            Value::Bool(_) => "System.Boolean",
            Value::Char(_) => "System.Char",
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
            Value::ScriptBlock(_) => "System.Management.Automation.ScriptBlock",
            Value::Array(_) => "System.Object[]",
            Value::Object(_) => "System.Management.Automation.PSCustomObject",
            Value::Type(_) => "System.RuntimeType",
            Value::String(_) => "System.String",
        }
    }
}

impl Value {
    /// Builds a PowerShell array.
    pub fn array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(items))
    }

    /// Builds an ordered object from its properties.
    pub fn object(properties: Vec<(String, Value)>) -> Value {
        Value::Object(Rc::new(properties))
    }

    /// `$null` and the empty-pipeline value both compare as null.
    pub fn is_null_like(&self) -> bool {
        matches!(self, Value::Null | Value::AutomationNull)
    }
}

/// How the value appears when it reaches the end of the pipeline and the
/// host writes it as text.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null | Value::AutomationNull => Ok(()),
            Value::Bool(value) => write!(f, "{}", if *value { "True" } else { "False" }),
            Value::Char(unit) => {
                let ch = char::from_u32(u32::from(*unit)).unwrap_or(char::REPLACEMENT_CHARACTER);
                write!(f, "{ch}")
            }
            Value::Int32(value) => write!(f, "{value}"),
            Value::Int64(value) => write!(f, "{value}"),
            Value::UInt32(value) => write!(f, "{value}"),
            Value::UInt64(value) => write!(f, "{value}"),
            Value::Int16(value) => write!(f, "{value}"),
            Value::UInt16(value) => write!(f, "{value}"),
            Value::Byte(value) => write!(f, "{value}"),
            Value::SByte(value) => write!(f, "{value}"),
            Value::Decimal(value) | Value::BigInteger(value) | Value::Type(value) => {
                fmt::Display::fmt(value, f)
            }
            Value::Double(value) => f.write_str(&format_double(*value)),
            Value::ScriptBlock(text) => fmt::Display::fmt(text, f),
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{item}")?;
                }
                Ok(())
            }
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
            Value::String(value) => fmt::Display::fmt(value, f),
        }
    }
}

fn format_double(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return value.to_string();
    }
    let mut text = format!("{value:.15}");
    if let Some(dot) = text.find('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.len() == dot + 1 {
            text.pop();
        }
    }
    text
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
