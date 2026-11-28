//! Overload binding and exception messages shared by .NET adapters.
use super::convert_int32;
use rpsh_runtime::{RuntimeError, Value};

pub(super) fn ambiguous_overload(method: &str, arity: usize) -> RuntimeError {
    RuntimeError::Script {
        id: "MethodCountCouldNotFindBest",
        category: "NotSpecified: (:) [], MethodException".into(),
        exception_type: "System.Management.Automation.MethodException",
        message: format!(
            "Multiple ambiguous overloads found for \"{method}\" and the argument count: \"{arity}\"."
        ),
    }
}

pub(super) fn overload_error(method: &str, arity: usize) -> RuntimeError {
    RuntimeError::Script {
        id: "MethodCountCouldNotFindBest",
        category: "NotSpecified: (:) [], MethodException".into(),
        exception_type: "System.Management.Automation.MethodException",
        message: format!(
            "Cannot find an overload for \"{method}\" and the argument count: \"{arity}\"."
        ),
    }
}

pub(super) fn invocation(
    id: &'static str,
    method: &str,
    count: usize,
    detail: &str,
) -> RuntimeError {
    RuntimeError::Script {
        id,
        category: "NotSpecified: (:) [], MethodInvocationException".into(),
        exception_type: "System.Management.Automation.MethodInvocationException",
        message: format!(
            "Exception calling \"{method}\" with \"{count}\" argument(s): \"{detail}\""
        ),
    }
}

pub(super) fn argument_error(
    method: &str,
    parameter: &str,
    ty: &str,
    text: &str,
    detail: &str,
) -> RuntimeError {
    RuntimeError::Script {
        id: "MethodArgumentConversionInvalidCastArgument",
        category: "NotSpecified: (:) [], MethodException".into(),
        exception_type: "System.Management.Automation.MethodException",
        message: format!(
            "Cannot convert argument \"{parameter}\", with value: \"{text}\", for \"{method}\" to type \"{ty}\": \"{detail}\""
        ),
    }
}

pub(super) fn integer(method: &str, name: &str, value: &Value) -> Result<i32, RuntimeError> {
    if matches!(value,Value::String(text) if text.is_empty()) {
        return Ok(0);
    }
    convert_int32(value).map_err(|_|{
        let why=format!("Cannot convert value \"{value}\" to type \"System.Int32\". Error: \"The input string '{value}' was not in a correct format.\"");
        argument_error(method,name,"System.Int32",&value.to_string(),&why)
    })
}

pub(super) fn nonnegative(name: &str, value: i64) -> String {
    format!(
        "{name} ('{value}') must be a non-negative value. (Parameter '{name}')\nActual value was {value}."
    )
}

pub(super) fn cast_error(id: &'static str, detail: String) -> RuntimeError {
    RuntimeError::Script {
        id,
        category: "InvalidArgument: (:) [], RuntimeException".into(),
        exception_type: "System.Management.Automation.RuntimeException",
        message: detail,
    }
}
