//! Operators. Upstream: `engine/runtime/Operations` (`NumericOps.cs`,
//! `StringOps.cs`) and the binders in `engine/runtime/Binding/Binders.cs`.

use crate::{RuntimeError, Value};

const INT32_OVERFLOW: &str = "Int32 overflow (promotion to Double)";
const MIXED_OPERANDS: &str = "operators on operand types other than Int32 and String";

fn refuse<T>(feature: &'static str) -> Result<T, RuntimeError> {
    Err(RuntimeError::NotImplemented { feature })
}

/// Binary `+`.
pub fn add(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int32(a), Value::Int32(b)) => match a.checked_add(*b) {
            Some(sum) => Ok(Value::Int32(sum)),
            None => refuse(INT32_OVERFLOW),
        },
        (Value::String(a), Value::String(b)) => {
            let mut joined = String::with_capacity(a.len() + b.len());
            joined.push_str(a);
            joined.push_str(b);
            Ok(Value::String(joined))
        }
        _ => refuse(MIXED_OPERANDS),
    }
}

/// Binary `-`.
pub fn subtract(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int32(a), Value::Int32(b)) => match a.checked_sub(*b) {
            Some(difference) => Ok(Value::Int32(difference)),
            None => refuse(INT32_OVERFLOW),
        },
        _ => refuse(MIXED_OPERANDS),
    }
}

/// Binary `*`.
pub fn multiply(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int32(a), Value::Int32(b)) => match a.checked_mul(*b) {
            Some(product) => Ok(Value::Int32(product)),
            None => refuse(INT32_OVERFLOW),
        },
        _ => refuse(MIXED_OPERANDS),
    }
}

/// Binary `/`: an exact Int32 division stays Int32, otherwise the result is
/// the Double `a / b`, as upstream does.
pub fn divide(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int32(_), Value::Int32(0)) => refuse("divide by zero"),
        (Value::Int32(a), Value::Int32(b)) => {
            if a % b == 0 {
                Ok(Value::Int32(a / b))
            } else {
                Ok(Value::Double(f64::from(*a) / f64::from(*b)))
            }
        }
        _ => refuse(MIXED_OPERANDS),
    }
}

/// Binary `%`: the sign follows the dividend, as in C#.
pub fn remainder(left: &Value, right: &Value) -> Result<Value, RuntimeError> {
    match (left, right) {
        (Value::Int32(_), Value::Int32(0)) => refuse("divide by zero"),
        (Value::Int32(a), Value::Int32(b)) => Ok(Value::Int32(a % b)),
        _ => refuse(MIXED_OPERANDS),
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
        for item in items {
            if compare_scalar(item, right, op, case_sensitive)? {
                kept.push(item.clone());
            }
        }
        return Ok(Value::Array(kept));
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
            let ordering =
                match (left, right) {
                    (Value::Null, Value::Null) => 0,
                    (Value::Null, _) => -1,
                    (_, Value::Null) => 1,
                    (Value::Int32(a), Value::Int32(b)) => a.cmp(b) as i8,
                    (Value::String(a), b) => {
                        let b = b.to_string();
                        let a = if case_sensitive {
                            a.clone()
                        } else {
                            a.to_lowercase()
                        };
                        let b = if case_sensitive { b } else { b.to_lowercase() };
                        a.cmp(&b) as i8
                    }
                    (a, b) => {
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
    let mut ti = 0;
    let mut pi = 0;
    let mut star = None;
    let mut match_at = 0;
    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
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
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
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
    for item in items {
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
        Value::Null => false,
        Value::Bool(value) => *value,
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
        Value::Object(_) | Value::Type(_) | Value::Decimal(_) | Value::BigInteger(_) => true,
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
            Ok(Value::Array(values))
        }
        _ => refuse("range on non-Int32 operands"),
    }
}

/// Binary and unary `-join`.
pub fn join(left: &Value, separator: &Value) -> Result<Value, RuntimeError> {
    let separator = match separator {
        Value::String(text) => text.as_str(),
        Value::Int32(value) => return join(left, &Value::String(value.to_string())),
        _ => return refuse("join separator other than a string"),
    };
    let text = match left {
        Value::Array(items) => items
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(separator),
        other => other.to_string(),
    };
    Ok(Value::String(text))
}

/// Unary `-join`: join an array with no separator.
pub fn unary_join(operand: &Value) -> Result<Value, RuntimeError> {
    join(operand, &Value::String(String::new()))
}

/// Unary `-`.
pub fn negate(operand: &Value) -> Result<Value, RuntimeError> {
    match operand {
        Value::Int32(value) => match value.checked_neg() {
            Some(negated) => Ok(Value::Int32(negated)),
            None => refuse(INT32_OVERFLOW),
        },
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
        Value::String(value.to_owned())
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
    fn overflow_is_refused_rather_than_wrapped() {
        for result in [
            add(&int(i32::MAX), &int(1)),
            subtract(&int(i32::MIN), &int(1)),
            multiply(&int(i32::MAX), &int(2)),
            negate(&int(i32::MIN)),
        ] {
            assert_eq!(result.unwrap_err().feature(), INT32_OVERFLOW);
        }
    }

    #[test]
    fn conversions_between_types_are_refused() {
        // Upstream converts here: `1 + '2'` is 3 and `'1' + 2` is '12'.
        assert_eq!(
            add(&int(1), &string("2")).unwrap_err().feature(),
            MIXED_OPERANDS
        );
        assert_eq!(
            add(&string("1"), &int(2)).unwrap_err().feature(),
            MIXED_OPERANDS
        );
        assert_eq!(
            multiply(&string("ab"), &int(3)).unwrap_err().feature(),
            MIXED_OPERANDS
        );
    }
}
