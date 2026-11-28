//! System.String's one-to-one casing and Unicode normalization.
use super::{Casing, invariant_mode};
use crate::{PsString, RuntimeError};

pub fn change_case(text: &PsString, locale: &str, upper: bool) -> Result<PsString, RuntimeError> {
    let casing = Casing::for_locale(locale);
    if let Some(ascii) = text.as_ascii()
        && casing != Casing::Turkic
    {
        return Ok(if upper {
            ascii.to_ascii_uppercase()
        } else {
            ascii.to_ascii_lowercase()
        }
        .into());
    }
    if invariant_mode() {
        return Ok(PsString::from_units(map_units(
            text.units(),
            Casing::Invariant,
            upper,
            true,
            simple_case,
        )));
    }
    #[cfg(unix)]
    if let Some(result) = super::icu::change_case(text, casing, upper) {
        return Ok(result);
    }
    Err(RuntimeError::NotImplemented {
        feature: "system ICU string casing",
    })
}

// The Unicode simple mapping differs from the full mapping of Greek letters
// with prosgegrammeni. .NET preserves one scalar per input scalar.
fn simple_case(ch: char, upper: bool) -> char {
    if !upper {
        return ch.to_lowercase().next().unwrap_or(ch);
    }
    let mut mapped = ch.to_uppercase();
    let first = mapped.next().unwrap_or(ch);
    if mapped.next().is_none() {
        return first;
    }
    match ch as u32 {
        c @ (0x1f80..=0x1f87 | 0x1f90..=0x1f97 | 0x1fa0..=0x1fa7) => {
            char::from_u32(c + 8).unwrap_or(ch)
        }
        0x1fb3 => '\u{1fbc}',
        0x1fc3 => '\u{1fcc}',
        0x1ff3 => '\u{1ffc}',
        _ => ch,
    }
}

pub(super) fn ordinal_upper(units: &[u16]) -> Vec<u16> {
    map_units(
        units.iter().copied(),
        Casing::Invariant,
        true,
        true,
        simple_case,
    )
}

pub(super) fn map_units(
    units: impl Iterator<Item = u16>,
    casing: Casing,
    upper: bool,
    ordinal: bool,
    map: impl Fn(char, bool) -> char,
) -> Vec<u16> {
    let mut result = Vec::with_capacity(units.size_hint().0);
    for decoded in char::decode_utf16(units) {
        let ch = match decoded {
            Ok(ch) => ch,
            Err(err) => {
                result.push(err.unpaired_surrogate());
                continue;
            }
        };
        let mapped = match (ch, upper, casing) {
            ('\u{131}', true, Casing::Invariant) | ('\u{130}', false, Casing::Invariant) => ch,
            ('\u{17f}', true, _) if ordinal => ch,
            ('i', true, Casing::Turkic) => '\u{130}',
            ('I', false, Casing::Turkic) => '\u{131}',
            _ => map(ch, upper),
        };
        result.extend_from_slice(mapped.encode_utf16(&mut [0; 2]));
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizationForm {
    C,
    D,
    Kc,
    Kd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizationError {
    InvalidUnicode,
    Unavailable,
}

impl NormalizationForm {
    fn needs_icu(text: &PsString) -> Result<bool, NormalizationError> {
        if text.as_ascii().is_some() || invariant_mode() {
            return Ok(false);
        }
        if char::decode_utf16(text.units()).any(|ch| matches!(ch, Err(_) | Ok('\u{fffe}'))) {
            return Err(NormalizationError::InvalidUnicode);
        }
        Ok(true)
    }

    pub fn normalize(self, text: &PsString) -> Result<PsString, NormalizationError> {
        if !Self::needs_icu(text)? {
            return Ok(text.clone());
        }
        #[cfg(unix)]
        if let Some(units) = super::icu::normalize(&text.as_utf16(), self) {
            return Ok(PsString::from_units(units));
        }
        Err(NormalizationError::Unavailable)
    }

    pub fn is_normalized(self, text: &PsString) -> Result<bool, NormalizationError> {
        if !Self::needs_icu(text)? {
            return Ok(true);
        }
        #[cfg(unix)]
        if let Some(result) = super::icu::is_normalized(&text.as_utf16(), self) {
            return Ok(result);
        }
        Err(NormalizationError::Unavailable)
    }
}
