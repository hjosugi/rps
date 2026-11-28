//! Bind the statically registered compiled cmdlets and run their invocation lifecycle.
use super::{
    CallArgument, Engine, EngineError, PreparedArguments, ScriptException,
    errors::record_with_exception, hidden_type, input,
};
use rpsh_commands::framework::{BoundParameters, CommandRuntime, Registration};
use rpsh_commands::signature::{COMMON_PARAMETERS, Parameter, ParameterType};
use rpsh_runtime::{ArrayList, PsString, RuntimeError, Value, ops};
use rpsh_syntax::ArgumentSeparator;
use std::io::{self, Write};

type Entry = Registration<EngineError>;

fn binding_record(entry: &Entry, id: &str, args: &[&str], target: Value) -> Value {
    let ty = "System.Management.Automation.ParameterBindingException";
    let text = rpsh_messages::format(&format!("ParameterBinderStrings/{id}"), args);
    let full_type = hidden_type(&target).unwrap_or_else(|| target.type_name().to_owned());
    let short = full_type
        .split("[[")
        .next()
        .unwrap_or(&full_type)
        .rsplit('.')
        .next()
        .unwrap_or(&full_type);
    let subject = if target.is_null_like() {
        ":".to_owned()
    } else if target.sequence().is_some() {
        format!("{full_type}:{short}")
    } else {
        format!("{target}:{short}")
    };
    let exception = Value::object(vec![
        ("__TypeName".into(), Value::String(ty.into())),
        ("Message".into(), Value::String(text.as_str().into())),
        ("InnerException".into(), Value::Null),
    ]);
    record_with_exception(
        &format!("{id},{}", entry.implementation),
        &format!(
            "InvalidArgument: ({subject}) [{}], ParameterBindingException",
            entry.signature.name
        ),
        exception,
        &text,
        target,
    )
}

fn binding_error(entry: &Entry, id: &str, args: &[&str]) -> EngineError {
    EngineError::Script(ScriptException::new(binding_record(
        entry,
        id,
        args,
        Value::Null,
    )))
}

fn resolve_in(
    entry: &Entry,
    parameters: &'static [Parameter],
    name: &str,
) -> Result<Option<&'static Parameter>, EngineError> {
    if let Some(exact) = parameters.iter().find(|p| p.is_named(name)) {
        return Ok(Some(exact));
    }
    let mut matches = parameters.iter().filter(|p| p.matches_prefix(name));
    let Some(first) = matches.next() else {
        return Ok(None);
    };
    if let Some(second) = matches.next() {
        let options = [first, second]
            .into_iter()
            .chain(matches)
            .map(|p| format!(" -{}", p.name))
            .collect::<String>();
        return Err(binding_error(
            entry,
            "AmbiguousParameter",
            &["", name, "", "", "", "", &options],
        ));
    }
    Ok(Some(first))
}

fn resolve(entry: &Entry, name: &str) -> Result<Option<(&'static Parameter, bool)>, EngineError> {
    if let Some(p) = resolve_in(entry, entry.signature.parameters, name)? {
        return Ok(Some((p, false)));
    }
    Ok(resolve_in(entry, COMMON_PARAMETERS, name)?.map(|p| (p, true)))
}

struct Invocation {
    bound: BoundParameters,
    common: Vec<(String, Value)>,
}

fn switch(entry: &Entry, param: &Parameter, value: Value) -> Result<Value, EngineError> {
    match value.base_value() {
        Value::Null | Value::AutomationNull => Ok(Value::Bool(true)),
        Value::Bool(_)
        | Value::Int32(_)
        | Value::UInt32(_)
        | Value::Int64(_)
        | Value::UInt64(_)
        | Value::Int16(_)
        | Value::UInt16(_)
        | Value::Byte(_)
        | Value::SByte(_)
        | Value::Decimal(_)
        | Value::Double(_) => Ok(Value::Bool(ops::is_true(&value))),
        _ => Err(binding_error(
            entry,
            "CannotConvertArgument",
            &[
                "",
                param.name,
                param.ty.full_name(),
                "",
                "",
                "",
                value.type_name(),
                "",
            ],
        )),
    }
}

fn mandatory_error(entry: &Entry, param: &Parameter, value: &Value) -> Option<Value> {
    if !param.mandatory {
        return None;
    }
    let id = if !param.allow_null && value.is_null_like() {
        "ParameterArgumentValidationErrorNullNotAllowed"
    } else if !param.allow_empty_collection && value.sequence().is_some_and(|v| v.is_empty()) {
        if matches!(value, Value::Array(_) | Value::StringArray(_)) {
            "ParameterArgumentValidationErrorEmptyArrayNotAllowed"
        } else {
            "ParameterArgumentValidationErrorEmptyCollectionNotAllowed"
        }
    } else {
        return None;
    };
    let message = rpsh_messages::format(&format!("ParameterBinderStrings/{id}"), &["", param.name]);
    Some(super::error_record(
        &format!("{id},{}", entry.implementation),
        &format!(
            "InvalidData: (:) [{}], ParameterBindingValidationException",
            entry.signature.name
        ),
        "System.Management.Automation.ParameterBindingValidationException",
        &message,
        &message,
    ))
}

fn remaining_arguments(mut items: Vec<Value>) -> Value {
    if items.len() == 1 && items[0].sequence().is_some() {
        return items.pop().unwrap();
    }
    // ValueFromRemainingArguments constructs a List<object> by adding each
    // token individually (initial capacity 4, doubling on expansion).
    let mut list = Vec::with_capacity(items.len().next_power_of_two().max(4));
    list.append(&mut items);
    Value::ObjectList(ArrayList::from_values(list))
}

fn bind(entry: &Entry, args: &PreparedArguments<'_>) -> Result<Invocation, EngineError> {
    let mut bound = BoundParameters::default();
    let mut common = Vec::new();
    let mut positional = Vec::new();
    // A named argument overrides a value splatted from a hashtable, even
    // when the hashtable was expanded first on the command line.
    let mut explicit = Vec::new();
    for arg in args.for_binding() {
        if let CallArgument::Parameter {
            name,
            from_hashtable: false,
            ..
        } = arg
            && let Some((p, _)) = resolve(entry, &name)?
        {
            explicit.push(p.name);
        }
    }
    let mut args = args.for_binding().peekable();
    let rest = entry.signature.parameters.iter().find(|p| p.remaining);
    while let Some(arg) = args.next() {
        let CallArgument::Parameter {
            name,
            text,
            separator,
            value,
            from_hashtable,
        } = arg
        else {
            if let CallArgument::Positional(value) = arg {
                positional.push(value);
            }
            continue;
        };
        let Some((p, is_common)) = resolve(entry, &name)? else {
            if rest.is_none() {
                return Err(binding_error(entry, "NamedParameterNotFound", &["", &name]));
            }
            positional.push(Value::String(PsString::command_parameter(&text, &name)));
            positional.extend(value);
            continue;
        };
        if from_hashtable && explicit.contains(&p.name) {
            continue;
        }
        if bound.get(p.name).is_some() {
            let reported = if p.ty == ParameterType::Switch {
                name.as_str()
            } else {
                p.name
            };
            return Err(binding_error(
                entry,
                "ParameterAlreadyBound",
                &["", reported],
            ));
        }
        let value = if p.ty == ParameterType::Switch && separator == ArgumentSeparator::Space {
            positional.extend(value);
            Value::Bool(true)
        } else {
            let value = value
                .or_else(|| {
                    if matches!(args.peek(), Some(CallArgument::Positional(_)))
                        && let Some(CallArgument::Positional(v)) = args.next()
                    {
                        Some(v)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    binding_error(entry, "MissingArgument", &["", p.name, p.ty.full_name()])
                })?;
            if p.ty == ParameterType::Switch {
                switch(entry, p, value)?
            } else {
                value
            }
        };
        if is_common {
            common.push((p.name.to_owned(), value.clone()));
        }
        bound.set(p.name, value);
    }
    let mut positionals = entry
        .signature
        .parameters
        .iter()
        .filter(|p| p.position.is_some())
        .collect::<Vec<_>>();
    positionals.sort_by_key(|p| p.position);
    let mut items = positional.into_iter();
    for p in positionals {
        if bound.get(p.name).is_some() {
            continue;
        }
        let Some(item) = items.next() else {
            break;
        };
        if p.remaining {
            bound.set(
                p.name,
                remaining_arguments(std::iter::once(item).chain(items.by_ref()).collect()),
            );
        } else {
            bound.set(p.name, item);
        }
    }
    if let Some(extra) = items.next() {
        if let Value::String(text) = &extra
            && let Some(name) = text.command_parameter_name()
        {
            return Err(binding_error(entry, "NamedParameterNotFound", &["", name]));
        }
        let text = match &extra {
            Value::Null | Value::AutomationNull => "$null".to_owned(),
            Value::ObjectList(_) => ArrayList::OBJECT_LIST_DISPLAY.to_owned(),
            Value::Array(_) | Value::StringArray(_) | Value::ArrayList(_) => {
                extra.type_name().to_owned()
            }
            value => value.to_string(),
        };
        return Err(binding_error(
            entry,
            "PositionalParameterNotFound",
            &["", &text],
        ));
    }
    for param in entry.signature.parameters {
        if let Some(value) = bound.get(param.name)
            && let Some(record) = mandatory_error(entry, param, value)
        {
            return Err(EngineError::Script(ScriptException::new(record)));
        }
    }
    Ok(Invocation { bound, common })
}

struct Runtime<'a> {
    engine: &'a mut Engine,
    common: &'a [(String, Value)],
    output: Vec<Value>,
    out_variable: Option<ArrayList>,
    error_variable: Option<ArrayList>,
}

impl CommandRuntime<EngineError> for Runtime<'_> {
    fn write_object(&mut self, value: Value, enumerate: bool) -> Result<(), EngineError> {
        let start = self.output.len();
        if enumerate {
            self.output.extend(input::pipeline_values(value)?);
        } else if !matches!(value, Value::AutomationNull) {
            self.output.push(value);
        }
        if let Some(capture) = &self.out_variable {
            capture.items_mut().extend_from_slice(&self.output[start..]);
        }
        Ok(())
    }
    fn write_error(&mut self, record: Value) -> Result<(), EngineError> {
        if let Some(capture) = &self.error_variable {
            match self.engine.error_action(self.common).as_str() {
                "ignore" => (),
                "stop" => {
                    let message = rpsh_messages::format(
                        "CommandBaseStrings/ErrorPreferenceStop",
                        &["ErrorActionPreference", &record.to_string()],
                    );
                    capture.items_mut().push(Value::object(vec![
                        (
                            "__TypeName".into(),
                            Value::String(
                                "System.Management.Automation.ActionPreferenceStopException".into(),
                            ),
                        ),
                        ("Message".into(), Value::String(message.into())),
                        ("ErrorRecord".into(), record.clone()),
                    ]));
                }
                _ => capture.items_mut().push(record.clone()),
            }
        }
        self.engine.cmdlet_result::<()>(
            Err(EngineError::Script(ScriptException::new(record))),
            self.common,
        )?;
        Ok(())
    }
}

/// Validate both capture targets before changing either session variable.
fn capture_target(
    entry: &Entry,
    bound: &BoundParameters,
    parameter: &str,
) -> Result<Option<(bool, String)>, EngineError> {
    let Some(value) = bound.get(parameter).filter(|v| !v.is_null_like()) else {
        return Ok(None);
    };
    let raw = value.to_string();
    let (append, name) = raw
        .strip_prefix('+')
        .map_or((false, raw.as_str()), |n| (true, n));
    let mut variable = name;
    let detail = if name.is_empty() {
        Some(rpsh_messages::format(
            "AutomationExceptions/Argument",
            &["path"],
        ))
    } else if let Some((scope, rest)) = name.split_once(':') {
        if ["global", "local", "script", "private", "variable"]
            .iter()
            .any(|s| scope.eq_ignore_ascii_case(s))
        {
            if rest.is_empty() {
                let message = rpsh_messages::format("AutomationExceptions/Argument", &["name"]);
                return Err(RuntimeError::Script {
                    id: "Argument",
                    category: "InvalidArgument: (:) [], RuntimeException".into(),
                    exception_type: "System.Management.Automation.RuntimeException",
                    message,
                }
                .into());
            }
            if scope.eq_ignore_ascii_case("variable") {
                variable = rest;
            }
            None
        } else {
            Some(rpsh_messages::format(
                "Metadata/ValidateVariableName",
                &[name],
            ))
        }
    } else {
        None
    };
    if let Some(detail) = detail {
        let message = rpsh_messages::format(
            "ParameterBinderStrings/ParameterArgumentValidationError",
            &["", parameter, "", "", "", "", &detail],
        );
        let record = super::error_record(
            &format!("ParameterArgumentValidationError,{}", entry.implementation),
            &format!(
                "InvalidData: (:) [{}], ParameterBindingValidationException",
                entry.signature.name
            ),
            "System.Management.Automation.ParameterBindingValidationException",
            &message,
            &message,
        );
        return Err(EngineError::Script(ScriptException::new(record)));
    }
    Ok(Some((append, variable.to_owned())))
}

impl Engine {
    fn capture_cmdlet_variable(
        &mut self,
        target: Option<(bool, String)>,
    ) -> Result<Option<ArrayList>, EngineError> {
        let Some((append, name)) = target else {
            return Ok(None);
        };
        let variable = name.as_str();
        let previous = append
            .then(|| {
                super::lookup(
                    variable,
                    &self.variables,
                    !self.last_failed,
                    &self.functions,
                )
            })
            .flatten();
        let list = match previous {
            Some(Value::ArrayList(list)) => list,
            Some(value) if !value.is_null_like() => {
                let items = value
                    .sequence()
                    .map(|v| v.to_vec())
                    .unwrap_or_else(|| vec![value]);
                ArrayList::from_values(items)
            }
            _ => ArrayList::default(),
        };
        self.assign_variable(variable, Value::ArrayList(list.clone()))?;
        Ok(Some(list))
    }

    pub(super) fn run_registered(
        &mut self,
        entry: &Entry,
        args: &PreparedArguments<'_>,
        pipeline: Option<&[Value]>,
    ) -> Result<Vec<Value>, EngineError> {
        let Invocation { mut bound, common } = bind(entry, args)?;
        let out_variable = capture_target(entry, &bound, "OutVariable")?;
        let error_variable = capture_target(entry, &bound, "ErrorVariable")?;
        let pipeline_param = entry
            .signature
            .parameters
            .iter()
            .find(|p| p.from_pipeline && bound.get(p.name).is_none());
        let missing = entry
            .signature
            .parameters
            .iter()
            .filter(|p| {
                p.mandatory
                    && bound.get(p.name).is_none()
                    && !(p.from_pipeline && pipeline.is_some())
            })
            .map(|p| p.name)
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(binding_error(
                entry,
                "MissingMandatoryParameter",
                &["", &format!(" {}", missing.join(" "))],
            ));
        }
        let out_variable = self.capture_cmdlet_variable(out_variable)?;
        let error_variable = self.capture_cmdlet_variable(error_variable)?;
        let mut cmdlet = (entry.construct)();
        let mut runtime = Runtime {
            engine: self,
            common: &common,
            output: Vec::new(),
            out_variable,
            error_variable,
        };
        let result = (|| {
            cmdlet.begin(&bound, &mut runtime)?;
            match pipeline {
                Some(inputs) => {
                    for value in inputs {
                        if let Some(param) = pipeline_param {
                            if let Some(record) = mandatory_error(entry, param, value) {
                                runtime.write_error(record)?;
                                continue;
                            }
                            bound.set(param.name, value.clone());
                            cmdlet.process(&bound, &mut runtime)?;
                        } else {
                            runtime.write_error(binding_record(
                                entry,
                                "InputObjectNotBound",
                                &[],
                                value.clone(),
                            ))?;
                        }
                    }
                }
                None => cmdlet.process(&bound, &mut runtime)?,
            }
            cmdlet.end(&mut runtime)
        })();
        if let Err(error) = result {
            cmdlet.stop(&mut runtime);
            return Err(error);
        }
        Ok(runtime.output)
    }
}

impl Engine {
    fn error_action(&self, named: &[(String, Value)]) -> String {
        super::function_drive::option(named, &["ErrorAction", "ea"])
            .or_else(|| self.variables.get("ErrorActionPreference"))
            .map_or_else(|| "continue".into(), |v| v.to_string().to_ascii_lowercase())
    }

    pub(super) fn cmdlet_result<T>(
        &mut self,
        result: Result<T, EngineError>,
        named: &[(String, Value)],
    ) -> Result<Option<T>, EngineError> {
        let error = match result {
            Ok(value) => return Ok(Some(value)),
            Err(EngineError::Script(error)) => error,
            Err(other) => return Err(other),
        };
        let action = self.error_action(named);
        self.cmdlet_has_errors = true;
        match action.as_str() {
            "stop" => return Err(EngineError::Script(error)),
            "ignore" => return Ok(None),
            "silentlycontinue" => {}
            "continue" => writeln!(io::stderr().lock(), "{error}")?,
            _ => {
                return Err(RuntimeError::NotImplemented {
                    feature: "this non-terminating cmdlet ErrorAction",
                }
                .into());
            }
        }
        self.remember_error(error.record());
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpsh_commands::{framework::Cmdlet, signature::WRITE_OUTPUT};
    use std::cell::RefCell;

    thread_local! { static PHASES: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) }; }

    struct Trace;

    impl Cmdlet<EngineError> for Trace {
        fn begin(
            &mut self,
            _: &BoundParameters,
            _: &mut dyn CommandRuntime<EngineError>,
        ) -> Result<(), EngineError> {
            PHASES.with_borrow_mut(|log| log.push("begin"));
            Ok(())
        }
        fn process(
            &mut self,
            args: &BoundParameters,
            _: &mut dyn CommandRuntime<EngineError>,
        ) -> Result<(), EngineError> {
            PHASES.with_borrow_mut(|log| log.push("process"));
            if args.get("InputObject") == Some(&Value::Int32(-1)) {
                return Err(RuntimeError::NotImplemented {
                    feature: "test interruption",
                }
                .into());
            }
            Ok(())
        }
        fn end(&mut self, _: &mut dyn CommandRuntime<EngineError>) -> Result<(), EngineError> {
            PHASES.with_borrow_mut(|log| log.push("end"));
            Ok(())
        }
        fn stop(&mut self, _: &mut dyn CommandRuntime<EngineError>) {
            PHASES.with_borrow_mut(|log| log.push("stop"));
        }
    }

    #[test]
    fn mandatory_null_and_empty_collection_follow_the_declaration() {
        use rpsh_commands::signature::{CmdletParameters as _, Signature};
        #[derive(rpsh_commands::CmdletParameters)]
        #[parameter(InputObject: PsObject, position(0), mandatory, pipeline)]
        struct StrictInput;
        static SIGNATURE: Signature = Signature::new("Test-Input", "Test", StrictInput::PARAMETERS);
        let entry = Entry {
            signature: &SIGNATURE,
            implementation: "Test.Input",
            construct: || Box::new(Trace),
        };
        let param = &SIGNATURE.parameters[0];
        let permitted = Entry::lookup("Write-Output").unwrap();
        for (value, id) in [
            (
                Value::Null,
                "ParameterArgumentValidationErrorNullNotAllowed",
            ),
            (
                Value::array(vec![]),
                "ParameterArgumentValidationErrorEmptyArrayNotAllowed",
            ),
            (
                Value::ObjectList(ArrayList::default()),
                "ParameterArgumentValidationErrorEmptyCollectionNotAllowed",
            ),
        ] {
            let record = mandatory_error(&entry, param, &value).unwrap();
            assert_eq!(
                super::super::member(&record, "FullyQualifiedErrorId")
                    .unwrap()
                    .to_string(),
                format!("{id},Test.Input")
            );
            assert!(
                mandatory_error(permitted, &permitted.signature.parameters[0], &value).is_none()
            );
        }
    }

    #[test]
    fn construct_after_binding_and_stop_processing_on_error() {
        let entry = Entry {
            signature: &WRITE_OUTPUT,
            implementation: "Test.WriteOutput",
            construct: || {
                PHASES.with_borrow_mut(|log| log.push("construct"));
                Box::new(Trace)
            },
        };
        let mut engine = Engine::new();
        let args = PreparedArguments {
            source: &[],
            expanded: vec![],
        };
        let phases = || PHASES.with_borrow_mut(std::mem::take);
        assert!(engine.run_registered(&entry, &args, None).is_err());
        assert!(phases().is_empty());
        engine.run_registered(&entry, &args, Some(&[])).unwrap();
        assert_eq!(phases(), ["construct", "begin", "end"]);
        engine
            .run_registered(&entry, &args, Some(&[Value::Int32(2), Value::Int32(3)]))
            .unwrap();
        assert_eq!(
            phases(),
            ["construct", "begin", "process", "process", "end"]
        );
        assert!(
            engine
                .run_registered(&entry, &args, Some(&[Value::Int32(-1), Value::Int32(5)]))
                .is_err()
        );
        assert_eq!(phases(), ["construct", "begin", "process", "stop"]);
    }
}
