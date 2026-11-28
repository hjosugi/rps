//! Public ScriptBlock invocation methods. The caller's automatic variables
//! are visible in the child scope while arguments and output are local to it.
use super::{
    Engine, EngineError, Flow, ScriptControl, ScriptException, capture_sink, errors, hidden_type,
    member, metadata, method_error, scalar_or_array,
};
use rpsh_runtime::{ArrayList, InputEnumerator, ScriptBlock, Value};
use rpsh_syntax::{Span, Statement};
use std::path::Path;

pub(super) const TYPE: &str = "System.Management.Automation.ScriptBlock";

pub(super) fn cast(value: Value) -> Result<Value, rpsh_runtime::RuntimeError> {
    if value.is_null_like() || matches!(value, Value::ScriptBlock(_)) {
        return Ok(value);
    }
    let ty = super::get_type(&value).to_string();
    let display = match &value {
        Value::Array(_) | Value::StringArray(_) => ty.clone(),
        Value::ObjectList(_) => ArrayList::OBJECT_LIST_DISPLAY.into(),
        Value::ArrayList(_) => ArrayList::TYPE.into(),
        Value::Collection(collection) => collection.ty.display_name.into(),
        _ => value.to_string(),
    };
    Err(rpsh_runtime::RuntimeError::Script {
        id: "ConvertToFinalInvalidCastException",
        category: "InvalidArgument: (:) [], RuntimeException".into(),
        exception_type: "System.Management.Automation.RuntimeException",
        message: format!(
            "Cannot convert the \"{display}\" value of type \"{ty}\" to type \"{TYPE}\"."
        ),
    })
}
const INVOCATION_INFO: &str = "System.Management.Automation.InvocationInfo";
const DICTIONARY: &str = "System.Collections.Generic.Dictionary`2[[System.String, System.Private.CoreLib, Version=11.0.0.0, Culture=neutral, PublicKeyToken=7cec85d7bea7798e],[System.Object, System.Private.CoreLib, Version=11.0.0.0, Culture=neutral, PublicKeyToken=7cec85d7bea7798e]]";

fn string(s: impl AsRef<str>) -> Value {
    Value::String(s.as_ref().into())
}

pub(super) fn wrapped(method: &str, count: usize, inner: Value) -> EngineError {
    let ty = hidden_type(&inner)
        .unwrap_or_else(|| "System.Management.Automation.RuntimeException".into());
    let id = ty.rsplit('.').next().unwrap_or(&ty);
    let detail = member(&inner, "Message").unwrap_or(Value::Null).to_string();
    let message = rpsh_messages::format(
        "ExtendedTypeSystem/MethodInvocationException",
        &[method, &count.to_string(), &detail],
    );
    let exception = Value::object(vec![
        (
            "__TypeName".into(),
            string("System.Management.Automation.MethodInvocationException"),
        ),
        ("Message".into(), string(&message)),
        ("InnerException".into(), inner),
    ]);
    EngineError::Script(ScriptException::new(errors::record_with_exception(
        id,
        "NotSpecified: (:) [], MethodInvocationException",
        exception,
        &message,
        Value::Null,
    )))
}

fn wrap_error(error: EngineError, method: &str, count: usize) -> EngineError {
    let record = match &error {
        EngineError::Script(script) => Some(script.record().clone()),
        EngineError::Runtime(runtime) => errors::catchable(runtime),
        _ => None,
    };
    match record.and_then(|record| member(&record, "Exception").ok()) {
        Some(inner) => wrapped(method, count, inner),
        None => error,
    }
}

fn params_array(values: &[Value]) -> &[Value] {
    match values {
        [Value::Null | Value::AutomationNull] => &[],
        [Value::Array(items) | Value::StringArray(items)] => items,
        _ => values,
    }
}

fn invocation_info(path: &Value, span: Span) -> Value {
    let file = path.to_string();
    let root = Path::new(&file).parent().and_then(Path::to_str);
    Value::object(vec![
        ("__TypeName".into(), string(INVOCATION_INFO)),
        ("MyCommand".into(), Value::Null),
        (
            "BoundParameters".into(),
            Value::object(vec![("__TypeName".into(), string(DICTIONARY))]),
        ),
        (
            "UnboundArguments".into(),
            Value::ObjectList(ArrayList::default()),
        ),
        (
            "CommandOrigin".into(),
            errors::enum_value(
                "System.Management.Automation.CommandOrigin",
                "Internal",
                &["Runspace", "Internal"],
            ),
        ),
        ("ExpectingInput".into(), Value::Bool(false)),
        ("InvocationName".into(), string("")),
        ("PipelineLength".into(), Value::Int32(0)),
        ("PipelinePosition".into(), Value::Int32(0)),
        ("ScriptName".into(), string(&file)),
        ("PSCommandPath".into(), path.clone()),
        ("PSScriptRoot".into(), root.map_or(Value::Null, string)),
        ("HistoryId".into(), Value::Int64(1)),
        ("__Start".into(), Value::UInt32(span.start)),
        ("__End".into(), Value::UInt32(span.end)),
    ])
}

pub(super) fn location(value: &Value, name: &str) -> Option<Value> {
    let Value::Object(fields) = value else {
        return None;
    };
    let key = [
        "line",
        "statement",
        "scriptlinenumber",
        "offsetinline",
        "positionmessage",
    ]
    .into_iter()
    .find(|k| name.eq_ignore_ascii_case(k))?;
    if hidden_type(value).as_deref() != Some(INVOCATION_INFO) {
        return None;
    }
    let field = |name: &str| fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    let file = field("ScriptName")?.to_string();
    let source = std::fs::read_to_string(&file).ok()?;
    let at = |k| {
        if let Some(Value::UInt32(i)) = field(k) {
            Some(*i as usize)
        } else {
            None
        }
    };
    let start = at("__Start")?;
    let before = source.get(..start)?;
    let statement = source.get(start..at("__End")?)?;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = source.get(line_start..)?.split_inclusive('\n').next()?;
    let number = before.bytes().filter(|b| *b == b'\n').count() + 1;
    let offset = before[line_start..].encode_utf16().count();
    Some(match key {
        "line" => string(line),
        "statement" => string(statement),
        "scriptlinenumber" => Value::Int32(number as i32),
        "offsetinline" => Value::Int32(offset as i32 + 1),
        _ => string(format!(
            "At {file}:{number} char:{}\n+ {}\n+ {}{}",
            offset + 1,
            line.trim_end_matches(['\r', '\n']),
            " ".repeat(offset),
            "~".repeat(
                statement
                    .lines()
                    .next()
                    .unwrap_or("")
                    .encode_utf16()
                    .count()
                    .max(1)
            )
        )),
    })
}

impl Engine {
    pub(super) fn script_method(
        &mut self,
        block: &ScriptBlock,
        name: &str,
        values: &[Value],
        span: Span,
    ) -> Option<Result<Value, EngineError>> {
        let method = if name.eq_ignore_ascii_case("Invoke") {
            "Invoke"
        } else if name.eq_ignore_ascii_case("InvokeReturnAsIs") {
            "InvokeReturnAsIs"
        } else if name.eq_ignore_ascii_case("InvokeWithContext") {
            "InvokeWithContext"
        } else if name.eq_ignore_ascii_case("Equals") {
            return Some(match values {
                [other] => Ok(Value::Bool(
                    matches!(other, Value::ScriptBlock(b) if b == block),
                )),
                _ => Err(method_error::overload_error("Equals", values.len()).into()),
            });
        } else {
            return None;
        };
        Some(self.invoke_script_method(block, method, values, span))
    }

    fn invoke_script_method(
        &mut self,
        block: &ScriptBlock,
        method: &str,
        values: &[Value],
        span: Span,
    ) -> Result<Value, EngineError> {
        let context = if method == "InvokeWithContext" {
            Some(super::invoke_context::Context::bind(values)?)
        } else {
            None
        };
        let ast = block.syntax()?;
        let Some(mut body) = ast.single_clause() else {
            let inner = Value::object(vec![
                (
                    "__TypeName".into(),
                    string("System.Management.Automation.PSInvalidOperationException"),
                ),
                (
                    "Message".into(),
                    string(rpsh_messages::text(
                        "AutomationExceptions/ScriptBlockInvokeOnOneClauseOnly",
                    )),
                ),
            ]);
            return Err(wrapped(method, values.len(), inner));
        };
        let params = if let Some(Statement::Param { parameters, .. }) = ast.statements.first() {
            if ast.named_blocks.is_empty() {
                body = &body[1..];
            }
            parameters.as_slice()
        } else {
            &[]
        };
        let positional = params_array(if context.is_some() {
            &values[2..]
        } else {
            values
        });
        let supplied: Vec<_> = params
            .iter()
            .zip(positional)
            .map(|(p, v)| (p.name.clone(), v.clone()))
            .collect();
        let mut local = self.variables.child();
        let mut definitions = Vec::new();
        if let Some(context) = context {
            let dynamic = !context.variables.is_empty() && ast.uses_dynamic_locals();
            for (name, object) in context.variables {
                // These automatic slots are set by the invocation itself.
                if [
                    "args",
                    "MyInvocation",
                    "PSBoundParameters",
                    "PSScriptRoot",
                    "PSCommandPath",
                ]
                .iter()
                .all(|reserved| !name.eq_ignore_ascii_case(reserved))
                {
                    local.inject(name, object, dynamic);
                }
            }
            for (name, value) in context.automatic {
                local.insert(name.into(), value);
            }
            definitions = context.functions;
        }
        let path = self.variables.get("PSCommandPath").unwrap_or(Value::Null);
        local.insert(
            "MyInvocation".into(),
            invocation_info(&path, self.statement_span.unwrap_or(span)),
        );
        if local.get("input").is_none() {
            local.insert(
                "input".into(),
                Value::InputEnumerator(InputEnumerator::new(Vec::new())),
            );
        }
        if let Some(path) = block.file() {
            local.set_script_path(Path::new(&path.to_string_lossy()));
        } else {
            local.insert("PSCommandPath".into(), string(""));
            local.insert("PSScriptRoot".into(), string(""));
        }
        let caller = std::mem::replace(&mut self.variables, local);
        let caller_file = std::mem::replace(&mut self.source_file, block.file().cloned());
        self.functions.enter_scope(false);
        for function in definitions {
            self.functions.insert(function);
        }
        let aliases = self.aliases.clone();
        let trap = self.trap.take();
        let outer = self.capture.replace(Vec::new());
        let output = (|| {
            if params.is_empty() {
                self.variables
                    .insert("args".into(), Value::array(positional.to_vec()));
            } else {
                self.bind_parameters(params, &positional[supplied.len()..], &supplied)?;
            }
            match self.exec_block(body, &mut capture_sink())? {
                Flow::Normal | Flow::Return(None) => Ok(()),
                Flow::Return(Some(v)) => self.write_value(v, &mut capture_sink()),
                flow => Err(EngineError::Control(ScriptControl(flow))),
            }
        })();
        let records = self.capture.take().unwrap_or_default();
        self.capture = outer;
        self.trap = trap;
        self.aliases = aliases;
        self.functions.leave_scope();
        self.restore_variables(caller);
        self.source_file = caller_file;
        output.map_err(|e| wrap_error(e, method, values.len()))?;
        if method == "InvokeReturnAsIs" {
            Ok(scalar_or_array(records.into_iter()))
        } else {
            Ok(Value::collection(&metadata::PSOBJECTS, records))
        }
    }
}
