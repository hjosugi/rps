//! PowerShell wildcard strings are sequences of UTF-16 code units.
use crate::{PsString, RuntimeError};

const ESCAPE: u16 = b'`' as u16;

fn wildcard_character(unit: u16) -> bool {
    matches!(unit, 0x2a | 0x3f | 0x5b | 0x5d)
}

fn required(pattern: Option<&PsString>) -> Result<&PsString, RuntimeError> {
    pattern.ok_or_else(|| RuntimeError::Script {
        id: "PSArgumentNullException",
        category: "InvalidArgument: (:) [], PSArgumentNullException".into(),
        exception_type: "System.Management.Automation.PSArgumentNullException",
        message: "Cannot process argument because the value of argument \"pattern\" is null. Change the value of argument \"pattern\" to a non-null value.".into(),
    })
}

/// Protect unescaped metacharacters, including the escape character itself.
/// The internal overload allows callers to leave specific characters alone.
pub fn escape(pattern: Option<&PsString>, except: &[u16]) -> Result<PsString, RuntimeError> {
    let pattern = required(pattern)?;
    let should_escape =
        |unit| (unit == ESCAPE || wildcard_character(unit)) && !except.contains(&unit);
    if !pattern.units().any(should_escape) {
        return Ok(pattern.clone());
    }
    let mut result = Vec::with_capacity(pattern.len());
    for unit in pattern.units() {
        if should_escape(unit) {
            result.push(ESCAPE);
        }
        result.push(unit);
    }
    Ok(PsString::from_units(result))
}

/// A backtick before an ordinary character or at the end stays literal.
pub fn unescape(pattern: Option<&PsString>) -> Result<PsString, RuntimeError> {
    let pattern = required(pattern)?;
    if !pattern.units().any(|unit| unit == ESCAPE) {
        return Ok(pattern.clone());
    }
    let mut result = Vec::with_capacity(pattern.len());
    let mut units = pattern.units().peekable();
    while let Some(unit) = units.next() {
        if unit == ESCAPE
            && let Some(escaped) = units.next_if(|&next| next == ESCAPE || wildcard_character(next))
        {
            result.push(escaped);
        } else {
            result.push(unit);
        }
    }
    Ok(PsString::from_units(result))
}

pub fn contains_wildcard_characters(units: impl IntoIterator<Item = u16>) -> bool {
    let mut units = units.into_iter();
    while let Some(unit) = units.next() {
        if wildcard_character(unit) {
            return true;
        }
        if unit == ESCAPE {
            units.next();
        }
    }
    false
}

// Cases ported from PowerShell v7.7.0 test/xUnit/csharp/test_WildcardPattern.cs,
// Copyright (c) Microsoft Corporation, licensed under the MIT License.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_pattern_raises_argument_null() {
        for result in [escape(None, &[]), unescape(None)] {
            let Err(RuntimeError::Script { exception_type, .. }) = result else {
                panic!("expected PSArgumentNullException");
            };
            assert_eq!(
                exception_type,
                "System.Management.Automation.PSArgumentNullException"
            );
        }
    }

    #[test]
    fn escapes() {
        for (input, expected) in [
            ("", ""),
            ("a", "a"),
            ("a*", "a`*"),
            ("*?[]", "`*`?`[`]"),
            ("`", "``"),
            ("```", "``````"),
            ("a`", "a``"),
            ("abc`def", "abc``def"),
            ("abc``def", "abc````def"),
            ("text with `backticks", "text with ``backticks"),
            ("ends with `", "ends with ``"),
            ("`starts with", "``starts with"),
            ("`in`between`", "``in``between``"),
            ("no special characters", "no special characters"),
            ("`*`", "```*``"),
            ("*`?[]", "`*```?`[`]"),
            ("`*`?`[", "```*```?```["),
            ("*?[]`", "`*`?`[`]``"),
            (
                "nested `backticks `inside`",
                "nested ``backticks ``inside``",
            ),
            ("wildcard*with`backtick", "wildcard`*with``backtick"),
        ] {
            let input = PsString::from(input);
            assert_eq!(escape(Some(&input), &[]).unwrap(), PsString::from(expected));
        }
    }

    #[test]
    fn keeps_exceptions() {
        for (input, expected) in [("a", "a"), ("a*", "a*"), ("*?[]", "*?[]")] {
            let input = PsString::from(input);
            assert_eq!(
                escape(Some(&input), &[42, 63, 91, 93]).unwrap(),
                PsString::from(expected)
            );
        }
    }

    #[test]
    fn unescapes() {
        for (input, expected) in [("", ""), ("a", "a"), ("a`*", "a*"), ("`*`?`[`]", "*?[]")] {
            let input = PsString::from(input);
            assert_eq!(unescape(Some(&input)).unwrap(), PsString::from(expected));
        }
    }

    #[test]
    fn unpaired_surrogates_survive_escaping() {
        let units = [0xd800, 42, 0xdc00];
        let pattern = PsString::from_units(units.to_vec());
        let escaped = escape(Some(&pattern), &[]).unwrap();
        assert_eq!(
            escaped.units().collect::<Vec<_>>(),
            [0xd800, ESCAPE, 42, 0xdc00]
        );
        assert_eq!(unescape(Some(&escaped)).unwrap(), pattern);
    }
}
