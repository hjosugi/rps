//! PowerShell's adapter for System.Collections.ArrayList.
use rpsh_runtime::{ArrayList, RuntimeError, Value};
use std::rc::Rc;

use super::{ambiguous_overload, convert_int32, member_enumeration, method_error, overload_error};

const INDEX: &str = "Index was out of range. Must be non-negative and less than the size of the collection. (Parameter 'index')";
const INSERT_INDEX: &str = "Index was out of range. Must be non-negative and less than or equal to the size of the collection. (Parameter 'index')";

fn value_list(values: Vec<Value>) -> Value {
    Value::ArrayList(ArrayList::from_values(values))
}

fn cast_detail(value: &Value, target: &str) -> String {
    match value {
        Value::Bool(_) => format!(
            "Cannot convert value \"{value}\" to type \"{target}\". Error: \"Invalid cast from 'System.Boolean' to '{target}'.\""
        ),
        _ => format!(
            "Cannot convert the \"{value}\" value of type \"{}\" to type \"{target}\".",
            value.type_name()
        ),
    }
}

pub(super) fn cast(value: Value) -> Result<Value, RuntimeError> {
    match value {
        Value::ArrayList(_) | Value::Null => Ok(value),
        Value::AutomationNull => Ok(Value::Null),
        Value::Array(values) | Value::StringArray(values) => Ok(value_list(values.to_vec())),
        other => Err(RuntimeError::Script {
            id: if matches!(other, Value::Bool(_)) {
                "InvalidCastIConvertible"
            } else {
                "ConvertToFinalInvalidCastException"
            },
            category: "InvalidArgument: (:) [], RuntimeException".into(),
            exception_type: "System.Management.Automation.RuntimeException",
            message: cast_detail(&other, ArrayList::TYPE),
        }),
    }
}

fn allocation_error(method: &str, arity: usize) -> RuntimeError {
    method_error::invocation(
        "OutOfMemoryException",
        method,
        arity,
        "Insufficient memory to continue the execution of the program.",
    )
}

fn collection(value: &Value, method: &str, arity: usize) -> Result<Vec<Value>, RuntimeError> {
    if value.is_null_like() {
        return Err(method_error::invocation(
            "ArgumentNullException",
            method,
            arity,
            "Value cannot be null. (Parameter 'c')",
        ));
    }
    value.sequence().map(|items| items.to_vec()).ok_or_else(|| {
        let ty = "System.Collections.ICollection";
        method_error::argument_error(method, "c", ty, &value.to_string(), &cast_detail(value, ty))
    })
}

pub(super) fn construct(arguments: &[Value]) -> Result<Value, RuntimeError> {
    match arguments {
        [] => Ok(value_list(Vec::new())),
        [
            value @ (Value::Array(_)
            | Value::StringArray(_)
            | Value::ArrayList(_)
            | Value::Null
            | Value::AutomationNull),
        ] => Ok(value_list(collection(value, ".ctor", 1)?)),
        [Value::Bool(_)] => Err(ambiguous_overload("new", 1)),
        [Value::Object(_)] => Err(overload_error("new", 1)),
        [capacity] => {
            let capacity = usize::try_from(method_error::integer(".ctor", "capacity", capacity)?)
                .map_err(|_| {
                method_error::invocation(
                    "ArgumentOutOfRangeException",
                    ".ctor",
                    1,
                    "'capacity' must be non-negative. (Parameter 'capacity')",
                )
            })?;
            let mut items = Vec::new();
            items
                .try_reserve_exact(capacity)
                .map_err(|_| allocation_error(".ctor", 1))?;
            Ok(value_list(items))
        }
        _ => Err(overload_error("new", arguments.len())),
    }
}

pub(super) fn property(list: &ArrayList, name: &str) -> Result<Value, RuntimeError> {
    match name.to_ascii_lowercase().as_str() {
        "count" => Ok(Value::Int32(list.items().len() as i32)),
        "capacity" => Ok(Value::Int32(list.items().capacity() as i32)),
        "isreadonly" | "isfixedsize" | "issynchronized" => Ok(Value::Bool(false)),
        "syncroot" => Ok(Value::ArrayList(list.clone())),
        _ => member_enumeration(&list.items(), name),
    }
}

fn checked_index(
    list: &ArrayList,
    at: &Value,
    method: &str,
    arity: usize,
    insert: bool,
) -> Result<usize, RuntimeError> {
    let index = method_error::integer(method, "index", at)?;
    let len = list.items().len();
    usize::try_from(index)
        .ok()
        .filter(|n| *n < len || insert && *n == len)
        .ok_or_else(|| {
            method_error::invocation(
                "ArgumentOutOfRangeException",
                method,
                arity,
                if insert { INSERT_INDEX } else { INDEX },
            )
        })
}

pub(super) fn position(index: i32, len: usize) -> Option<usize> {
    let position = if index < 0 {
        len.checked_sub(index.unsigned_abs() as usize)?
    } else {
        index as usize
    };
    (position < len).then_some(position)
}

pub(super) fn set_index(list: &ArrayList, index: &Value, value: Value) -> Result<(), RuntimeError> {
    let index = convert_int32(index)?;
    let at = position(index, list.items().len()).ok_or_else(|| RuntimeError::Script {
        id: "System.ArgumentOutOfRangeException",
        category: "OperationStopped: (:) [], ArgumentOutOfRangeException".into(),
        exception_type: "System.ArgumentOutOfRangeException",
        message: INDEX.into(),
    })?;
    list.items_mut()[at] = value;
    Ok(())
}

pub(super) fn set_capacity(list: &ArrayList, value: &Value) -> Result<(), RuntimeError> {
    let capacity = convert_int32(value)?;
    let mut items = list.items_mut();
    let requested = usize::try_from(capacity).ok().filter(|n| *n >= items.len()).ok_or_else(|| RuntimeError::Script {
        id: "ExceptionWhenSetting",
        category: "NotSpecified: (:) [], SetValueInvocationException".into(),
        exception_type: "System.Management.Automation.SetValueInvocationException",
        message: "Exception setting \"Capacity\": \"capacity was less than the current size. (Parameter 'value')\"".into(),
    })?;
    if requested != items.capacity() {
        let mut resized = Vec::new();
        resized
            .try_reserve_exact(if requested == 0 { 4 } else { requested })
            .map_err(|_| allocation_error("Capacity", 1))?;
        resized.append(&mut items);
        *items = resized;
    }
    Ok(())
}

// ICollection methods use .NET Object.Equals, including case-sensitive strings
// and reference identity for arrays and ordinary property bags.
fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(x), Value::Array(y)) | (Value::StringArray(x), Value::StringArray(y)) => {
            Rc::ptr_eq(x, y)
        }
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        (Value::Double(x), Value::Double(y)) => x == y || x.is_nan() && y.is_nan(),
        _ => a == b,
    }
}

pub(super) fn method(
    list: &ArrayList,
    name: &str,
    arguments: &[Value],
) -> Result<Value, RuntimeError> {
    let method = name.to_ascii_lowercase();
    let reserve = |count| {
        list.reserve(count)
            .map_err(|_| allocation_error(name, arguments.len()))
    };
    match (method.as_str(), arguments) {
        ("add", [value]) => {
            reserve(1)?;
            let mut items = list.items_mut();
            let index = items.len();
            items.push(value.clone());
            return Ok(Value::Int32(index as i32));
        }
        ("addrange", [value]) | ("insertrange", [_, value]) => {
            let (at, method) = if arguments.len() == 2 {
                (
                    checked_index(list, &arguments[0], "InsertRange", 2, true)?,
                    "InsertRange",
                )
            } else {
                (list.items().len(), "AddRange")
            };
            let values = collection(value, method, arguments.len())?;
            reserve(values.len())?;
            list.items_mut().splice(at..at, values);
        }
        ("clear", []) => list.items_mut().clear(),
        ("clone", []) => return Ok(value_list(list.items().to_vec())),
        ("contains" | "indexof" | "remove", [value]) => {
            let index = list.items().iter().position(|item| equal(item, value));
            match method.as_str() {
                "contains" => return Ok(Value::Bool(index.is_some())),
                "indexof" => return Ok(Value::Int32(index.map_or(-1, |at| at as i32))),
                _ => {
                    if let Some(at) = index {
                        list.items_mut().remove(at);
                    }
                }
            }
        }
        ("equals", [value]) => {
            return Ok(Value::Bool(
                matches!(value, Value::ArrayList(other) if list == other),
            ));
        }
        ("insert", [at, value]) => {
            let at = checked_index(list, at, "Insert", 2, true)?;
            reserve(1)?;
            list.items_mut().insert(at, value.clone());
        }
        ("get_item", [at]) => {
            let at = checked_index(list, at, "get_Item", 1, false)?;
            return Ok(list.items()[at].clone());
        }
        ("set_item", [at, value]) => {
            let at = checked_index(list, at, "set_Item", 2, false)?;
            list.items_mut()[at] = value.clone();
        }
        ("reverse", []) => list.items_mut().reverse(),
        ("trimtosize", []) => {
            let count = Value::Int32(list.items().len() as i32);
            set_capacity(list, &count)?;
        }
        ("removeat", [at]) => {
            let at = checked_index(list, at, "RemoveAt", 1, false)?;
            list.items_mut().remove(at);
        }
        ("toarray", []) => return Ok(Value::array(list.items().to_vec())),
        ("tostring", []) => return Ok(Value::String(ArrayList::TYPE.into())),
        (
            "binarysearch" | "copyto" | "getenumerator" | "getrange" | "lastindexof"
            | "removerange" | "repeat" | "setrange" | "sort" | "synchronized" | "fixedsize"
            | "readonly" | "adapter",
            _,
        )
        | ("toarray", [_])
        | ("indexof", [_, _] | [_, _, _])
        | ("reverse", [_, _]) => {
            return Err(RuntimeError::NotImplemented {
                feature: "this ArrayList method overload",
            });
        }
        (
            "add" | "addrange" | "insertrange" | "clear" | "clone" | "contains" | "indexof"
            | "remove" | "equals" | "insert" | "removeat" | "toarray" | "tostring" | "reverse"
            | "trimtosize" | "get_item" | "set_item",
            _,
        ) => return Err(overload_error(name, arguments.len())),
        _ => {
            return Err(RuntimeError::NotImplemented {
                feature: "this ArrayList method",
            });
        }
    }
    Ok(Value::AutomationNull)
}
