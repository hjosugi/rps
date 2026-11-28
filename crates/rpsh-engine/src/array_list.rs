//! The mutable IList adapters for ArrayList and List[object].
use rpsh_runtime::{ArrayList, RuntimeError, Value};
use std::rc::Rc;

use super::{ambiguous_overload, convert_int32, member_enumeration, method_error, overload_error};

const INDEX: &str = "Index was out of range. Must be non-negative and less than the size of the collection. (Parameter 'index')";
const INSERT_INDEX: &str = "Index was out of range. Must be non-negative and less than or equal to the size of the collection. (Parameter 'index')";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    ArrayList,
    ObjectList,
}

impl Kind {
    fn wrap(self, list: ArrayList) -> Value {
        match self {
            Self::ArrayList => Value::ArrayList(list),
            Self::ObjectList => Value::ObjectList(list),
        }
    }

    fn make(self, values: Vec<Value>) -> Value {
        self.wrap(ArrayList::from_values(values))
    }
}

pub(super) fn is_object_list_type(lower: &str) -> bool {
    matches!(
        lower.strip_prefix("system.").unwrap_or(lower),
        "collections.generic.list[object]" | "collections.generic.list[system.object]"
    )
}

pub(super) fn cast_object_list(value: Value) -> Value {
    if value.is_null_like() {
        return Value::Null;
    }
    if matches!(value, Value::ObjectList(_)) {
        return value;
    }
    let items = match value {
        Value::Array(items) | Value::StringArray(items) => items.to_vec(),
        other => vec![other],
    };
    Kind::ObjectList.make(items)
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
        Value::Array(values) | Value::StringArray(values) => {
            Ok(Kind::ArrayList.make(values.to_vec()))
        }
        Value::ObjectList(list) => Ok(Kind::ArrayList.make(list.items().to_vec())),
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

fn collection(
    kind: Kind,
    value: &Value,
    method: &str,
    arity: usize,
) -> Result<Vec<Value>, RuntimeError> {
    let (parameter, ty) = match kind {
        Kind::ArrayList => ("c", "System.Collections.ICollection"),
        Kind::ObjectList => (
            "collection",
            "System.Collections.Generic.IEnumerable`1[System.Object]",
        ),
    };
    if value.is_null_like() {
        return Err(method_error::invocation(
            "ArgumentNullException",
            method,
            arity,
            &format!("Value cannot be null. (Parameter '{parameter}')"),
        ));
    }
    if !(kind == Kind::ObjectList && matches!(value, Value::ArrayList(_)))
        && let Some(items) = value.sequence()
    {
        return Ok(items.to_vec());
    }
    let text = if matches!(value, Value::ArrayList(_)) {
        ArrayList::TYPE.to_owned()
    } else {
        value.to_string()
    };
    let detail = match value {
        Value::ArrayList(_) => {
            format!("Cannot convert the \"{text}\" value of type \"{text}\" to type \"{ty}\".")
        }
        Value::Bool(_) if kind == Kind::ObjectList => {
            let full =
                ArrayList::OBJECT_LIST_TYPE.replacen("Generic.List", "Generic.IEnumerable", 1);
            format!(
                "Cannot convert value \"{value}\" to type \"{ty}\". Error: \"Invalid cast from 'System.Boolean' to '{full}'.\""
            )
        }
        _ => cast_detail(value, ty),
    };
    Err(method_error::argument_error(
        method, parameter, ty, &text, &detail,
    ))
}

pub(super) fn construct(kind: Kind, arguments: &[Value]) -> Result<Value, RuntimeError> {
    match arguments {
        [] => Ok(kind.make(Vec::new())),
        [Value::ArrayList(_)] if kind == Kind::ObjectList => Err(overload_error("new", 1)),
        [
            value @ (Value::Array(_)
            | Value::StringArray(_)
            | Value::ArrayList(_)
            | Value::ObjectList(_)
            | Value::Null
            | Value::AutomationNull),
        ] => Ok(kind.make(collection(kind, value, ".ctor", 1)?)),
        [Value::Bool(_)] => Err(ambiguous_overload("new", 1)),
        [Value::Object(_)] => Err(overload_error("new", 1)),
        [capacity] => {
            let capacity = usize::try_from(method_error::integer(".ctor", "capacity", capacity)?)
                .map_err(|_| {
                method_error::invocation(
                    "ArgumentOutOfRangeException",
                    ".ctor",
                    1,
                    if kind == Kind::ArrayList {
                        "'capacity' must be non-negative. (Parameter 'capacity')"
                    } else {
                        "Non-negative number required. (Parameter 'capacity')"
                    },
                )
            })?;
            let mut items = Vec::new();
            items
                .try_reserve_exact(capacity)
                .map_err(|_| allocation_error(".ctor", 1))?;
            Ok(kind.make(items))
        }
        _ => Err(overload_error("new", arguments.len())),
    }
}

pub(super) fn property(kind: Kind, list: &ArrayList, name: &str) -> Result<Value, RuntimeError> {
    match name.to_ascii_lowercase().as_str() {
        "count" => Ok(Value::Int32(list.items().len() as i32)),
        "capacity" => Ok(Value::Int32(list.items().capacity() as i32)),
        "isreadonly" | "isfixedsize" | "issynchronized" => Ok(Value::Bool(false)),
        "syncroot" => Ok(kind.wrap(list.clone())),
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

pub(super) fn set_capacity(
    kind: Kind,
    list: &ArrayList,
    value: &Value,
) -> Result<(), RuntimeError> {
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
            .try_reserve_exact(if requested == 0 && kind == Kind::ArrayList {
                4
            } else {
                requested
            })
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
    kind: Kind,
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
            return Ok(if kind == Kind::ArrayList {
                Value::Int32(index as i32)
            } else {
                Value::AutomationNull
            });
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
            let values = collection(kind, value, method, arguments.len())?;
            reserve(values.len())?;
            list.items_mut().splice(at..at, values);
        }
        ("clear", []) => list.items_mut().clear(),
        ("clone", []) if kind == Kind::ArrayList => return Ok(kind.make(list.items().to_vec())),
        ("contains" | "indexof" | "remove", [value]) => {
            let index = list.items().iter().position(|item| equal(item, value));
            match method.as_str() {
                "contains" => return Ok(Value::Bool(index.is_some())),
                "indexof" => return Ok(Value::Int32(index.map_or(-1, |at| at as i32))),
                _ => {
                    if let Some(at) = index {
                        list.items_mut().remove(at);
                    }
                    if kind == Kind::ObjectList {
                        return Ok(Value::Bool(index.is_some()));
                    }
                }
            }
        }
        ("equals", [value]) => {
            return Ok(Value::Bool(kind.wrap(list.clone()) == *value));
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
        ("trimtosize", []) if kind == Kind::ArrayList => {
            let count = Value::Int32(list.items().len() as i32);
            set_capacity(kind, list, &count)?;
        }
        ("removeat", [at]) => {
            let at = checked_index(list, at, "RemoveAt", 1, false)?;
            list.items_mut().remove(at);
        }
        ("trimexcess", []) if kind == Kind::ObjectList => {
            let items = list.items();
            let len = items.len();
            let threshold = (items.capacity() as f64 * 0.9) as usize;
            drop(items);
            if len < threshold {
                set_capacity(kind, list, &Value::Int32(len as i32))?;
            }
        }
        ("toarray", []) => return Ok(Value::array(list.items().to_vec())),
        ("tostring", []) => {
            return Ok(Value::String(
                match kind {
                    Kind::ArrayList => ArrayList::TYPE,
                    Kind::ObjectList => ArrayList::OBJECT_LIST_DISPLAY,
                }
                .into(),
            ));
        }
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
