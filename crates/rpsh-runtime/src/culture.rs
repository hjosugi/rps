//! String comparisons used by PowerShell operators and .NET string methods.

use crate::{PsString, RuntimeError};
use std::cmp::Ordering;

#[cfg(unix)]
mod icu;

/// ICU uses the empty locale name for the invariant (root) culture.
pub fn current_locale() -> String {
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
