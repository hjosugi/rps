//! String comparisons used by PowerShell operators and .NET string methods.

use crate::{PsString, RuntimeError};
use std::cmp::Ordering;

#[cfg(unix)]
mod icu;

/// ICU uses the empty locale name for the invariant (root) culture.
pub fn current_locale() -> String {
    if invariant_mode() {
        return String::new();
    }
    #[cfg(unix)]
    if let Some(locale) = icu::default_locale() {
        return locale;
    }
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(std::env::var_os)
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    let value = value.to_string_lossy();
    let locale = value.split('.').next().unwrap_or("");
    match locale {
        "C" | "POSIX" => String::new(),
        _ => locale.to_owned(),
    }
}

fn invariant_mode() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("DOTNET_SYSTEM_GLOBALIZATION_INVARIANT")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    })
}

// Ordinal ignore-case uses one-to-one uppercase mappings, retaining unpaired
// UTF-16 units. It does not expand ligatures or fold a dotless i into ASCII I.
fn ordinal_upper(units: &[u16]) -> Vec<u16> {
    let mut folded = Vec::with_capacity(units.len());
    for item in char::decode_utf16(units.iter().copied()) {
        match item {
            Ok(ch) => {
                let mut mapped = ch.to_uppercase();
                let first = mapped.next().unwrap_or(ch);
                let upper = if mapped.next().is_some() || matches!(ch, '\u{131}' | '\u{17f}') {
                    ch
                } else {
                    first
                };
                folded.extend_from_slice(upper.encode_utf16(&mut [0; 2]));
            }
            Err(error) => folded.push(error.unpaired_surrogate()),
        }
    }
    folded
}

pub fn ordinal_compare(left: &[u16], right: &[u16], case_sensitive: bool) -> Ordering {
    if case_sensitive {
        left.cmp(right)
    } else {
        ordinal_upper(left).cmp(&ordinal_upper(right))
    }
}

pub fn ordinal_has_affix(
    text: &[u16],
    pattern: &[u16],
    case_sensitive: bool,
    prefix: bool,
) -> bool {
    if text.len() < pattern.len() {
        return false;
    }
    let start = if prefix {
        0
    } else {
        text.len() - pattern.len()
    };
    ordinal_compare(&text[start..start + pattern.len()], pattern, case_sensitive).is_eq()
}

pub fn has_affix(
    text: &PsString,
    pattern: &PsString,
    locale: &str,
    case_sensitive: bool,
    prefix: bool,
) -> Result<bool, RuntimeError> {
    if pattern.is_empty() {
        return Ok(true);
    }
    let text = text.as_utf16();
    let pattern = pattern.as_utf16();
    if invariant_mode() {
        return Ok(ordinal_has_affix(&text, &pattern, case_sensitive, prefix));
    }
    #[cfg(unix)]
    if let Some(matched) = icu::has_affix(&text, &pattern, locale, case_sensitive, prefix) {
        return Ok(matched);
    }
    Err(RuntimeError::NotImplemented {
        feature: "system ICU string collation",
    })
}

/// TextInfo.ToLower(char) maps one code unit. Surrogates remain intact and
/// dotted/dotless I depend on whether the culture is invariant or Turkic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Casing {
    Invariant,
    Turkic,
    Other,
}

impl Casing {
    pub(crate) fn for_locale(locale: &str) -> Self {
        let language = locale.split(['_', '-']).next().unwrap_or("");
        if language.is_empty() || language.eq_ignore_ascii_case("root") {
            Self::Invariant
        } else if language.eq_ignore_ascii_case("tr") || language.eq_ignore_ascii_case("az") {
            Self::Turkic
        } else {
            Self::Other
        }
    }

    pub(crate) fn lower(self, unit: u16) -> u16 {
        match unit {
            0x49 if self == Self::Turkic => 0x131,
            0x130 if self == Self::Invariant => unit,
            0x130 => 0x69,
            0..=0x7f => (unit as u8).to_ascii_lowercase().into(),
            _ => char::from_u32(u32::from(unit))
                .and_then(|ch| ch.to_lowercase().next())
                .and_then(|lower| u16::try_from(u32::from(lower)).ok())
                .unwrap_or(unit),
        }
    }
}

/// String.CompareTo exposes the raw difference in globalization-invariant mode.
pub fn compare_to(left: &PsString, right: &PsString) -> Result<i32, RuntimeError> {
    if invariant_mode() {
        for (a, b) in left.units().zip(right.units()) {
            if a != b {
                return Ok(i32::from(a) - i32::from(b));
            }
        }
        return Ok(left.len() as i32 - right.len() as i32);
    }
    Ok(compare(left, right, &current_locale(), true)? as i32)
}

/// `locale = ""` selects the invariant *culture*, which still uses ICU.
/// The separately configured globalization-invariant mode uses ordinal rules.
pub fn compare(
    left: &PsString,
    right: &PsString,
    locale: &str,
    case_sensitive: bool,
) -> Result<Ordering, RuntimeError> {
    if left == right {
        return Ok(Ordering::Equal);
    }
    let left = left.as_utf16();
    let right = right.as_utf16();
    if invariant_mode() {
        return Ok(ordinal_compare(&left, &right, case_sensitive));
    }
    #[cfg(unix)]
    if let Some(order) = icu::compare(&left, &right, locale, case_sensitive) {
        return Ok(order);
    }
    Err(RuntimeError::NotImplemented {
        feature: "system ICU string collation",
    })
}
