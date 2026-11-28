//! PowerShell wildcard strings are sequences of UTF-16 code units.
use crate::{PsString, RuntimeError, Value, culture};

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

#[derive(Debug)]
enum Strategy {
    All,
    Literal(PsString),
    Prefix(PsString),
    Pattern(Vec<Atom>),
}

/// A parsed PowerShell wildcard can be reused across an input collection.
#[derive(Debug)]
pub struct Matcher {
    strategy: Strategy,
    locale: String,
    casing: Option<culture::Casing>,
    ordinal_ignore_case: bool,
}

impl Matcher {
    pub fn new(pattern: &PsString, options: i32) -> Result<Self, RuntimeError> {
        let locale = if options & 4 != 0 || pattern.as_ascii() == Some("*") {
            String::new()
        } else {
            culture::current_locale()
        };
        Self::for_locale(pattern, options & 2 != 0, locale)
    }

    fn for_locale(
        pattern: &PsString,
        ignore_case: bool,
        locale: String,
    ) -> Result<Self, RuntimeError> {
        let special = pattern
            .units()
            .position(|unit| wildcard_character(unit) || unit == ESCAPE);
        let casing = ignore_case.then(|| culture::Casing::for_locale(&locale));
        let strategy = match special {
            None => Strategy::Literal(pattern.clone()),
            Some(0) if pattern.len() == 1 && pattern.unit(0) == Some(0x2a) => Strategy::All,
            Some(last) if last + 1 == pattern.len() && pattern.unit(last) == Some(0x2a) => {
                Strategy::Prefix(PsString::from_units(pattern.units().take(last).collect()))
            }
            _ => {
                let mut atoms = parse_atoms(pattern)?;
                if let Some(casing) = casing {
                    for atom in &mut atoms {
                        match atom {
                            Atom::Literal(unit) => *unit = casing.lower(*unit),
                            Atom::Class(members) => *members = fold_class(members, casing),
                            _ => {}
                        }
                    }
                }
                Strategy::Pattern(atoms)
            }
        };
        let ordinal_ignore_case =
            ignore_case && locale.replace('-', "_").eq_ignore_ascii_case("en_US_POSIX");
        Ok(Self {
            strategy,
            locale,
            casing,
            ordinal_ignore_case,
        })
    }

    pub fn is_match(&self, input: Option<&PsString>) -> Result<bool, RuntimeError> {
        let Some(input) = input else { return Ok(false) };
        let sensitive = self.casing.is_none();
        match &self.strategy {
            Strategy::All => Ok(true),
            Strategy::Literal(pattern) if self.ordinal_ignore_case => {
                Ok(culture::ordinal_compare(&input.as_utf16(), &pattern.as_utf16(), false).is_eq())
            }
            Strategy::Literal(pattern) => {
                Ok(culture::compare(input, pattern, &self.locale, sensitive)?.is_eq())
            }
            Strategy::Prefix(prefix) if self.ordinal_ignore_case => Ok(culture::ordinal_has_affix(
                &input.as_utf16(),
                &prefix.as_utf16(),
                false,
                true,
            )),
            Strategy::Prefix(prefix) => {
                culture::has_affix(input, prefix, &self.locale, sensitive, true)
            }
            Strategy::Pattern(atoms) => Ok(match_units(atoms, &input.as_utf16(), self.casing)),
        }
    }
}

impl ClassMember {
    fn limits(self) -> (u16, u16) {
        match self {
            Self::Literal(unit) => (unit, unit),
            Self::Range(low, high) => (low, high),
        }
    }
}

// Map the whole range before matching: transforming just its endpoints would
// drop punctuation in [A-z] and miss case equivalents outside the range.
fn fold_class(members: &[ClassMember], casing: culture::Casing) -> Vec<ClassMember> {
    let mut units = Vec::new();
    for member in members {
        let (low, high) = member.limits();
        units.extend((low..=high).map(|unit| casing.lower(unit)));
    }
    units.sort_unstable();
    units.dedup();
    let mut ranges = Vec::new();
    let mut rest = units.as_slice();
    while let Some((&low, tail)) = rest.split_first() {
        let mut high = low;
        rest = tail;
        while let Some((&next, tail)) = rest.split_first() {
            if high.checked_add(1) != Some(next) {
                break;
            }
            high = next;
            rest = tail;
        }
        ranges.push(ClassMember::Range(low, high));
    }
    ranges
}

fn match_units(atoms: &[Atom], text: &[u16], casing: Option<culture::Casing>) -> bool {
    let (mut pattern_at, mut text_at) = (0, 0);
    let mut star_retry = None;
    while text_at < text.len() {
        let unit = casing.map_or(text[text_at], |case| case.lower(text[text_at]));
        let matched = match atoms.get(pattern_at) {
            Some(Atom::Any) => true,
            Some(Atom::Literal(expected)) => *expected == unit,
            Some(Atom::Class(members)) => members.iter().any(|member| {
                let (low, high) = member.limits();
                (low..=high).contains(&unit)
            }),
            Some(Atom::Star) => {
                pattern_at += 1;
                star_retry = Some((pattern_at, text_at));
                continue;
            }
            None => false,
        };
        if matched {
            pattern_at += 1;
            text_at += 1;
        } else if let Some((after_star, consumed)) = &mut star_retry {
            *consumed += 1;
            text_at = *consumed;
            pattern_at = *after_star;
        } else {
            return false;
        }
    }
    atoms[pattern_at..]
        .iter()
        .all(|atom| matches!(atom, Atom::Star))
}

/// Read the pattern and options kept by the engine's WildcardPattern object.
pub fn stored(entries: &[(String, Value)]) -> Option<(&PsString, i32)> {
    let pattern = entries.iter().find_map(|(key, value)| match value {
        Value::String(pattern) if key == "__WildcardPattern" => Some(pattern),
        _ => None,
    })?;
    let options = entries.iter().find_map(|(key, value)| match value {
        Value::Int32(bits) if key == "__WildcardOptions" => Some(*bits),
        _ => None,
    })?;
    Some((pattern, options))
}

/// Operators surface the original WildcardPatternException as a RuntimeException.
pub fn operator_pattern(pattern: &PsString, options: i32) -> Result<Matcher, RuntimeError> {
    Matcher::new(pattern, options).map_err(|error| match error {
        RuntimeError::Script {
            id: "WildcardPatternException",
            exception_type,
            message,
            ..
        } => RuntimeError::Script {
            id: "RuntimeException",
            category: "NotSpecified: (:) [], WildcardPatternException".into(),
            exception_type,
            message,
        },
        other => other,
    })
}

pub fn operator_match(
    text: &PsString,
    pattern: &PsString,
    case_sensitive: bool,
) -> Result<bool, RuntimeError> {
    operator_pattern(pattern, if case_sensitive { 0 } else { 2 })?.is_match(Some(text))
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
    fn turkish_i_depends_on_culture_for_each_pattern_strategy() {
        for (locale, target, yes, no) in [
            ("tr_TR", "ı", "I", "i"),
            ("tr_TR", "İ", "i", "I"),
            ("", "I", "i", "ı"),
        ] {
            for (pattern, input) in [
                (yes.to_owned(), target.to_owned()),
                (format!("{yes}*"), format!("{target}more")),
                (format!("{yes}?"), format!("{target}x")),
                (format!("[{yes}]"), target.to_owned()),
            ] {
                let matcher =
                    Matcher::for_locale(&pattern.as_str().into(), true, locale.into()).unwrap();
                assert!(
                    matcher.is_match(Some(&input.as_str().into())).unwrap(),
                    "{locale} {pattern} {input}"
                );
            }
            let matcher =
                Matcher::for_locale(&format!("[{no}]").into(), true, locale.into()).unwrap();
            assert!(!matcher.is_match(Some(&target.into())).unwrap());
        }
    }

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
