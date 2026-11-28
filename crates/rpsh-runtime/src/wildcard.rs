//! PowerShell wildcard strings are sequences of UTF-16 code units.
use crate::{PsString, RuntimeError};

const ESCAPE: u16 = b'`' as u16;

fn wildcard_character(unit: u16) -> bool {
    matches!(unit, 0x2a | 0x3f | 0x5b | 0x5d)
}

/// Direct API calls reject a null pattern.
pub fn require_pattern(pattern: Option<&PsString>) -> Result<&PsString, RuntimeError> {
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
    let pattern = require_pattern(pattern)?;
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
    let pattern = require_pattern(pattern)?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Atom {
    Any,
    Star,
    Literal(u16),
    Class(Vec<ClassMember>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClassMember {
    Literal(u16),
    Range(u16, u16),
}

fn invalid_pattern(pattern: &PsString) -> RuntimeError {
    RuntimeError::Script {
        id: "WildcardPatternException",
        category: "InvalidArgument: (:) [], WildcardPatternException".into(),
        exception_type: "System.Management.Automation.WildcardPatternException",
        message: format!("The specified wildcard character pattern is not valid: {pattern}"),
    }
}

fn parse_class(
    units: &mut impl Iterator<Item = u16>,
    pattern: &PsString,
) -> Result<Atom, RuntimeError> {
    // A right bracket immediately following the opening one is a member.
    // A quoted dash must survive as a literal even in the middle of a class.
    let mut contents = Vec::new();
    loop {
        let unit = units.next().ok_or_else(|| invalid_pattern(pattern))?;
        match unit {
            0x5d if !contents.is_empty() => break,
            ESCAPE => contents.push((units.next().ok_or_else(|| invalid_pattern(pattern))?, false)),
            _ => contents.push((unit, unit == 0x2d)),
        }
    }
    let mut rest = contents.as_slice();
    let mut members = Vec::new();
    while let Some((&(first, _), tail)) = rest.split_first() {
        if let [(_, true), (last, _), remaining @ ..] = tail {
            if first > *last {
                return Err(invalid_pattern(pattern));
            }
            members.push(ClassMember::Range(first, *last));
            rest = remaining;
        } else {
            members.push(ClassMember::Literal(first));
            rest = tail;
        }
    }
    Ok(Atom::Class(members))
}

fn parse_atoms(pattern: &PsString) -> Result<Vec<Atom>, RuntimeError> {
    let mut units = pattern.units();
    let mut atoms = Vec::new();
    while let Some(unit) = units.next() {
        let atom = match unit {
            0x2a => Atom::Star,
            0x3f => Atom::Any,
            0x5b => parse_class(&mut units, pattern)?,
            ESCAPE => match units.next() {
                Some(quoted) => Atom::Literal(quoted),
                None if atoms.is_empty() => break, // Upstream matches a lone backtick to the empty string.
                None => Atom::Literal(ESCAPE),
            },
            literal => Atom::Literal(literal),
        };
        atoms.push(atom);
    }
    Ok(atoms)
}

/// Translate PowerShell wildcard tokens to the operand of WQL LIKE.
/// A class whose exact bounds WQL cannot represent requires client-side filtering.
pub fn to_wql(pattern: &PsString) -> Result<PsString, RuntimeError> {
    let unsupported = || RuntimeError::Script {
        id: "PSInvalidCastException",
        category: "InvalidArgument: (:) [], PSInvalidCastException".into(),
        exception_type: "System.Management.Automation.PSInvalidCastException",
        message: format!(
            "Cannot convert the \"{pattern}\" value of type \"System.Management.Automation.WildcardPattern\" to type \"WQL\"."
        ),
    };
    let reserved_bound = |unit| matches!(unit, 0x2d | 0x5b..=0x5e);
    let mut result = Vec::new();
    for atom in parse_atoms(pattern)? {
        match atom {
            Atom::Star => result.push(0x25),
            Atom::Any => result.push(0x5f),
            Atom::Literal(unit @ (0x25 | 0x5f | 0x5b)) => result.extend([0x5b, unit, 0x5d]),
            Atom::Literal(unit) => result.push(unit),
            Atom::Class(members) => {
                result.push(0x5b);
                for member in members {
                    match member {
                        ClassMember::Literal(0x2d | 0x5c..=0x5e) => return Err(unsupported()),
                        ClassMember::Literal(unit) => result.push(unit),
                        ClassMember::Range(first, last) => {
                            if reserved_bound(first) || reserved_bound(last) {
                                return Err(unsupported());
                            }
                            result.extend([first, 0x2d, last]);
                        }
                    }
                }
                result.push(0x5d);
            }
        }
    }
    Ok(PsString::from_units(result))
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
