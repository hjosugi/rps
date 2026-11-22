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
