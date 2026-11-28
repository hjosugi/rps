//! PowerShell chooses between the System.Object, string, Boolean and enum
//! overloads before .NET validates the two UTF-16 ranges.
use super::{
    ambiguous_overload, dotnet_string_argument, globalization, hidden_type,
    method_error::{self, integer, nonnegative},
    overload_error,
};
use rpsh_runtime::{
    PsString, RuntimeError, Value,
    culture::{self, StringComparison},
    ops,
};
use std::ops::Range;
use std::rc::Rc;

fn null_order<T>(left: &Option<T>, right: &Option<T>) -> Option<i32> {
    match (left.is_some(), right.is_some()) {
        (true, true) => None,
        (x, y) => Some(i32::from(x) - i32::from(y)),
    }
}

enum Comparison {
    Mode(StringComparison),
    Culture(String, bool),
    CultureOrdinal(StringComparison),
}

impl Comparison {
    fn bind(options: &[Value], count: usize) -> Result<Self, RuntimeError> {
        match options {
            [] => Ok(Self::Culture(culture::current_locale(), true)),
            [m] if hidden_type(m).as_deref() == Some(globalization::COMPARISON_TYPE) => {
                globalization::comparison("Compare", m).map(Self::Mode)
            }
            [
                Value::Int32(_)
                | Value::Int64(_)
                | Value::UInt32(_)
                | Value::UInt64(_)
                | Value::Int16(_)
                | Value::UInt16(_)
                | Value::SByte(_)
                | Value::Byte(_),
            ] => Err(ambiguous_overload("Compare", count)),
            [ignore] => Ok(Self::Culture(
                culture::current_locale(),
                !ops::is_true(ignore),
            )),
            [locale, flags]
                if hidden_type(locale).as_deref() == Some(globalization::CULTURE_TYPE) =>
            {
                let locale = globalization::method_locale("Compare", locale)?;
                match flags.to_string().to_ascii_lowercase().as_str() {
                    "none" | "0" | "stringsort" | "536870912" => Ok(Self::Culture(locale, true)),
                    "ignorecase" | "1" => Ok(Self::Culture(locale, false)),
                    "ordinal" | "1073741824" => Ok(Self::CultureOrdinal(StringComparison::Ordinal)),
                    "ordinalignorecase" | "268435456" => {
                        Ok(Self::CultureOrdinal(StringComparison::OrdinalIgnoreCase))
                    }
                    _ => Err(RuntimeError::NotImplemented {
                        feature: "this CultureInfo CompareOptions combination",
                    }),
                }
            }
            [ignore, locale] => Ok(Self::Culture(
                globalization::method_locale("Compare", locale)?,
                !ops::is_true(ignore),
            )),
            _ => Err(overload_error("Compare", count)),
        }
    }

    fn evaluate(&self, left: &[u16], right: &[u16], whole: bool) -> Result<i32, RuntimeError> {
        match self {
            Self::Mode(mode) => mode.compare(left, right, whole),
            Self::Culture(locale, sensitive) => {
                culture::compare_locale(left, right, locale, *sensitive)
            }
            Self::CultureOrdinal(mode) => mode.compare(left, right, false),
        }
    }
}

struct Region {
    left: i32,
    right: i32,
    length: i32,
}

impl Region {
    fn bounds(
        &self,
        a: &Option<PsString>,
        b: &Option<PsString>,
        explicit_mode: bool,
    ) -> Result<(Range<usize>, Range<usize>), String> {
        let size = |s: &Option<PsString>| s.as_ref().map_or(0, |s| s.len() as i64);
        let left = i64::from(self.left);
        let right = i64::from(self.right);
        let length = i64::from(self.length);
        let count = |s: &Option<PsString>, offset| {
            if s.is_some() {
                length.min(size(s) - offset)
            } else {
                length
            }
        };
        let len_a = count(a, left);
        let len_b = count(b, right);
        if explicit_mode {
            if length < 0 {
                return Err(nonnegative("length", length));
            }
            let invalid = [
                (left < 0, "indexA"),
                (right < 0, "indexB"),
                (left > size(a), "indexA"),
                (right > size(b), "indexB"),
            ];
            if let Some((_, name)) = invalid.iter().find(|(bad, _)| *bad) {
                return Err(format!(
                    "Index was out of range. Must be non-negative and less than or equal to the size of the collection. (Parameter '{name}')"
                ));
            }
        } else {
            for (name, value) in [
                ("length1", len_a),
                ("length2", len_b),
                ("offset1", left),
                ("offset2", right),
            ] {
                if value < 0 {
                    return Err(nonnegative(name, value));
                }
            }
            for (name, start, len, size) in [
                ("string1", left, len_a, size(a)),
                ("string2", right, len_b, size(b)),
            ] {
                if start + len > size {
                    return Err(format!(
                        "Offset and length must refer to a position in the string. (Parameter '{name}')"
                    ));
                }
            }
        }
        Ok((
            left as usize..(left + len_a) as usize,
            right as usize..(right + len_b) as usize,
        ))
    }
}

pub(super) fn compare(arguments: &[Value], ordinal: bool) -> Result<Value, RuntimeError> {
    let method = if ordinal { "CompareOrdinal" } else { "Compare" };
    if ordinal && !matches!(arguments.len(), 2 | 5) {
        return Err(overload_error(method, arguments.len()));
    }
    let (a, b, region, options) = match arguments {
        [a, b, options @ ..] if options.len() <= 2 => (a, b, None, options),
        [a, left, b, right, length, options @ ..] if options.len() <= 2 => {
            let region = Region {
                left: integer(method, "indexA", left)?,
                right: integer(method, "indexB", right)?,
                length: integer(method, "length", length)?,
            };
            (a, b, Some(region), options)
        }
        _ => return Err(overload_error(method, arguments.len())),
    };
    let comparison = if ordinal {
        Comparison::Mode(StringComparison::Ordinal)
    } else {
        Comparison::bind(options, arguments.len())?
    };
    let a = dotnet_string_argument(a);
    let b = dotnet_string_argument(b);
    let explicit_mode = matches!(comparison, Comparison::Mode(_));
    let nulls = null_order(&a, &b);
    if (explicit_mode || region.is_none())
        && let Some(order) = nulls
    {
        return Ok(Value::Int32(order));
    }
    let ranges = region
        .map(|r| r.bounds(&a, &b, explicit_mode))
        .transpose()
        .map_err(|detail| {
            method_error::invocation(
                "ArgumentOutOfRangeException",
                method,
                arguments.len(),
                &detail,
            )
        })?;
    let (Some(a), Some(b)) = (a, b) else {
        return Ok(Value::Int32(nulls.unwrap_or_default()));
    };
    let a = a.as_utf16();
    let b = b.as_utf16();
    match ranges {
        Some((left, right)) => comparison.evaluate(&a[left], &b[right], false),
        None => comparison.evaluate(&a, &b, true),
    }
    .map(Value::Int32)
}

fn equals_in_mode(
    a: Option<PsString>,
    b: Option<PsString>,
    mode: StringComparison,
) -> Result<Value, RuntimeError> {
    let equal = match (a, b) {
        (Some(a), Some(b)) => mode.compare(&a.as_utf16(), &b.as_utf16(), true)? == 0,
        (None, None) => true,
        _ => false,
    };
    Ok(Value::Bool(equal))
}

fn object_equals(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
        (Value::Object(x), Value::Object(y)) => match hidden_type(a) {
            Some(ty)
                if hidden_type(b).as_ref() == Some(&ty)
                    && matches!(
                        ty.as_str(),
                        globalization::COMPARISON_TYPE
                            | globalization::FORM_TYPE
                            | globalization::CULTURE_TYPE
                            | "System.Management.Automation.Language.NullString"
                            | "System.DateTime"
                            | "System.TimeSpan"
                            | "System.Version"
                            | "System.Guid"
                    ) =>
            {
                a == b
            }
            _ => Rc::ptr_eq(x, y),
        },
        (Value::Double(x), Value::Double(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ if a.is_null_like() => b.is_null_like(),
        _ => a == b,
    }
}

pub(super) fn static_equals(arguments: &[Value]) -> Result<Value, RuntimeError> {
    match arguments {
        [Value::String(a), b] => Ok(Value::Bool(dotnet_string_argument(b).as_ref() == Some(a))),
        [a, b] => Ok(Value::Bool(object_equals(a, b))),
        [a, b, m] => equals_in_mode(
            dotnet_string_argument(a),
            dotnet_string_argument(b),
            globalization::comparison("Equals", m)?,
        ),
        _ => Err(overload_error("Equals", arguments.len())),
    }
}

pub(super) fn instance_equals(text: &PsString, arguments: &[Value]) -> Result<Value, RuntimeError> {
    match arguments {
        [right] => Ok(Value::Bool(matches!(right,Value::String(b) if b==text))),
        [right, m] => equals_in_mode(
            Some(text.clone()),
            dotnet_string_argument(right),
            globalization::comparison("Equals", m)?,
        ),
        _ => Err(overload_error("Equals", arguments.len())),
    }
}

pub(super) fn compare_to(text: &PsString, arguments: &[Value]) -> Result<Value, RuntimeError> {
    match arguments {
        [Value::String(right)] => culture::compare_to(text, right).map(Value::Int32),
        [value] if value.is_null_like() || super::is_null_string(value) => Ok(Value::Int32(1)),
        [_] => Err(method_error::invocation(
            "ArgumentException",
            "CompareTo",
            1,
            "Object must be of type String.",
        )),
        _ => Err(overload_error("CompareTo", arguments.len())),
    }
}
