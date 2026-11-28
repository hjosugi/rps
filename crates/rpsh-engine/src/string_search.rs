//! .NET string overloads bind a char or a string before choosing a search
//! range. Indices and counts always address UTF-16 code units.
use super::{convert_int32, globalization, hidden_type, overload_error};
use rpsh_runtime::{
    PsString, RuntimeError, Value,
    culture::{self, StringComparison},
    ops,
};

struct Term {
    text: PsString,
    character: bool,
}

impl Term {
    fn bind(
        value: &Value,
        method: &str,
        arity: usize,
        allow_char: bool,
    ) -> Result<Self, RuntimeError> {
        if hidden_type(value).as_deref() == Some("System.Management.Automation.Language.NullString")
        {
            return Err(globalization::invocation(
                "ArgumentNullException",
                method,
                arity,
                "Value cannot be null. (Parameter 'value')",
            ));
        }
        let character = if allow_char {
            match value {
                Value::Char(unit) => Some(*unit),
                v if v.is_null_like() => Some(0),
                Value::Int32(_)
                | Value::Int64(_)
                | Value::UInt32(_)
                | Value::UInt64(_)
                | Value::Int16(_)
                | Value::UInt16(_)
                | Value::Byte(_)
                | Value::SByte(_) => {
                    let unit=convert_int32(value).ok().and_then(|n|u16::try_from(n).ok()).ok_or_else(||{
                        let why=format!("Cannot convert value \"{value}\" to type \"System.Char\". Error: \"Value was either too large or too small for a character.\"");
                        globalization::argument_error(method,"value","System.Char",&value.to_string(),&why)
                    })?;
                    Some(unit)
                }
                Value::Bool(_) => {
                    return Err(RuntimeError::Script {
                        id: "MethodCountCouldNotFindBest",
                        category: "NotSpecified: (:) [], MethodException".into(),
                        exception_type: "System.Management.Automation.MethodException",
                        message: format!(
                            "Multiple ambiguous overloads found for \"{method}\" and the argument count: \"{arity}\"."
                        ),
                    });
                }
                _ => None,
            }
        } else {
            None
        };
        let text = character.map_or_else(
            || value.to_ps_string(),
            |unit| PsString::from_units(vec![unit]),
        );
        Ok(Self {
            text,
            character: character.is_some(),
        })
    }
}

fn integer(method: &str, name: &str, value: &Value) -> Result<i32, RuntimeError> {
    if matches!(value,Value::String(text) if text.is_empty()) {
        return Ok(0);
    }
    convert_int32(value).map_err(|_|{
        let why=format!("Cannot convert value \"{value}\" to type \"System.Int32\". Error: \"The input string '{value}' was not in a correct format.\"");
        globalization::argument_error(method,name,"System.Int32",&value.to_string(),&why)
    })
}

fn nonnegative(name: &str, value: i64) -> String {
    format!(
        "{name} ('{value}') must be a non-negative value. (Parameter '{name}')\nActual value was {value}."
    )
}

fn char_ignore_case_range(
    length: i64,
    start: i64,
    count: i64,
    reverse: bool,
) -> Result<(), String> {
    if start < 0 {
        return Err(nonnegative("startIndex", start));
    }
    if if reverse {
        start >= length
    } else {
        start > length
    } {
        let limit = if reverse {
            "less than"
        } else {
            "less than or equal to"
        };
        return Err(format!(
            "startIndex ('{start}') must be {limit} '{length}'. (Parameter 'startIndex')\nActual value was {start}."
        ));
    }
    if count < 0 {
        return Err(nonnegative("count", count));
    }
    if reverse {
        let offset = start + 1 - count;
        if offset < 0 {
            return Err(nonnegative("startSearchAt", offset));
        }
    } else if count > length - start {
        let maximum = length - start;
        return Err(format!(
            "count ('{count}') must be less than or equal to '{maximum}'. (Parameter 'count')\nActual value was {count}."
        ));
    }
    Ok(())
}

pub(super) fn substring(text: &PsString, arguments: &[Value]) -> Result<Value, RuntimeError> {
    let method = "Substring";
    let fail = |detail: &str| {
        globalization::invocation(
            "ArgumentOutOfRangeException",
            method,
            arguments.len(),
            detail,
        )
    };
    let (start, length) = match arguments {
        [s] => (integer(method, "startIndex", s)?, None),
        [s, l] => (
            integer(method, "startIndex", s)?,
            Some(integer(method, "length", l)?),
        ),
        _ => return Err(overload_error(method, arguments.len())),
    };
    if start < 0 {
        return Err(fail(&format!(
            "startIndex ('{start}') must be a non-negative value. (Parameter 'startIndex')\nActual value was {start}."
        )));
    }
    let start = start as usize;
    if start > text.len() {
        return Err(fail(
            "startIndex cannot be larger than length of string. (Parameter 'startIndex')",
        ));
    }
    let length = match length {
        None => text.len() - start,
        Some(n) if n < 0 => {
            return Err(fail(&format!(
                "length ('{n}') must be a non-negative value. (Parameter 'length')\nActual value was {n}."
            )));
        }
        Some(n) => n as usize,
    };
    if length > text.len() - start {
        return Err(fail(
            "Index and length must refer to a location within the string. (Parameter 'length')",
        ));
    }
    Ok(Value::String(PsString::from_units(
        text.as_utf16()[start..start + length].to_vec(),
    )))
}

pub(super) fn index(
    text: &PsString,
    arguments: &[Value],
    reverse: bool,
) -> Result<Value, RuntimeError> {
    let method = if reverse { "LastIndexOf" } else { "IndexOf" };
    let Some((value, rest)) = arguments.split_first() else {
        return Err(overload_error(method, 0));
    };
    if rest.len() > 3 {
        return Err(overload_error(method, arguments.len()));
    }
    let term = Term::bind(value, method, arguments.len(), true)?;
    let default = if term.character {
        StringComparison::Ordinal
    } else {
        StringComparison::CurrentCulture
    };
    let (mode, range) = match rest.split_last() {
        Some((mode, range))
            if rest.len() == 3
                || hidden_type(mode).as_deref() == Some(globalization::COMPARISON_TYPE) =>
        {
            (globalization::comparison(method, mode)?, range)
        }
        _ => (default, rest),
    };
    let length = text.len() as i64;
    let start = range
        .first()
        .map(|v| integer(method, "startIndex", v).map(i64::from))
        .transpose()?;
    let count = range
        .get(1)
        .map(|v| integer(method, "count", v).map(i64::from))
        .transpose()?;
    let fail = |detail: &str| {
        globalization::invocation(
            "ArgumentOutOfRangeException",
            method,
            arguments.len(),
            detail,
        )
    };
    let count_error = || {
        fail(
            "Count must be positive and count must refer to a location within the string/array/collection. (Parameter 'count')",
        )
    };
    if reverse && term.character && length == 0 {
        return Ok(Value::Int32(-1));
    }
    if term.character && mode == StringComparison::OrdinalIgnoreCase {
        let start = start.unwrap_or(if reverse { length - 1 } else { 0 });
        let count = count.unwrap_or(if reverse { start + 1 } else { length - start });
        char_ignore_case_range(length, start, count, reverse).map_err(|detail| fail(&detail))?;
    }
    let (offset, count) = if reverse {
        let mut start = start.unwrap_or(length - 1);
        let mut count = count.unwrap_or(start + 1);
        let ordinal_char = term.character && mode == StringComparison::Ordinal;
        if !ordinal_char && start == length {
            start -= 1;
            if count > 0 {
                count -= 1;
            }
        }
        if !ordinal_char && start == -1 && length == 0 {
            count = 0;
        } else if start < 0 || start >= length {
            return Err(fail(
                "Index was out of range. Must be non-negative and less than the size of the collection. (Parameter 'startIndex')",
            ));
        }
        if count < 0 || count > start + 1 {
            return Err(count_error());
        }
        (start + 1 - count, count)
    } else {
        let start = start.unwrap_or(0);
        if start < 0 || start > length {
            return Err(fail(
                "Index was out of range. Must be non-negative and less than or equal to the size of the collection. (Parameter 'startIndex')",
            ));
        }
        let count = count.unwrap_or(length - start);
        if count < 0 || count > length - start {
            return Err(count_error());
        }
        (start, count)
    };
    let units = text.as_utf16();
    let local = mode.search(
        &units[offset as usize..(offset + count) as usize],
        &term.text.as_utf16(),
        reverse,
    )?;
    Ok(Value::Int32(
        local.map_or(-1, |index| (offset as usize + index) as i32),
    ))
}

pub(super) fn contains(text: &PsString, arguments: &[Value]) -> Result<Value, RuntimeError> {
    let (value, mode) = match arguments {
        [v] => (v, StringComparison::Ordinal),
        [v, m] => (v, globalization::comparison("Contains", m)?),
        _ => return Err(overload_error("Contains", arguments.len())),
    };
    let term = Term::bind(value, "Contains", arguments.len(), true)?;
    Ok(Value::Bool(
        mode.search(&text.as_utf16(), &term.text.as_utf16(), false)?
            .is_some(),
    ))
}

pub(super) fn affix(
    text: &PsString,
    arguments: &[Value],
    prefix: bool,
) -> Result<Value, RuntimeError> {
    let method = if prefix { "StartsWith" } else { "EndsWith" };
    let value = arguments.first().ok_or_else(|| overload_error(method, 0))?;
    let term = Term::bind(value, method, arguments.len(), arguments.len() < 3)?;
    let mode = match arguments {
        [_] if term.character => StringComparison::Ordinal,
        [_] => StringComparison::CurrentCulture,
        [_, m] => globalization::comparison(method, m)?,
        [_, ignore, c] => {
            let locale = globalization::method_locale(method, c)?;
            return culture::has_affix(text, &term.text, &locale, !ops::is_true(ignore), prefix)
                .map(Value::Bool);
        }
        _ => return Err(overload_error(method, arguments.len())),
    };
    mode.affix(text, &term.text, prefix).map(Value::Bool)
}
