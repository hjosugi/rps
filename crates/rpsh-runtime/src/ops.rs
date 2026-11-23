//! Operators. Upstream: `engine/runtime/Operations` (`NumericOps.cs`,
//! `StringOps.cs`) and the binders in `engine/runtime/Binding/Binders.cs`.

use std::rc::Rc;

use crate::{PsString, RuntimeError, Value};

const MIXED_OPERANDS: &str = "operators on operand types other than numbers and strings";

fn refuse<T>(feature: &'static str) -> Result<T, RuntimeError> {
    Err(RuntimeError::NotImplemented { feature })
}

/// The integer types that share a result kind. Anything narrower than `Int32`
/// promotes to `Int32`, as upstream's binder does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum IntKind {
    I32,
    I64,
    U32,
    U64,
}

#[derive(Clone, Copy)]
enum NumericKind {
    Int(IntKind),
    Double,
}

fn int_kind(value: &Value) -> Option<IntKind> {
    match value {
        Value::Int32(_) | Value::Int16(_) | Value::UInt16(_) | Value::Byte(_) | Value::SByte(_) => {
            Some(IntKind::I32)
        }
        Value::UInt32(_) => Some(IntKind::U32),
        Value::Int64(_) => Some(IntKind::I64),
        Value::UInt64(_) => Some(IntKind::U64),
        _ => None,
    }
}

fn int_value(value: &Value) -> Option<i128> {
    match value {
        Value::Int32(value) => Some(i128::from(*value)),
        Value::Int64(value) => Some(i128::from(*value)),
        Value::UInt32(value) => Some(i128::from(*value)),
        Value::UInt64(value) => Some(i128::from(*value)),
        Value::Int16(value) => Some(i128::from(*value)),
        Value::UInt16(value) => Some(i128::from(*value)),
        Value::Byte(value) => Some(i128::from(*value)),
        Value::SByte(value) => Some(i128::from(*value)),
        _ => None,
    }
}

fn float_value(value: &Value) -> Option<f64> {
    match value {
        Value::Double(value) => Some(*value),
        other => int_value(other).map(|value| value as f64),
    }
}

/// The result kind of a binary arithmetic operator. A negative value mixed
/// with an unsigned 64-bit operand is `Decimal` upstream; that is refused
/// until `Decimal` arithmetic is implemented.
fn numeric_kind(left: &Value, right: &Value) -> Result<NumericKind, RuntimeError> {
    if matches!(left, Value::Double(_)) || matches!(right, Value::Double(_)) {
        return Ok(NumericKind::Double);
    }
    let (Some(left_kind), Some(right_kind)) = (int_kind(left), int_kind(right)) else {
        return refuse(MIXED_OPERANDS);
    };
    let left_value = int_value(left).unwrap_or_default();
    let right_value = int_value(right).unwrap_or_default();
    Ok(NumericKind::Int(
        if left_kind == IntKind::U64 || right_kind == IntKind::U64 {
            if left_value >= 0 && right_value >= 0 {
                IntKind::U64
            } else {
                return refuse("UInt64 mixed with a negative value (Decimal)");
            }
        } else if left_kind == IntKind::I64 || right_kind == IntKind::I64 {
            IntKind::I64
        } else if left_kind == IntKind::U32 || right_kind == IntKind::U32 {
            if left_value >= 0 && right_value >= 0 {
                IntKind::U32
            } else {
                IntKind::I64
            }
        } else {
            IntKind::I32
        },
    ))
}

/// The value of an integer result, promoting to `Double` when it does not fit
/// the type the operands chose.
fn integer_result(kind: IntKind, value: i128) -> Value {
    match kind {
        IntKind::I32 => {
            i32::try_from(value).map_or_else(|_| Value::Double(value as f64), Value::Int32)
        }
        IntKind::I64 => {
            i64::try_from(value).map_or_else(|_| Value::Double(value as f64), Value::Int64)
        }
        IntKind::U32 => {
            u32::try_from(value).map_or_else(|_| Value::Double(value as f64), Value::UInt32)
        }
        IntKind::U64 => {
            u64::try_from(value).map_or_else(|_| Value::Double(value as f64), Value::UInt64)
        }
    }
}

fn double_operands(left: &Value, right: &Value, operation: impl FnOnce(f64, f64) -> f64) -> Value {
    Value::Double(operation(
        float_value(left).unwrap_or_default(),
        float_value(right).unwrap_or_default(),
    ))
}

/// A numeric operand, converting a string the way `LanguagePrimitives` does
/// for arithmetic: `1 + '2'` is `3`.
fn numeric_of(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::AutomationNull => Some(Value::Int32(0)),
        Value::String(text) => parse_number_text(&text.to_string_lossy()),
        Value::Int32(_)
        | Value::Int64(_)
        | Value::UInt32(_)
        | Value::UInt64(_)
        | Value::Int16(_)
        | Value::UInt16(_)
        | Value::Byte(_)
        | Value::SByte(_)
        | Value::Double(_) => Some(value.clone()),
        _ => None,
    }
}

fn parse_number_text(text: &str) -> Option<Value> {
    let text = text.trim();
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        let value = i128::from_str_radix(hex, 16).ok()?;
        return integer_literal(if negative { -value } else { value });
    }
    if let Some(binary) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
        let value = i128::from_str_radix(binary, 2).ok()?;
        return integer_literal(if negative { -value } else { value });
    }
    let signed = if negative {
        format!("-{body}")
    } else {
        body.to_owned()
    };
    if let Ok(value) = signed.parse::<i32>() {
        Some(Value::Int32(value))
    } else if let Ok(value) = signed.parse::<i64>() {
        Some(Value::Int64(value))
    } else {
        signed.parse::<f64>().ok().map(Value::Double)
    }
}

fn integer_literal(value: i128) -> Option<Value> {
    if let Ok(value) = i32::try_from(value) {
        Some(Value::Int32(value))
    } else {
        i64::try_from(value).ok().map(Value::Int64)
    }
}

fn numeric_pair(left: &Value, right: &Value) -> Result<(Value, Value), RuntimeError> {
    match (numeric_of(left), numeric_of(right)) {
        (Some(left), Some(right)) => Ok((left, right)),
        _ => refuse(MIXED_OPERANDS),
    }
}

/// Binary `+`. A `Char` is a string in addition upstream, and a string on the
/// left concatenates: `'1' + 2` is `"12"`.
pub fn add(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    if matches!(left, Value::Null | Value::AutomationNull)
        && matches!(right, Value::Null | Value::AutomationNull)
    {
        return Ok(Value::Null);
    }
    if matches!(left, Value::Null | Value::AutomationNull)
        && let Value::String(text) = right
    {
        return Ok(Value::String(text.clone()));
    }
    if let Value::Array(items) = left {
        let mut result = items.as_ref().clone();
        match right {
            Value::Array(more) => result.extend(more.iter().cloned()),
            value => result.push(value.clone()),
        }
        return Ok(Value::array(result));
    }
    if matches!(left, Value::Char(_)) || matches!(right, Value::Char(_)) {
        return Ok(Value::String(PsString::from(format!("{left}{right}"))));
    }
    if let Value::String(text) = left {
        return Ok(Value::String(PsString::from(format!("{text}{right}"))));
    }
    let (left, right) = numeric_pair(left, right)?;
    match numeric_kind(&left, &right)? {
        NumericKind::Double => Ok(double_operands(&left, &right, |a, b| a + b)),
        NumericKind::Int(kind) => Ok(integer_result(
            kind,
            int_value(&left).unwrap_or_default() + int_value(&right).unwrap_or_default(),
        )),
    }
}

/// Binary `-`.
pub fn subtract(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(left, right)?;
    match numeric_kind(&left, &right)? {
        NumericKind::Double => Ok(double_operands(&left, &right, |a, b| a - b)),
        NumericKind::Int(kind) => Ok(integer_result(
            kind,
            int_value(&left).unwrap_or_default() - int_value(&right).unwrap_or_default(),
        )),
    }
}

/// Binary `*`. A string on the left repeats when the right is an `Int32`:
/// `'ab' * 3` is `"ababab"`.
pub fn multiply(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    if let (Value::Array(items), Value::Int32(times)) = (left, right) {
        let times = usize::try_from(*times).map_err(|_| RuntimeError::NotImplemented {
            feature: "negative array repetition",
        })?;
        let mut result = Vec::with_capacity(items.len() * times);
        for _ in 0..times {
            result.extend(items.iter().cloned());
        }
        return Ok(Value::array(result));
    }
    if let (Value::String(text), Value::Int32(times)) = (left, right) {
        let times = usize::try_from(*times).map_err(|_| RuntimeError::NotImplemented {
            feature: "negative string repetition",
        })?;
        return Ok(Value::String(PsString::from(
            text.to_string_lossy().repeat(times),
        )));
    }
    let (left, right) = numeric_pair(left, right)?;
    match numeric_kind(&left, &right)? {
        NumericKind::Double => Ok(double_operands(&left, &right, |a, b| a * b)),
        NumericKind::Int(kind) => {
            let left_value = int_value(&left).unwrap_or_default();
            let right_value = int_value(&right).unwrap_or_default();
            match left_value.checked_mul(right_value) {
                Some(product) => Ok(integer_result(kind, product)),
                None => Ok(Value::Double(left_value as f64 * right_value as f64)),
            }
        }
    }
}

/// Binary `/`: an exact integer division stays integer, otherwise the result
/// is `Double`, as upstream does.
pub fn divide(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(left, right)?;
    match numeric_kind(&left, &right)? {
        NumericKind::Double => Ok(double_operands(&left, &right, |a, b| a / b)),
        NumericKind::Int(kind) => {
            let left_value = int_value(&left).unwrap_or_default();
            let right_value = int_value(&right).unwrap_or_default();
            if right_value == 0 {
                return refuse("divide by zero");
            }
            if left_value % right_value == 0 {
                Ok(integer_result(kind, left_value / right_value))
            } else {
                Ok(Value::Double(left_value as f64 / right_value as f64))
            }
        }
    }
}

/// Binary `%`: the sign follows the dividend, as in C#.
pub fn remainder(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    let (left, right) = numeric_pair(left, right)?;
    match numeric_kind(&left, &right)? {
        NumericKind::Double => Ok(double_operands(&left, &right, |a, b| a % b)),
        NumericKind::Int(kind) => {
            let left_value = int_value(&left).unwrap_or_default();
            let right_value = int_value(&right).unwrap_or_default();
            if right_value == 0 {
                return refuse("divide by zero");
            }
            Ok(integer_result(kind, left_value % right_value))
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Comparison {
    Eq,
    Ne,
    Ge,
    Gt,
    Lt,
    Le,
    Like,
    NotLike,
}

/// Comparison operators, including the array-filtering form.
pub fn compare(
    left: &Value,
    right: &Value,
    op: Comparison,
    case_sensitive: bool,
) -> Result<Value, RuntimeError> {
    if let Value::Array(items) = left {
        let mut kept = Vec::new();
        for item in items.iter() {
            if compare_scalar(item, right, op, case_sensitive)? {
                kept.push(item.clone());
            }
        }
        return Ok(Value::array(kept));
    }
    Ok(Value::Bool(compare_scalar(
        left,
        right,
        op,
        case_sensitive,
    )?))
}

fn compare_scalar(
    left: &Value,
    right: &Value,
    op: Comparison,
    case_sensitive: bool,
) -> Result<bool, RuntimeError> {
    match op {
        Comparison::Like | Comparison::NotLike => {
            let text = left.to_string();
            let pattern = right.to_string();
            let matched = wildcard(&text, &pattern, case_sensitive);
            Ok(if matches!(op, Comparison::NotLike) {
                !matched
            } else {
                matched
            })
        }
        _ => {
            if let (Value::Object(a), Value::Object(b)) = (left, right) {
                return match op {
                    Comparison::Eq => Ok(Rc::ptr_eq(a, b)),
                    Comparison::Ne => Ok(!Rc::ptr_eq(a, b)),
                    _ => refuse("ordering comparison on objects"),
                };
            }
            if let Value::Char(unit) = left {
                let left = Value::String(PsString::from_units(vec![*unit]));
                return compare_scalar(&left, right, op, case_sensitive);
            }
            if let Value::Char(unit) = right {
                let right = Value::String(PsString::from_units(vec![*unit]));
                return compare_scalar(left, &right, op, case_sensitive);
            }
            let ordering =
                match (left, right) {
                    (Value::Null | Value::AutomationNull, Value::Null | Value::AutomationNull) => 0,
                    (Value::Null | Value::AutomationNull, _) => -1,
                    (_, Value::Null | Value::AutomationNull) => 1,
                    (Value::Int32(a), Value::Int32(b)) => a.cmp(b) as i8,
                    (Value::String(a), b) => {
                        let b = match b {
                            Value::String(b) => b.clone(),
                            other => PsString::from(other.to_string()),
                        };
                        compare_strings(a, &b, case_sensitive)?
                    }
                    (a, b) => {
                        if let (Some(a), Some(b)) = (int_value(a), int_value(b)) {
                            return Ok(match op {
                                Comparison::Eq => a == b,
                                Comparison::Ne => a != b,
                                Comparison::Ge => a >= b,
                                Comparison::Gt => a > b,
                                Comparison::Lt => a < b,
                                Comparison::Le => a <= b,
                                Comparison::Like | Comparison::NotLike => unreachable!(),
                            });
                        }
                        let a = a.to_string().parse::<f64>().map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "comparison on these operand types",
                            }
                        })?;
                        let b = b.to_string().parse::<f64>().map_err(|_| {
                            RuntimeError::NotImplemented {
                                feature: "comparison on these operand types",
                            }
                        })?;
                        a.partial_cmp(&b)
                            .map(|ordering| match ordering {
                                std::cmp::Ordering::Less => -1,
                                std::cmp::Ordering::Equal => 0,
                                std::cmp::Ordering::Greater => 1,
                            })
                            .ok_or(RuntimeError::NotImplemented {
                                feature: "comparison on NaN",
                            })?
                    }
                };
            Ok(match op {
                Comparison::Eq => ordering == 0,
                Comparison::Ne => ordering != 0,
                Comparison::Ge => ordering >= 0,
                Comparison::Gt => ordering > 0,
                Comparison::Lt => ordering < 0,
                Comparison::Le => ordering <= 0,
                Comparison::Like | Comparison::NotLike => unreachable!(),
            })
        }
    }
}

/// Ordinal comparison for `-ceq` and friends. Case-insensitive comparison is
/// ASCII for now: culture-aware folding (`'ﬀ' -eq 'ff'`) needs ICU, which is
/// its own dependency decision, so anything else is refused rather than
/// approximated.
fn compare_strings(
    left: &PsString,
    right: &PsString,
    case_sensitive: bool,
) -> Result<i8, RuntimeError> {
    if case_sensitive {
        return Ok(left.units().cmp(right.units()) as i8);
    }
    match (left.as_ascii(), right.as_ascii()) {
        (Some(left), Some(right)) => {
            Ok(left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()) as i8)
        }
        _ => refuse("culture-aware string comparison (ICU)"),
    }
}

#[derive(Debug, Clone)]
enum WildcardAtom {
    Star,
    Any,
    Char(char),
    Class(Vec<(char, char)>),
}

/// Public `-like` match, used by `switch -Wildcard` too.
pub fn wildcard_match(text: &str, pattern: &str, case_sensitive: bool) -> bool {
    wildcard(text, pattern, case_sensitive)
}

/// `.NET`'s wildcard matching: `*`, `?`, `[abc]`, `[a-z]` and a backtick that
/// escapes the next character. A malformed class (`[` with no `]`) is literal.
fn wildcard(text: &str, pattern: &str, case_sensitive: bool) -> bool {
    let text: Vec<char> = if case_sensitive {
        text.chars().collect()
    } else {
        text.to_lowercase().chars().collect()
    };
    let pattern: Vec<char> = if case_sensitive {
        pattern.chars().collect()
    } else {
        pattern.to_lowercase().chars().collect()
    };
    let atoms = wildcard_atoms(&pattern);
    let mut ti = 0;
    let mut pi = 0;
    let mut star = None;
    let mut match_at = 0;
    while ti < text.len() {
        let matches = pi < atoms.len()
            && match &atoms[pi] {
                WildcardAtom::Star => false,
                WildcardAtom::Any => true,
                WildcardAtom::Char(ch) => *ch == text[ti],
                WildcardAtom::Class(ranges) => ranges
                    .iter()
                    .any(|(low, high)| (*low..=*high).contains(&text[ti])),
            };
        if matches {
            ti += 1;
            pi += 1;
        } else if pi < atoms.len() && matches!(&atoms[pi], WildcardAtom::Star) {
            star = Some(pi);
            match_at = ti;
            pi += 1;
        } else if let Some(star) = star {
            pi = star + 1;
            match_at += 1;
            ti = match_at;
        } else {
            return false;
        }
    }
    while pi < atoms.len() && matches!(&atoms[pi], WildcardAtom::Star) {
        pi += 1;
    }
    pi == atoms.len()
}

fn wildcard_atoms(pattern: &[char]) -> Vec<WildcardAtom> {
    let mut atoms = Vec::new();
    let mut at = 0;
    while at < pattern.len() {
        match pattern[at] {
            '*' => {
                atoms.push(WildcardAtom::Star);
                at += 1;
            }
            '?' => {
                atoms.push(WildcardAtom::Any);
                at += 1;
            }
            '`' => {
                if let Some(ch) = pattern.get(at + 1) {
                    atoms.push(WildcardAtom::Char(*ch));
                    at += 2;
                } else {
                    atoms.push(WildcardAtom::Char('`'));
                    at += 1;
                }
            }
            '[' => {
                let Some(end) = pattern[at + 1..].iter().position(|ch| *ch == ']') else {
                    atoms.push(WildcardAtom::Char('['));
                    at += 1;
                    continue;
                };
                let inner = &pattern[at + 1..at + 1 + end];
                let mut ranges = Vec::new();
                let mut index = 0;
                while index < inner.len() {
                    if inner.get(index + 1) == Some(&'-')
                        && let Some(high) = inner.get(index + 2)
                    {
                        ranges.push((inner[index], *high));
                        index += 3;
                    } else {
                        ranges.push((inner[index], inner[index]));
                        index += 1;
                    }
                }
                atoms.push(WildcardAtom::Class(ranges));
                at = at + 1 + end + 1;
            }
            ch => {
                atoms.push(WildcardAtom::Char(ch));
                at += 1;
            }
        }
    }
    atoms
}

/// `-contains` / `-in`.
pub fn contains(
    collection: &Value,
    needle: &Value,
    case_sensitive: bool,
) -> Result<Value, RuntimeError> {
    let Value::Array(items) = collection else {
        return Err(RuntimeError::NotImplemented {
            feature: "contains on a non-array",
        });
    };
    for item in items.iter() {
        if compare_scalar(item, needle, Comparison::Eq, case_sensitive)? {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

/// `-shl` / `-shr`.
pub fn shift(left: &Value, right: &Value, left_shift: bool) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int32(a), Value::Int32(b)) => {
            let amount = *b as u32;
            Ok(Value::Int32(if left_shift {
                a.wrapping_shl(amount)
            } else {
                a.wrapping_shr(amount)
            }))
        }
        _ => refuse("shift on non-Int32 operands"),
    }
}

/// Logical operators.
pub fn logical(left: &Value, right: &Value, op: &str) -> Result<Value, RuntimeError> {
    let l = is_true(left);
    let r = is_true(right);
    Ok(Value::Bool(match op {
        "and" => l && r,
        "or" => l || r,
        "xor" => l ^ r,
        _ => unreachable!(),
    }))
}

/// Bitwise operators.
pub fn bitwise(left: &Value, right: &Value, op: &str) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int32(a), Value::Int32(b)) => Ok(Value::Int32(match op {
            "band" => a & b,
            "bor" => a | b,
            "bxor" => a ^ b,
            _ => unreachable!(),
        })),
        _ => refuse("bitwise operators on non-Int32 operands"),
    }
}

pub fn not(operand: &Value) -> Value {
    Value::Bool(!is_true(operand))
}

pub fn bnot(operand: &Value) -> Result<Value, RuntimeError> {
    match operand {
        Value::Int32(value) => Ok(Value::Int32(!value)),
        _ => refuse("bitwise not on non-Int32 operands"),
    }
}

pub fn is_true(value: &Value) -> bool {
    match value {
        Value::Null | Value::AutomationNull => false,
        Value::Bool(value) => *value,
        Value::Char(unit) => *unit != 0,
        Value::Int32(value) => *value != 0,
        Value::Int64(value) => *value != 0,
        Value::UInt32(value) => *value != 0,
        Value::UInt64(value) => *value != 0,
        Value::Int16(value) => *value != 0,
        Value::UInt16(value) => *value != 0,
        Value::Byte(value) => *value != 0,
        Value::SByte(value) => *value != 0,
        Value::Double(value) => *value != 0.0,
        Value::String(value) => !value.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(_)
        | Value::Type(_)
        | Value::Decimal(_)
        | Value::BigInteger(_)
        | Value::ScriptBlock(_) => true,
    }
}

/// The range operator `a..b`.
pub fn range(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int32(a), Value::Int32(b)) => {
            let mut values = Vec::new();
            if a <= b {
                for value in *a..=*b {
                    values.push(Value::Int32(value));
                }
            } else {
                for value in (*b..=*a).rev() {
                    values.push(Value::Int32(value));
                }
            }
            Ok(Value::array(values))
        }
        _ => refuse("range on non-Int32 operands"),
    }
}

/// Binary and unary `-join`.
pub fn join(left: &Value, separator: &Value) -> Result<Value, RuntimeError> {
    let separator = match separator {
        Value::String(text) => text.to_string_lossy(),
        Value::Int32(value) => return join(left, &Value::String(value.to_string().into())),
        _ => return refuse("join separator other than a string"),
    };
    let text = match left {
        Value::Array(items) => items
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(&separator),
        other => other.to_string(),
    };
    Ok(Value::String(text.into()))
}

/// The `-f` format operator: `{index[:spec]}` placeholders and `{{`/`}}`.
pub fn format(template: &Value, arguments: &Value) -> Result<Value, RuntimeError> {
    let Value::String(text) = template else {
        return refuse("format string is not a string");
    };
    let args: Vec<&Value> = match arguments {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let chars: Vec<char> = text.to_string_lossy().chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        match chars[at] {
            '{' => {
                if chars.get(at + 1) == Some(&'{') {
                    out.push('{');
                    at += 2;
                    continue;
                }
                at += 1;
                let start = at;
                while at < chars.len() && chars[at].is_ascii_digit() {
                    at += 1;
                }
                let index = chars[start..at].iter().collect::<String>();
                let mut spec = String::new();
                if chars.get(at) == Some(&':') {
                    at += 1;
                    let spec_start = at;
                    while at < chars.len() && chars[at] != '}' {
                        at += 1;
                    }
                    spec = chars[spec_start..at].iter().collect();
                }
                if chars.get(at) != Some(&'}') {
                    return refuse("malformed format string");
                }
                at += 1;
                let index: usize = index.parse().map_err(|_| RuntimeError::NotImplemented {
                    feature: "malformed format string",
                })?;
                let argument = args.get(index).ok_or(RuntimeError::NotImplemented {
                    feature: "format index out of range",
                })?;
                out.push_str(&format_one(argument, &spec)?);
            }
            '}' => {
                if chars.get(at + 1) == Some(&'}') {
                    out.push('}');
                    at += 2;
                } else {
                    return refuse("malformed format string");
                }
            }
            other => {
                out.push(other);
                at += 1;
            }
        }
    }
    Ok(Value::String(out.into()))
}

fn format_one(value: &Value, spec: &str) -> Result<String, RuntimeError> {
    if spec.is_empty() {
        return Ok(value.to_string());
    }
    let kind = spec.chars().next().unwrap_or(' ').to_ascii_uppercase();
    let precision = &spec[1..];
    let precision: Option<usize> = if precision.is_empty() {
        None
    } else {
        Some(
            precision
                .parse()
                .map_err(|_| RuntimeError::NotImplemented {
                    feature: "this format specifier",
                })?,
        )
    };
    match kind {
        'N' => {
            let value = float_value(value).ok_or(RuntimeError::NotImplemented {
                feature: "this format specifier",
            })?;
            let precision = precision.unwrap_or(2);
            Ok(group_thousands(&format!("{value:.precision$}"), precision))
        }
        'F' => {
            let value = float_value(value).ok_or(RuntimeError::NotImplemented {
                feature: "this format specifier",
            })?;
            let precision = precision.unwrap_or(2);
            Ok(format!("{value:.precision$}"))
        }
        'D' => {
            let value = int_value(value).ok_or(RuntimeError::NotImplemented {
                feature: "this format specifier",
            })?;
            Ok(format!("{value:0width$}", width = precision.unwrap_or(0)))
        }
        'C' => {
            let value = float_value(value).ok_or(RuntimeError::NotImplemented {
                feature: "this format specifier",
            })?;
            Ok(format!("${}", group_thousands(&format!("{value:.2}"), 2)))
        }
        _ => refuse("this format specifier"),
    }
}

/// Inserts thousands separators into the integer part of `text`, which has
/// `decimals` fractional digits.
fn group_thousands(text: &str, decimals: usize) -> String {
    let (sign, text) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text),
    };
    let (integer, fraction) = if decimals > 0 {
        text.split_once('.').unwrap_or((text, ""))
    } else {
        (text, "")
    };
    let mut grouped = String::new();
    for (index, ch) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    if decimals > 0 {
        format!("{sign}{grouped}.{fraction}")
    } else {
        format!("{sign}{grouped}")
    }
}

/// Unary `-join`: join an array with no separator.
pub fn unary_join(operand: &Value) -> Result<Value, RuntimeError> {
    join(operand, &Value::String(Default::default()))
}

/// Unary `-`.
pub fn negate(operand: &Value) -> Result<Value, RuntimeError> {
    match operand {
        Value::Double(value) => Ok(Value::Double(-value)),
        Value::Int32(value) => Ok(value
            .checked_neg()
            .map_or_else(|| Value::Double(-f64::from(*value)), Value::Int32)),
        Value::Int64(value) => Ok(value
            .checked_neg()
            .map_or_else(|| Value::Double(-(*value as f64)), Value::Int64)),
        // Unsigned negation becomes Double upstream.
        Value::UInt32(value) => Ok(Value::Double(-f64::from(*value))),
        Value::UInt64(value) => Ok(Value::Double(-(*value as f64))),
        // Anything narrower than Int32 negates as Int32.
        Value::Int16(_) | Value::UInt16(_) | Value::Byte(_) | Value::SByte(_) => Ok(Value::Int32(
            -(int_value(operand).unwrap_or_default() as i32),
        )),
        _ => refuse(MIXED_OPERANDS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(value: i32) -> Value {
        Value::Int32(value)
    }

    fn string(value: &str) -> Value {
        Value::String(value.into())
    }

    #[test]
    fn int32_arithmetic() {
        assert_eq!(add(&int(2), &int(3)), Ok(int(5)));
        assert_eq!(subtract(&int(2), &int(3)), Ok(int(-1)));
        assert_eq!(multiply(&int(-4), &int(3)), Ok(int(-12)));
        assert_eq!(negate(&int(7)), Ok(int(-7)));
        assert_eq!(divide(&int(6), &int(2)), Ok(int(3)));
        assert_eq!(divide(&int(7), &int(2)), Ok(Value::Double(3.5)));
        assert_eq!(remainder(&int(7), &int(3)), Ok(int(1)));
        assert_eq!(remainder(&int(-7), &int(3)), Ok(int(-1)));
    }

    #[test]
    fn string_concatenation() {
        assert_eq!(add(&string("ab"), &string("cd")), Ok(string("abcd")));
    }

    #[test]
    fn ascii_string_comparison_folds_case() {
        assert_eq!(
            compare(&string("a"), &string("A"), Comparison::Eq, false),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            compare(&string("a"), &string("A"), Comparison::Eq, true),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn culture_aware_comparison_is_refused_until_icu() {
        let result = compare(&string("é"), &string("É"), Comparison::Eq, false);
        assert_eq!(
            result.unwrap_err().feature(),
            "culture-aware string comparison (ICU)"
        );
        // Case-sensitive comparison is ordinal and exact in both forms.
        assert_eq!(
            compare(&string("é"), &string("É"), Comparison::Eq, true),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn chars_compare_as_strings_and_keep_surrogates() {
        assert_eq!(
            compare(
                &Value::Char(0x61),
                &Value::Char(0x41),
                Comparison::Eq,
                false
            ),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            compare(
                &Value::Char(0xD83D),
                &Value::Char(0xDE00),
                Comparison::Eq,
                true
            ),
            Ok(Value::Bool(false))
        );
        assert_eq!(
            compare(
                &Value::Char(0xD83D),
                &Value::Char(0xD83D),
                Comparison::Eq,
                true
            ),
            Ok(Value::Bool(true))
        );
        assert!(!is_true(&Value::Char(0)));
        assert!(is_true(&Value::Char(0x61)));
    }

    #[test]
    fn int32_overflow_promotes_to_double() {
        assert_eq!(
            add(&int(i32::MAX), &int(1)),
            Ok(Value::Double(2_147_483_648.0))
        );
        assert_eq!(
            subtract(&int(i32::MIN), &int(1)),
            Ok(Value::Double(-2_147_483_649.0))
        );
        assert_eq!(
            multiply(&int(i32::MAX), &int(2)),
            Ok(Value::Double(4_294_967_294.0))
        );
        assert_eq!(negate(&int(i32::MIN)), Ok(Value::Double(2_147_483_648.0)));
    }

    #[test]
    fn wider_integers_keep_their_type() {
        assert_eq!(
            add(&Value::Int64(2_147_483_648), &int(1)),
            Ok(Value::Int64(2_147_483_649))
        );
        assert_eq!(add(&Value::UInt32(1), &int(1)), Ok(Value::UInt32(2)));
        assert_eq!(add(&Value::UInt64(1), &int(1)), Ok(Value::UInt64(2)));
        assert_eq!(add(&Value::Byte(1), &int(1)), Ok(int(2)));
        assert_eq!(add(&Value::Int16(1), &Value::Int16(1)), Ok(int(2)));
    }

    #[test]
    fn mixing_a_negative_with_uint32_widens_to_int64() {
        assert_eq!(add(&Value::UInt32(1), &int(-3)), Ok(Value::Int64(-2)));
    }

    #[test]
    fn double_wins_over_integers() {
        assert_eq!(add(&int(1), &Value::Double(2.5)), Ok(Value::Double(3.5)));
        assert_eq!(
            multiply(&Value::Double(1.5), &int(2)),
            Ok(Value::Double(3.0))
        );
        assert_eq!(
            remainder(&int(5), &Value::Double(2.0)),
            Ok(Value::Double(1.0))
        );
    }

    #[test]
    fn integer_division_is_exact_or_double() {
        assert_eq!(divide(&int(6), &int(2)), Ok(int(3)));
        assert_eq!(divide(&int(7), &int(2)), Ok(Value::Double(3.5)));
        assert_eq!(divide(&Value::Int64(6), &int(2)), Ok(Value::Int64(3)));
        assert_eq!(divide(&Value::UInt32(7), &int(2)), Ok(Value::Double(3.5)));
    }

    #[test]
    fn format_operator_substitutes_and_formats() {
        assert_eq!(
            format(&string("{0}-{1}"), &Value::array(vec![int(1), int(2)])),
            Ok(string("1-2"))
        );
        assert_eq!(
            format(&string("{0:N2}"), &Value::Double(1234567.891)),
            Ok(string("1,234,567.89"))
        );
        assert_eq!(format(&string("{{x}}"), &int(1)), Ok(string("{x}")));
    }

    #[test]
    fn wildcards_match_classes_and_escapes() {
        assert!(wildcard("abc", "a?c", true));
        assert!(wildcard("abc", "a[bx]c", true));
        assert!(wildcard("abc", "a[a-c]c", true));
        assert!(!wildcard("abc", "a[d]c", true));
        assert!(wildcard("ABC", "a*", false));
        assert!(!wildcard("ABC", "a*", true));
        assert!(wildcard("a*c", "a`*c", true));
        assert!(wildcard("a[bc]", "a`[bc`]", true));
    }

    #[test]
    fn arrays_concatenate_and_repeat() {
        let array = Value::array(vec![int(1), int(2)]);
        assert_eq!(
            add(&array, &int(3)),
            Ok(Value::array(vec![int(1), int(2), int(3)]))
        );
        assert_eq!(
            add(&array, &Value::array(vec![int(3)])),
            Ok(Value::array(vec![int(1), int(2), int(3)]))
        );
        assert_eq!(
            multiply(&array, &int(2)),
            Ok(Value::array(vec![int(1), int(2), int(1), int(2)]))
        );
        // The left operand decides: a scalar does not add an array.
        assert_eq!(add(&int(3), &array).unwrap_err().feature(), MIXED_OPERANDS);
    }

    #[test]
    fn null_behaves_as_zero_in_arithmetic() {
        assert_eq!(add(&Value::Null, &int(1)), Ok(int(1)));
        assert_eq!(subtract(&int(1), &Value::Null), Ok(int(1)));
        assert_eq!(add(&Value::Null, &Value::Null), Ok(Value::Null));
        assert_eq!(multiply(&Value::Null, &int(2)), Ok(int(0)));
    }

    #[test]
    fn char_addition_is_string_concatenation() {
        assert_eq!(
            add(&Value::Char(0x61), &int(1)),
            Ok(Value::String("a1".into()))
        );
        assert_eq!(
            add(&int(1), &Value::Char(0x61)),
            Ok(Value::String("1a".into()))
        );
    }

    #[test]
    fn arithmetic_coerces_strings_by_the_left_operand() {
        assert_eq!(add(&int(1), &string("2")), Ok(int(3)));
        assert_eq!(add(&string("1"), &int(2)), Ok(string("12")));
        assert_eq!(add(&int(1), &string("2.5")), Ok(Value::Double(3.5)));
        assert_eq!(multiply(&string("ab"), &int(3)), Ok(string("ababab")));
        assert_eq!(multiply(&int(2), &string("3")), Ok(int(6)));
        assert_eq!(subtract(&string("10"), &int(2)), Ok(int(8)));
        assert_eq!(
            add(&int(1), &string("ab")).unwrap_err().feature(),
            MIXED_OPERANDS
        );
    }
}
