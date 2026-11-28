//! String comparisons used by PowerShell operators and .NET string methods.

use crate::{PsString, RuntimeError};
use std::cmp::Ordering;

mod search;
pub use search::StringComparison;
mod text;
use text::ordinal_upper;
pub use text::{NormalizationError, NormalizationForm, change_case};

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

/// Windows locale identifiers are accepted by the Culture cmdlet parameter.
pub fn locale_for_lcid(id: u32) -> Option<String> {
    if id == 0x7f {
        return Some(String::new());
    }
    #[cfg(unix)]
    if !invariant_mode() {
        return icu::locale_for_lcid(id);
    }
    None
}

/// Canonical BCP 47 name and Windows LCID for a locale supported on Unix.
pub fn locale_details(locale: &str) -> Option<(String, u32)> {
    if locale.is_empty() || locale.eq_ignore_ascii_case("C") {
        return Some((String::new(), 127));
    }
    if locale.eq_ignore_ascii_case("root") {
        return Some((String::new(), 4096));
    }
    if !locale
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return None;
    }
    #[cfg(unix)]
    if !invariant_mode() {
        return icu::locale_details(locale);
    }
    None
}

/// Convert ICU's time pattern into DateTimeFormatInfo syntax.
pub fn time_pattern(locale: &str, short: bool) -> Option<String> {
    if locale.is_empty() {
        return Some(if short { "HH:mm" } else { "HH:mm:ss" }.to_owned());
    }
    #[cfg(unix)]
    if let Some(pattern) = icu::time_pattern(locale, short) {
        let mut result = String::new();
        let mut quoted = false;
        let mut period = false;
        let mut chars = pattern.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => result.push_str("\\\\"),
                '\'' if chars.next_if_eq(&'\'').is_some() => result.push_str("\\\'"),
                '\'' => {
                    quoted = !quoted;
                    result.push(ch);
                }
                _ if quoted => result.push(ch),
                'a' | 'b' | 'B' if !period => {
                    result.push_str("tt");
                    period = true;
                }
                'H' | 'h' | 'm' | 's' => result.push(ch),
                other if !other.is_ascii_alphabetic() => result.push(other),
                _ => {}
            }
        }
        return Some(result);
    }
    None
}

fn invariant_mode() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("DOTNET_SYSTEM_GLOBALIZATION_INVARIANT")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    })
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

/// A key for repeated comparisons with the same locale and case option.
pub fn sort_key(
    text: &PsString,
    locale: &str,
    case_sensitive: bool,
) -> Result<Vec<u8>, RuntimeError> {
    let units = text.as_utf16();
    if invariant_mode() {
        let units = if case_sensitive {
            units
        } else {
            ordinal_upper(&units).into()
        };
        return Ok(units.iter().flat_map(|unit| unit.to_be_bytes()).collect());
    }
    #[cfg(unix)]
    if let Some(key) = icu::sort_key(&units, locale, case_sensitive) {
        return Ok(key);
    }
    Err(RuntimeError::NotImplemented {
        feature: "system ICU string collation",
    })
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
