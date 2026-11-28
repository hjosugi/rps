//! PowerShell exceptions, the active handler, and the session's error history.
use super::{Engine, EngineError, Flow, hidden_type, member};
use rpsh_runtime::{PsString, RuntimeError, Value};
use rpsh_syntax::{Span, Statement};
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;

/// A script error that `try/catch` or `trap` can catch, from the refusal
/// features that are real PowerShell exceptions rather than missing support.
pub(super) fn error_record(
    id: &str,
    category: &str,
    exception_type: &str,
    message: &str,
    text: &str,
) -> Value {
    let exception = Value::object(vec![
        (
            "__TypeName".to_owned(),
            Value::String(exception_type.to_owned().into()),
        ),
        (
            "Message".to_owned(),
            Value::String(message.to_owned().into()),
        ),
    ]);
    record_with_exception(id, category, exception, text, Value::Null)
}

pub(super) fn record_with_exception(
    id: &str,
    category: &str,
    exception: Value,
    text: &str,
    target: Value,
) -> Value {
    Value::object(vec![
        ("__Text".to_owned(), Value::String(text.to_owned().into())),
        (
            "__TypeName".to_owned(),
            Value::String("System.Management.Automation.ErrorRecord".into()),
        ),
        (
            "FullyQualifiedErrorId".to_owned(),
            Value::String(id.to_owned().into()),
        ),
        ("CategoryInfo".to_owned(), category_info(category)),
        ("Exception".to_owned(), exception),
        ("TargetObject".into(), target),
    ])
}

pub(super) fn catchable(error: &RuntimeError) -> Option<Value> {
    if let Some(script) = error.script() {
        return Some(error_record(
            script.id,
            script.category,
            script.exception_type,
            script.message,
            script.message,
        ));
    }
    let feature = match error {
        RuntimeError::NotImplemented { feature } => feature,
        RuntimeError::Script { .. } => return None,
    };
    let (id, message) = match *feature {
        "cannot convert a string to a double" => (
            "InvalidCastFromStringToDouble",
            "Cannot convert value to type \"System.Double\".",
        ),
        _ => return None,
    };
    Some(error_record(
        id,
        "NotSpecified: (:) [], RuntimeException",
        "System.Management.Automation.RuntimeException",
        message,
        message,
    ))
}

const RECORD: &str = "System.Management.Automation.ErrorRecord";
const CATEGORIES: &[&str] = &[
    "NotSpecified",
    "OpenError",
    "CloseError",
    "DeviceError",
    "DeadlockDetected",
    "InvalidArgument",
    "InvalidData",
    "InvalidOperation",
    "InvalidResult",
    "InvalidType",
    "MetadataError",
    "NotImplemented",
    "NotInstalled",
    "ObjectNotFound",
    "OperationStopped",
    "OperationTimeout",
    "SyntaxError",
    "ParserError",
    "PermissionDenied",
    "ResourceBusy",
    "ResourceExists",
    "ResourceUnavailable",
    "ReadError",
    "WriteError",
    "FromStdErr",
    "SecurityError",
    "ProtocolError",
    "ConnectionError",
    "AuthenticationError",
    "LimitsExceeded",
    "QuotaExceeded",
    "NotEnabled",
];

pub(super) fn enum_value(ty: &str, name: &str, names: &[&str]) -> Value {
    let number = names.iter().position(|entry| *entry == name).unwrap_or(0);
    Value::object(vec![
        ("__TypeName".into(), Value::String(ty.into())),
        ("__Text".into(), Value::String(name.into())),
        ("__EnumValue".into(), Value::Int32(number as i32)),
    ])
}

fn category_info(text: &str) -> Value {
    let (kind, rest) = text.split_once(": (").unwrap_or(("NotSpecified", ""));
    let (target, rest) = rest.rsplit_once(") [").unwrap_or(("", ""));
    let (name, ty) = target.rsplit_once(':').unwrap_or(("", ""));
    let (activity, reason) = rest.split_once("], ").unwrap_or(("", ""));
    let abbreviated = |s: &str| {
        let units: Vec<u16> = s.encode_utf16().collect();
        if units.len() > 40 {
            let mut result = units[..19].to_vec();
            result.push('…' as u16);
            result.extend_from_slice(&units[units.len() - 19..]);
            PsString::from_units(result)
        } else {
            PsString::from(s)
        }
    };
    let display = if [name, ty, activity].iter().any(|s| s.len() > 40) {
        PsString::from(format!("{kind}: ("))
            .concat(&abbreviated(name))
            .concat(&":".into())
            .concat(&abbreviated(ty))
            .concat(&") [".into())
            .concat(&abbreviated(activity))
            .concat(&format!("], {reason}").into())
    } else {
        text.into()
    };
    Value::object(vec![
        (
            "__TypeName".into(),
            Value::String("System.Management.Automation.ErrorCategoryInfo".into()),
        ),
        ("__Text".into(), Value::String(display)),
        (
            "Category".into(),
            enum_value(
                "System.Management.Automation.ErrorCategory",
                kind,
                CATEGORIES,
            ),
        ),
        ("Activity".into(), Value::String(activity.into())),
        ("Reason".into(), Value::String(reason.into())),
        ("TargetName".into(), Value::String(name.into())),
        ("TargetType".into(), Value::String(ty.into())),
    ])
}

pub(super) fn constant_error(remove: bool) -> RuntimeError {
    RuntimeError::Script {
        id: if remove { "VariableNotRemovable" } else { "VariableNotWritable" },
        category: "WriteError: (Error:String) [], SessionStateUnauthorizedAccessException".into(),
        exception_type: "System.Management.Automation.SessionStateUnauthorizedAccessException",
        message: if remove { "Cannot remove variable Error because it is constant or read-only. If the variable is read-only, try the operation again specifying the Force option." } else { "Cannot overwrite variable Error because it is read-only or constant." }.into(),
    }
}

fn constant_cmdlet(command: &str, force: bool) -> RuntimeError {
    let (id, name) = match command {
        "new-variable" if !force => {
            return RuntimeError::Script {
                id: "VariableAlreadyExists,Microsoft.PowerShell.Commands.NewVariableCommand",
                category: "ResourceExists: (Error:String) [New-Variable], SessionStateException"
                    .into(),
                exception_type: "System.Management.Automation.SessionStateException",
                message: "A variable with name 'Error' already exists.".into(),
            };
        }
        "new-variable" => (
            "VariableNotWritable,Microsoft.PowerShell.Commands.NewVariableCommand",
            "New-Variable",
        ),
        "set-variable" => (
            "VariableNotWritable,Microsoft.PowerShell.Commands.SetVariableCommand",
            "Set-Variable",
        ),
        "clear-variable" => (
            "VariableNotWritable,Microsoft.PowerShell.Commands.ClearVariableCommand",
            "Clear-Variable",
        ),
        "remove-variable" => (
            "VariableNotRemovable,Microsoft.PowerShell.Commands.RemoveVariableCommand",
            "Remove-Variable",
        ),
        _ => unreachable!("variable cmdlet"),
    };
    let mut error = constant_error(command == "remove-variable");
    if let RuntimeError::Script {
        id: slot, category, ..
    } = &mut error
    {
        *slot = id;
        *category =
            format!("WriteError: (Error:String) [{name}], SessionStateUnauthorizedAccessException");
    }
    error
}

/// A catchable PowerShell error, including its original statement on a rethrow.
#[derive(Clone, Debug)]
pub struct ScriptException {
    record: Value,
    span: Option<Span>,
    recorded: bool,
    explicit_throw: bool,
}

impl ScriptException {
    pub(super) fn new(record: Value) -> Self {
        Self {
            record,
            span: None,
            recorded: false,
            explicit_throw: false,
        }
    }

    pub(super) fn rethrow(&self) -> Self {
        Self {
            recorded: false,
            ..self.clone()
        }
    }

    fn exception_type(&self) -> String {
        member(&self.record, "Exception")
            .ok()
            .and_then(|value| hidden_type(&value))
            .unwrap_or_default()
    }

    pub(super) fn matches(&self, kind: &str) -> bool {
        let actual = self.exception_type();
        let short = actual.rsplit('.').next().unwrap_or(&actual);
        kind.eq_ignore_ascii_case(&actual)
            || kind.eq_ignore_ascii_case(short)
            || [
                "Exception",
                "System.Exception",
                "RuntimeException",
                "System.Management.Automation.RuntimeException",
            ]
            .iter()
            .any(|ty| kind.eq_ignore_ascii_case(ty))
    }

    fn label(&self) -> String {
        if self.explicit_throw {
            return "Exception".into();
        }
        let ty = self.exception_type();
        if ty.ends_with(".CommandNotFoundException")
            && let Ok(category) = member(&self.record, "CategoryInfo")
            && let Ok(name) = member(&category, "TargetName")
        {
            return name.to_string();
        }
        ty.rsplit('.').next().unwrap_or("Exception").into()
    }

    /// ConsoleHost's plain ConciseView for a command or a single-line script extent.
    pub fn diagnostic(&self, source: &str, path: Option<&Path>) -> String {
        let Some((span, path)) = self.span.zip(path) else {
            return self.to_string();
        };
        let Some(before) = source.get(..span.start as usize) else {
            return self.to_string();
        };
        let Some(fragment) = source.get(span.start as usize..span.end as usize) else {
            return self.to_string();
        };
        let Some(line) = source[before.rfind('\n').map_or(0, |n| n + 1)..]
            .lines()
            .next()
        else {
            return self.to_string();
        };
        let number = before.bytes().filter(|b| *b == b'\n').count() + 1;
        let indent = before
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .count();
        let width = fragment
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .count()
            .max(1);
        let label = self.label();
        let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        let message = self
            .record
            .to_string()
            .lines()
            .collect::<Vec<_>>()
            .join("\n     | ");
        let mut diagnostic = format!(
            "{label}: {}:{number}\nLine |\n{number:>4} |  {}\n     |  {}{}\n     | {message}",
            path.display(),
            line.trim_end(),
            " ".repeat(indent),
            "~".repeat(width)
        );
        if std::env::var_os("TERM").as_deref() != Some(std::ffi::OsStr::new("dumb")) {
            diagnostic.push('\n');
        }
        diagnostic
    }
}

impl fmt::Display for ScriptException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.label(), self.record)
    }
}

pub(super) fn thrown(value: Value) -> ScriptException {
    if hidden_type(&value).as_deref() == Some(RECORD) {
        return ScriptException::new(value);
    }
    let (message, target) = if value.is_null_like() {
        ("ScriptHalted".to_owned(), ":".to_owned())
    } else {
        let ty = value.type_name().rsplit('.').next().unwrap_or_default();
        (value.to_string(), format!("{value}:{ty}"))
    };
    let mut record = error_record(
        &message,
        &format!("OperationStopped: ({target}) [], RuntimeException"),
        "System.Management.Automation.RuntimeException",
        &message,
        &message,
    );
    if let Value::Object(fields) = &mut record
        && let Some((_, target)) = Rc::make_mut(fields)
            .iter_mut()
            .find(|(key, _)| key == "TargetObject")
    {
        *target = value;
    }
    ScriptException {
        explicit_throw: true,
        ..ScriptException::new(record)
    }
}

fn exception_field(record: &Value) -> Option<&Value> {
    let Value::Object(fields) = record else {
        return None;
    };
    fields
        .iter()
        .find(|(name, _)| name == "Exception")
        .map(|(_, value)| value)
}

impl Engine {
    pub(super) fn check_error_variable(
        &self,
        command: &str,
        name: &str,
        named: &[(String, Value)],
    ) -> Result<(), RuntimeError> {
        let key = super::strip_scope(name);
        let matches = key.eq_ignore_ascii_case("Error")
            || command != "new-variable"
                && key.contains(['*', '?', '['])
                && rpsh_runtime::ops::wildcard_match("Error", key, false);
        if matches {
            let at_root = !self.variables.nested
                || super::is_global_scope(name)
                || named.iter().any(|(parameter, v)| {
                    parameter.eq_ignore_ascii_case("Scope")
                        && matches!(
                            v.to_string().to_ascii_lowercase().as_str(),
                            "global" | "script"
                        )
                });
            if at_root {
                let force = named
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("Force") && rpsh_runtime::ops::is_true(v));
                return Err(constant_cmdlet(command, force));
            }
        }
        Ok(())
    }

    pub(super) fn remember_error(&self, record: &Value) {
        let history = self.variables.errors();
        let mut entries = history.items_mut();
        let same = |a: &Value, b: &Value| matches!((a,b),(Value::Object(x), Value::Object(y)) if Rc::ptr_eq(x,y));
        if let Some(first) = entries.first()
            && (same(first, record)
                || exception_field(first)
                    .zip(exception_field(record))
                    .is_some_and(|(a, b)| same(a, b)))
        {
            return;
        }
        entries.truncate(255);
        entries.insert(0, record.clone());
    }

    pub(super) fn record_exception(&mut self, err: EngineError, span: Span) -> EngineError {
        let mut error = match err {
            EngineError::Runtime(error) => match catchable(&error) {
                Some(record) => ScriptException::new(record),
                None => return EngineError::Runtime(error),
            },
            EngineError::Script(error) => error,
            other => return other,
        };
        error.span.get_or_insert(span);
        if !error.recorded {
            self.remember_error(&error.record);
            error.recorded = true;
        }
        self.last_failed = true;
        EngineError::Script(error)
    }

    pub(super) fn handle_exception(
        &mut self,
        error: &ScriptException,
        body: &[Statement],
        out: &mut dyn Write,
    ) -> Result<Flow, EngineError> {
        let previous = self.current_exception.replace(error.clone());
        let item = self.variables.insert("_".into(), error.record.clone());
        self.last_failed = true;
        let result = self.exec_block(body, out);
        self.current_exception = previous;
        match item {
            Some(value) => {
                self.variables.insert("_".into(), value);
            }
            None => {
                self.variables.remove("_");
            }
        }
        result
    }
}
