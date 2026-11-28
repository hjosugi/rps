//! UTF-16 matching with System.StringComparison.
use super::{current_locale, invariant_mode};
use crate::{PsString, RuntimeError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum StringComparison {
    CurrentCulture,
    CurrentCultureIgnoreCase,
    InvariantCulture,
    InvariantCultureIgnoreCase,
    Ordinal,
    OrdinalIgnoreCase,
}

impl StringComparison {
    pub const ENTRIES: &[(&str, i32, Self)] = &[
        ("CurrentCulture", 0, Self::CurrentCulture),
        (
            "CurrentCultureIgnoreCase",
            1,
            Self::CurrentCultureIgnoreCase,
        ),
        ("InvariantCulture", 2, Self::InvariantCulture),
        (
            "InvariantCultureIgnoreCase",
            3,
            Self::InvariantCultureIgnoreCase,
        ),
        ("Ordinal", 4, Self::Ordinal),
        ("OrdinalIgnoreCase", 5, Self::OrdinalIgnoreCase),
    ];
    pub fn sensitive(self) -> bool {
        matches!(
            self,
            Self::CurrentCulture | Self::InvariantCulture | Self::Ordinal
        )
    }
    pub fn ordinal(self) -> bool {
        matches!(self, Self::Ordinal | Self::OrdinalIgnoreCase) || invariant_mode()
    }
    pub fn locale(self) -> String {
        if matches!(self, Self::CurrentCulture | Self::CurrentCultureIgnoreCase) {
            current_locale()
        } else {
            String::new()
        }
    }

    pub fn search(
        self,
        text: &[u16],
        pattern: &[u16],
        reverse: bool,
    ) -> Result<Option<usize>, RuntimeError> {
        if pattern.is_empty() {
            return Ok(Some(if reverse { text.len() } else { 0 }));
        }
        if self.ordinal() {
            let mut windows = text.windows(pattern.len());
            let folded = (!self.sensitive()).then(|| super::ordinal_upper(pattern));
            let contains = |window: &[u16]| match &folded {
                None => window == pattern,
                Some(target) if window.iter().all(|&u| u < 128) => window
                    .iter()
                    .zip(target)
                    .all(|(&a, &b)| u16::from((a as u8).to_ascii_uppercase()) == b),
                Some(target) => super::ordinal_upper(window) == *target,
            };
            return Ok(if reverse {
                windows.rposition(contains)
            } else {
                windows.position(contains)
            });
        }
        #[cfg(unix)]
        if let Some(at) =
            super::icu::search(text, pattern, &self.locale(), self.sensitive(), reverse)
        {
            return Ok(usize::try_from(at).ok());
        }
        Err(RuntimeError::NotImplemented {
            feature: "system ICU string search",
        })
    }

    pub fn affix(
        self,
        text: &PsString,
        pattern: &PsString,
        prefix: bool,
    ) -> Result<bool, RuntimeError> {
        if self.ordinal() {
            return Ok(super::ordinal_has_affix(
                &text.as_utf16(),
                &pattern.as_utf16(),
                self.sensitive(),
                prefix,
            ));
        }
        super::has_affix(text, pattern, &self.locale(), self.sensitive(), prefix)
    }
}
