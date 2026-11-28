//! The PowerShell string: a sequence of UTF-16 code units.
//!
//! DESIGN.md §3.3 and docs/02_value_model.md settle the representation:
//! ASCII content is stored as bytes and anything else as UTF-16 code units,
//! never both ("compact"). `.Length`, indexing and `Substring` therefore
//! count UTF-16 units and a lone surrogate is a valid string that becomes
//! U+FFFD only when written as UTF-8, as .NET does.
//!
//! The handle is one thin pointer, so `Value` stays 16 bytes. Collapsing the
//! handle and the payload into one allocation is tracked in #35; the
//! observable string behaviour is the same.

use std::borrow::Cow;
use std::fmt;
use std::rc::Rc;

enum Repr {
    Ascii(Vec<u8>),
    Wide(Vec<u16>),
}

impl Repr {
    fn from_utf8(text: &str) -> Self {
        if text.is_ascii() {
            Self::Ascii(text.as_bytes().to_vec())
        } else {
            Self::Wide(text.encode_utf16().collect())
        }
    }
}

enum Storage {
    String(Repr),
    // The reference attaches <CommandParameterName> to unbound strings in
    // $args. Copies retain the note when a later invocation splats them.
    Parameter(Box<(Repr, String)>),
}

/// An immutable PowerShell `System.String`.
#[derive(Clone)]
pub struct PsString(Rc<Storage>);

impl PsString {
    fn new(repr: Repr) -> Self {
        Self(Rc::new(Storage::String(repr)))
    }

    fn repr(&self) -> &Repr {
        match &*self.0 {
            Storage::String(text) => text,
            Storage::Parameter(parameter) => &parameter.0,
        }
    }

    /// The original spelling of an unbound command parameter in a function.
    pub fn command_parameter(text: &str, name: &str) -> Self {
        Self(Rc::new(Storage::Parameter(Box::new((
            Repr::from_utf8(text),
            name.to_owned(),
        )))))
    }

    pub fn command_parameter_name(&self) -> Option<&str> {
        match &*self.0 {
            Storage::Parameter(parameter) => Some(&parameter.1),
            Storage::String(_) => None,
        }
    }

    /// A string from valid UTF-8. Non-ASCII input is stored as UTF-16 units;
    /// an unpaired surrogate cannot occur here.
    pub fn from_utf8(text: &str) -> Self {
        Self::new(Repr::from_utf8(text))
    }

    /// A string from UTF-16 code units, keeping lone surrogates.
    pub fn from_units(units: Vec<u16>) -> Self {
        if units.iter().all(|&unit| unit < 0x80) {
            Self::new(Repr::Ascii(units.iter().map(|&unit| unit as u8).collect()))
        } else {
            Self::new(Repr::Wide(units))
        }
    }

    /// `.Length`: the number of UTF-16 code units.
    pub fn len(&self) -> usize {
        match self.repr() {
            Repr::Ascii(bytes) => bytes.len(),
            Repr::Wide(units) => units.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The UTF-16 code unit at `index`, if any.
    pub fn unit(&self, index: usize) -> Option<u16> {
        match self.repr() {
            Repr::Ascii(bytes) => bytes.get(index).map(|&byte| u16::from(byte)),
            Repr::Wide(units) => units.get(index).copied(),
        }
    }

    /// The UTF-16 code units, in order.
    pub fn units(&self) -> Box<dyn Iterator<Item = u16> + '_> {
        match self.repr() {
            Repr::Ascii(bytes) => Box::new(bytes.iter().map(|&byte| u16::from(byte))),
            Repr::Wide(units) => Box::new(units.iter().copied()),
        }
    }

    /// A contiguous UTF-16 view for APIs that take a pointer and unit count.
    pub fn as_utf16(&self) -> Cow<'_, [u16]> {
        match self.repr() {
            Repr::Ascii(bytes) => Cow::Owned(bytes.iter().copied().map(u16::from).collect()),
            Repr::Wide(units) => Cow::Borrowed(units),
        }
    }

    /// A new string with `other` appended.
    pub fn concat(&self, other: &Self) -> Self {
        match (self.repr(), other.repr()) {
            (Repr::Ascii(a), Repr::Ascii(b)) => {
                let mut bytes = Vec::with_capacity(a.len() + b.len());
                bytes.extend_from_slice(a);
                bytes.extend_from_slice(b);
                Self::new(Repr::Ascii(bytes))
            }
            _ => {
                let mut units = Vec::with_capacity(self.len() + other.len());
                units.extend(self.units());
                units.extend(other.units());
                PsString::from_units(units)
            }
        }
    }

    /// The string as UTF-8. A lone surrogate becomes U+FFFD, as .NET's UTF-8
    /// encoder does at the boundary.
    pub fn to_string_lossy(&self) -> String {
        match self.repr() {
            Repr::Ascii(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            Repr::Wide(units) => String::from_utf16_lossy(units),
        }
    }

    /// The text if it is ASCII, for fast paths.
    pub fn as_ascii(&self) -> Option<&str> {
        match self.repr() {
            Repr::Ascii(bytes) => std::str::from_utf8(bytes).ok(),
            Repr::Wide(_) => None,
        }
    }
}

impl Default for PsString {
    fn default() -> Self {
        PsString::from_utf8("")
    }
}

impl From<&str> for PsString {
    fn from(text: &str) -> Self {
        PsString::from_utf8(text)
    }
}

impl From<String> for PsString {
    fn from(text: String) -> Self {
        PsString::from_utf8(&text)
    }
}

impl PartialEq for PsString {
    fn eq(&self, other: &Self) -> bool {
        if let (Some(a), Some(b)) = (self.as_ascii(), other.as_ascii()) {
            return a == b;
        }
        self.len() == other.len() && self.units().eq(other.units())
    }
}

impl Eq for PsString {}

impl fmt::Display for PsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

impl fmt::Debug for PsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_counts_utf16_code_units() {
        assert_eq!(PsString::from("abc").len(), 3);
        assert_eq!(PsString::from("日本語").len(), 3);
        assert_eq!(PsString::from("😀").len(), 2);
    }

    #[test]
    fn a_lone_surrogate_is_kept_and_written_as_replacement() {
        let text = PsString::from_units(vec![0x61, 0xD800, 0x62]);
        assert_eq!(text.len(), 3);
        assert_eq!(text.unit(1), Some(0xD800));
        assert_eq!(text.to_string_lossy(), "a\u{FFFD}b");
    }

    #[test]
    fn ascii_content_is_narrow() {
        let text = PsString::from("abc");
        assert!(text.as_ascii().is_some());
        let wide = PsString::from("é");
        assert!(wide.as_ascii().is_none());
        assert_eq!(wide.to_string_lossy(), "é");
    }

    #[test]
    fn concatenation_keeps_ascii_narrow() {
        let joined = PsString::from("a").concat(&PsString::from("b"));
        assert!(joined.as_ascii().is_some());
        assert_eq!(joined.to_string_lossy(), "ab");
    }
}
