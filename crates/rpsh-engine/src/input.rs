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

/// A ConsoleHost file invocation reads redirected text one record at a time.
/// The stdin lock is released before executing user code for that record.
pub(super) enum Records {
    Values(std::vec::IntoIter<Value>),
    Console { after_cr: bool },
}

impl From<Vec<Value>> for Records {
    fn from(values: Vec<Value>) -> Self {
        Self::Values(values.into_iter())
    }
}

impl Iterator for Records {
    type Item = std::io::Result<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Values(values) => values.next().map(Ok),
            Self::Console { after_cr } => {
                console_line(&mut std::io::stdin().lock(), after_cr).transpose()
            }
        }
    }
}

fn console_line(
    reader: &mut impl std::io::BufRead,
    after_cr: &mut bool,
) -> std::io::Result<Option<Value>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = match reader.fill_buf() {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if chunk.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        if std::mem::take(after_cr) && chunk[0] == b'\n' {
            reader.consume(1);
            continue;
        }
        if let Some(end) = chunk.iter().position(|&b| matches!(b, b'\r' | b'\n')) {
            bytes.extend_from_slice(&chunk[..end]);
            *after_cr = chunk[end] == b'\r';
            reader.consume(end + 1);
            break;
        }
        bytes.extend_from_slice(chunk);
        let n = chunk.len();
        reader.consume(n);
    }
    Ok(Some(Value::String(
        String::from_utf8_lossy(&bytes).as_ref().into(),
    )))
}
