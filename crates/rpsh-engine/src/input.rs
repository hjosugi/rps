//! Enumeration at pipeline and foreach boundaries.
use super::{flatten_value, method_error, overload_error};
use rpsh_runtime::{InputEnumerator, RuntimeError, Value};

const MODIFIED_INPUT: &str = "Collection was modified; enumeration operation may not execute.";

pub(super) fn check(input: &InputEnumerator) -> Result<(), RuntimeError> {
    if input.valid() {
        return Ok(());
    }
    Err(RuntimeError::Script {
        id: "BadEnumeration",
        category: "InvalidOperation: (System.Collections.…istEnumeratorSimple:ArrayListEnumeratorSimple) [], RuntimeException".into(),
        exception_type: "System.Management.Automation.RuntimeException",
        message: format!("An error occurred while enumerating through a collection: {MODIFIED_INPUT}."),
    })
}

pub(super) enum Iteration {
    Items(std::vec::IntoIter<Value>),
    Input(InputEnumerator),
}

impl Iteration {
    pub(super) fn new(value: Value) -> Self {
        match value {
            Value::InputEnumerator(input) => Self::Input(input),
            other => Self::Items(flatten_value(other).into_iter()),
        }
    }

    pub(super) fn next_value(&mut self) -> Result<Option<Value>, RuntimeError> {
        match self {
            Self::Items(items) => Ok(items.next()),
            Self::Input(input) => {
                check(input)?;
                Ok(input
                    .advance()
                    .then(|| input.current().expect("successful MoveNext").clone()))
            }
        }
    }
}

pub(super) fn pipeline_values(value: Value) -> Result<Vec<Value>, RuntimeError> {
    if let Value::InputEnumerator(input) = &value {
        check(input)?;
    }
    Ok(if matches!(value, Value::Null) {
        vec![value]
    } else {
        flatten_value(value)
    })
}

pub(super) fn method(
    input: &InputEnumerator,
    name: &str,
    arity: usize,
) -> Option<Result<Value, RuntimeError>> {
    let advance = name.eq_ignore_ascii_case("MoveNext");
    if !advance && !name.eq_ignore_ascii_case("Reset") {
        return None;
    }
    let method = if advance { "MoveNext" } else { "Reset" };
    Some(if arity != 0 {
        Err(overload_error(method, arity))
    } else if !input.valid() {
        Err(method_error::invocation(
            "InvalidOperationException",
            method,
            0,
            MODIFIED_INPUT,
        ))
    } else if advance {
        Ok(Value::Bool(input.advance()))
    } else {
        input.reset();
        Ok(Value::AutomationNull)
    })
}
