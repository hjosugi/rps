//! Switch-Process has one optional String[] parameter. As a remaining-
//! arguments parameter it also receives otherwise unbound parameter names.
use super::{
    ArgumentBinding, Engine, EngineError, PreparedArguments, ScriptException, environment,
    errors::record_with_exception, functions::common_binding,
};
use rpsh_runtime::{PsString, Value};
use rpsh_syntax::CommandArgument;
use std::io::{self, Write};

const BINDING: &str = "System.Management.Automation.ParameterBindingException";

fn with_command(name: &str) -> bool {
    !name.is_empty() && "withcommand".starts_with(&name.to_ascii_lowercase())
}

fn record(id: &str, kind: &str, ty: &str, message: &str, target: Value) -> Value {
    let reason = ty.rsplit('.').next().unwrap_or(ty);
    let subject = match &target {
        Value::Null | Value::AutomationNull => ":".to_owned(),
        Value::StringArray(_) => "System.String[]:String[]".to_owned(),
        v => format!(
            "{v}:{}",
            v.type_name().rsplit('.').next().unwrap_or_default()
        ),
    };
    let exception = Value::object(vec![
        ("__TypeName".into(), Value::String(ty.into())),
        ("Message".into(), Value::String(message.into())),
        ("InnerException".into(), Value::Null),
    ]);
    record_with_exception(
        &format!("{id},Microsoft.PowerShell.Commands.SwitchProcessCommand"),
        &format!("{kind}: ({subject}) [Switch-Process], {reason}"),
        exception,
        message,
        target,
    )
}

fn terminating(value: Value) -> EngineError {
    EngineError::Script(ScriptException::new(value))
}

fn binding_error(id: &str, args: &[&str]) -> EngineError {
    let message = rpsh_messages::format(&format!("ParameterBinderStrings/{id}"), args);
    terminating(record(
        id,
        "InvalidArgument",
        BINDING,
        &message,
        Value::Null,
    ))
}

/// Named binding converts the parameter value as a whole. Positional
/// binding expands a sole collection, leaving nested arrays unconvertible.
fn strings(values: &[Value]) -> Result<Vec<Option<PsString>>, EngineError> {
    values
        .iter()
        .map(|v| {
            if v.sequence().is_some() {
                return Err(binding_error(
                    "CannotConvertArgument",
                    &[
                        "",
                        "WithCommand",
                        "System.String",
                        "",
                        "",
                        "",
                        v.type_name(),
                        "Specified method is not supported.",
                    ],
                ));
            }
            Ok((!v.is_null_like()).then(|| v.to_ps_string()))
        })
        .collect()
}

impl Engine {
    pub(super) fn switch_process(
        &mut self,
        args: &PreparedArguments<'_>,
        input: &[Value],
    ) -> Result<(), EngineError> {
        let mut seen = false;
        for argument in args.source {
            if let CommandArgument::Parameter { name, value, .. } = argument
                && with_command(name)
            {
                if seen {
                    return Err(binding_error("ParameterAlreadyBound", &["", "WithCommand"]));
                }
                if value.is_none() {
                    return Err(binding_error(
                        "MissingArgument",
                        &["", "WithCommand", "System.String[]"],
                    ));
                }
                seen = true;
            }
        }
        let (positional, named) = self.evaluate_arguments_matching(args, |name| {
            if with_command(name) {
                ArgumentBinding::Named
            } else {
                common_binding(&name.to_ascii_lowercase()).unwrap_or(ArgumentBinding::Unbound)
            }
        })?;
        let values = if let Some((_, value)) = named.iter().find(|(key, _)| with_command(key)) {
            if let Some(first) = positional.first() {
                return Err(binding_error(
                    "PositionalParameterNotFound",
                    &["", &first.to_string()],
                ));
            }
            if value.is_null_like() {
                return Err(terminating(record(
                    "System.NullReferenceException",
                    "NotSpecified",
                    "System.NullReferenceException",
                    "Object reference not set to an instance of an object.",
                    Value::Null,
                )));
            }
            let sequence = value.sequence();
            strings(
                sequence
                    .as_deref()
                    .unwrap_or_else(|| std::slice::from_ref(value)),
            )?
        } else if let [only] = positional.as_slice()
            && let Some(sequence) = only.sequence()
        {
            strings(&sequence)?
        } else {
            strings(&positional)?
        };
        if !input.is_empty() {
            let action = named
                .iter()
                .find(|(k, _)| {
                    k.eq_ignore_ascii_case("ea") || k.eq_ignore_ascii_case("ErrorAction")
                })
                .map(|(_, v)| v)
                .or_else(|| self.variables.get("ErrorActionPreference"))
                .map_or_else(|| "continue".into(), |v| v.to_string().to_ascii_lowercase());
            self.cmdlet_has_errors = true;
            let message = rpsh_messages::text("ParameterBinderStrings/InputObjectNotBound");
            for value in input {
                let error = record(
                    "InputObjectNotBound",
                    "InvalidArgument",
                    BINDING,
                    message,
                    value.clone(),
                );
                match action.as_str() {
                    "ignore" => continue,
                    "stop" => return Err(terminating(error)),
                    "silentlycontinue" => {}
                    "continue" => writeln!(io::stderr().lock(), "Switch-Process: {message}")?,
                    _ => {
                        return Err(rpsh_runtime::RuntimeError::NotImplemented {
                            feature: "this action preference for rejected pipeline input",
                        }
                        .into());
                    }
                }
                self.remember_error(&error);
            }
        }
        let Some(program) = values.first() else {
            return Ok(());
        };
        let name = program
            .as_ref()
            .map_or_else(String::new, PsString::to_string_lossy);
        let path = self
            .application_paths(&name, false)
            .into_iter()
            .find(|path| {
                !path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("ps1"))
            })
            .ok_or_else(|| {
                let message =
                    rpsh_messages::format("CommandBaseStrings/NativeCommandNotFound", &[&name]);
                terminating(record(
                    "CommandNotFound",
                    "InvalidArgument",
                    "System.Management.Automation.CommandNotFoundException",
                    &message,
                    program.clone().map_or(Value::Null, Value::String),
                ))
            })?;
        // The marshaled execve argv ends at the first null reference. A NUL
        // inside an individual string ends that argument, not the vector.
        let argv: Vec<_> = values[1..]
            .iter()
            .map_while(Option::as_ref)
            .map(|s| {
                let mut text = s.to_string_lossy();
                if let Some(at) = text.find('\0') {
                    text.truncate(at);
                }
                text
            })
            .collect();
        let error = rpsh_native::replace_process(
            &path,
            &argv,
            environment::child_overrides(&self.variables),
        );
        let command = values
            .iter()
            .map(|s| {
                s.as_ref()
                    .map_or_else(String::new, PsString::to_string_lossy)
            })
            .collect::<Vec<_>>()
            .join(" ");
        let message = rpsh_messages::format(
            "CommandBaseStrings/ExecFailed",
            &[&error.raw_os_error().unwrap_or(22).to_string(), &command],
        );
        Err(terminating(record(
            "ExecutionFailed",
            "InvalidOperation",
            "System.Exception",
            &message,
            Value::string_array(values),
        )))
    }
}
