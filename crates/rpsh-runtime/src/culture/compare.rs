//! The return value of String.Compare is observable even when its sign would
//! suffice for sorting. String.CompareOrdinal has a separate whole-string path.
use super::{StringComparison, invariant_mode, text::ordinal_upper_char};
use crate::RuntimeError;
use std::char::DecodeUtf16Error;

fn length_difference(a: &[u16], b: &[u16]) -> i32 {
    a.len() as i32 - b.len() as i32
}

fn compare_units(a: &[u16], b: &[u16]) -> i32 {
    a.iter()
        .zip(b)
        .find_map(|(&x, &y)| (x != y).then_some(i32::from(x) - i32::from(y)))
        .unwrap_or_else(|| length_difference(a, b))
}

fn whole_ordinal(a: &[u16], b: &[u16]) -> i32 {
    for index in 0..2 {
        let x = a.get(index).copied().unwrap_or(0);
        let y = b.get(index).copied().unwrap_or(0);
        if x != y {
            return i32::from(x) - i32::from(y);
        }
    }
    if a.len().min(b.len()) <= 2 {
        length_difference(a, b)
    } else {
        compare_units(&a[2..], &b[2..])
    }
}

fn upper_scalar(unit: Result<char, DecodeUtf16Error>) -> i32 {
    match unit {
        Ok(ch) => ordinal_upper_char(ch) as i32,
        Err(err) => i32::from(err.unpaired_surrogate()),
    }
}

pub(super) fn ignore_case_key(text: &[u16]) -> Vec<u8> {
    char::decode_utf16(text.iter().copied())
        .flat_map(|unit| upper_scalar(unit).to_be_bytes())
        .collect()
}

fn compare_ignore_case(a: &[u16], b: &[u16]) -> i32 {
    let mut left = char::decode_utf16(a.iter().copied());
    let mut right = char::decode_utf16(b.iter().copied());
    while let (Some(x), Some(y)) = (left.next(), right.next()) {
        let x = upper_scalar(x);
        let y = upper_scalar(y);
        if x != y {
            // OrdinalCasing orders a surrogate pair after any unpaired unit.
            // InvariantModeCasing returns the full scalar difference instead.
            if (x > 0xffff) != (y > 0xffff) && !invariant_mode() {
                return if x > 0xffff { 1 } else { -1 };
            }
            return x - y;
        }
    }
    length_difference(a, b)
}

pub(super) fn ordinal(a: &[u16], b: &[u16], sensitive: bool) -> i32 {
    if sensitive {
        compare_units(a, b)
    } else {
        compare_ignore_case(a, b)
    }
}

pub fn compare_locale(
    a: &[u16],
    b: &[u16],
    locale: &str,
    sensitive: bool,
) -> Result<i32, RuntimeError> {
    if a == b {
        return Ok(0);
    }
    if invariant_mode() {
        return Ok(ordinal(a, b, sensitive));
    }
    #[cfg(unix)]
    if let Some(order) = super::icu::compare(a, b, locale, sensitive) {
        return Ok(order as i32);
    }
    Err(RuntimeError::NotImplemented {
        feature: "system ICU string collation",
    })
}

impl StringComparison {
    pub fn compare(self, a: &[u16], b: &[u16], whole: bool) -> Result<i32, RuntimeError> {
        match self {
            Self::Ordinal if whole => Ok(whole_ordinal(a, b)),
            Self::Ordinal | Self::OrdinalIgnoreCase => Ok(ordinal(a, b, self.sensitive())),
            _ => compare_locale(a, b, &self.locale(), self.sensitive()),
        }
    }
}
