//! Function provider operations use the same scoped table as command discovery.
//! Each item has one content record, a ScriptBlock, independent of its text.
use super::{
    ArgumentBinding, Engine, EngineError, FunctionDef, ScriptException, convert_int32,
    errors::record_with_exception, functions,
};
use rpsh_runtime::{RuntimeError, Value, ops};

pub(super) fn name(path: &str) -> Option<&str> {
    let (drive, name) = path.split_once(':')?;
    drive.eq_ignore_ascii_case("function").then(|| {
        super::startup::Stage::Providers.enter();
        name.trim_start_matches(['/', '\\'])
    })
}

pub(super) fn option<'a>(named: &'a [(String, Value)], names: &[&str]) -> Option<&'a Value> {
    named.iter().find_map(|(key, value)| {
        names
            .iter()
            .any(|name| key.eq_ignore_ascii_case(name))
            .then_some(value)
    })
}

pub(super) fn switch(named: &[(String, Value)], name: &str) -> bool {
    option(named, &[name]).is_some_and(ops::is_true)
}

pub(super) fn binding(key: &str) -> ArgumentBinding {
    let lower = key.to_ascii_lowercase();
    match lower.as_str() {
        "force" | "passthru" | "raw" | "asbytestream" | "nonewline" | "wait" | "recurse"
        | "whatif" | "confirm" => ArgumentBinding::Switch,
        other => functions::common_binding(other).unwrap_or(ArgumentBinding::Named),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Command {
    Variable,
    SetItem,
    NewItem,
    ClearItem,
    RemoveItem,
    GetContent,
    SetContent,
    AddContent,
    ClearContent,
}

impl Command {
    fn label(self) -> &'static str {
        match self {
            Self::Variable => "",
            Self::SetItem => "Set-Item",
            Self::NewItem => "New-Item",
            Self::ClearItem => "Clear-Item",
            Self::RemoveItem => "Remove-Item",
            Self::GetContent => "Get-Content",
            Self::SetContent => "Set-Content",
            Self::AddContent => "Add-Content",
            Self::ClearContent => "Clear-Content",
        }
    }

    fn error(
        self,
        id: &str,
        category: &str,
        ty: &str,
        target: Value,
        key: &str,
        args: &[&str],
    ) -> EngineError {
        let label = self.label();
        let id = if label.is_empty() {
            id.to_owned()
        } else {
            format!(
                "{id},Microsoft.PowerShell.Commands.{}Command",
                label.replace('-', "")
            )
        };
        let subject = if target.is_null_like() {
            ":".into()
        } else {
            format!(
                "{target}:{}",
                target.type_name().rsplit('.').next().unwrap_or_default()
            )
        };
        let message = rpsh_messages::format(key, args);
        let exception = Value::object(vec![
            (
                "__TypeName".into(),
                Value::String(if ty == "InvalidOperationException" {
                    "System.InvalidOperationException".into()
                } else {
                    format!("System.Management.Automation.{ty}").into()
                }),
            ),
            ("Message".into(), Value::String(message.as_str().into())),
            ("InnerException".into(), Value::Null),
        ]);
        EngineError::Script(ScriptException::new(record_with_exception(
            &id,
            &format!("{category}: ({subject}) [{label}], {ty}"),
            exception,
            &message,
            target,
        )))
    }

    fn argument(self, parameter: &str) -> EngineError {
        self.error(
            "Argument",
            "InvalidArgument",
            "PSArgumentException",
            Value::Null,
            "AutomationExceptions/Argument",
            &[parameter],
        )
    }

    pub(super) fn missing(self, name: &str) -> EngineError {
        let path = format!("Function:{}{name}", std::path::MAIN_SEPARATOR);
        self.error(
            "PathNotFound",
            "ObjectNotFound",
            "ItemNotFoundException",
            Value::String(path.as_str().into()),
            "SessionStateStrings/PathNotFound",
            &[&path],
        )
    }

    fn root(self) -> EngineError {
        let (id, arg, null) = match self {
            Self::ClearItem => ("ClearItemNullPath", "path", true),
            Self::SetItem => ("SetItemNullName", "name", true),
            Self::NewItem => ("NewItemNullPath", "path", false),
            _ => return self.argument("path"),
        };
        let (ty, key) = if null {
            (
                "PSArgumentNullException",
                "AutomationExceptions/ArgumentNull",
            )
        } else {
            ("PSArgumentException", "AutomationExceptions/Argument")
        };
        self.error(
            id,
            "InvalidArgument",
            ty,
            Value::String("".into()),
            key,
            &[arg],
        )
    }

    pub(super) fn unsupported_content(self) -> EngineError {
        let key = if self == Self::ClearContent {
            "SessionStateStrings/IContent_Clear_NotSupported"
        } else {
            "SessionStateStrings/IContent_Seek_NotSupported"
        };
        self.error(
            "NotSupported",
            "NotImplemented",
            "PSNotSupportedException",
            Value::Null,
            key,
            &[],
        )
    }

    pub(super) fn check_parameters(self, named: &[(String, Value)]) -> Result<(), EngineError> {
        for (param, _) in named {
            let lower = param.to_ascii_lowercase();
            let invalid = match self {
                Self::GetContent | Self::SetContent | Self::AddContent | Self::ClearContent => {
                    matches!(
                        lower.as_str(),
                        "raw"
                            | "asbytestream"
                            | "delimiter"
                            | "encoding"
                            | "nonewline"
                            | "wait"
                            | "stream"
                    )
                }
                Self::ClearItem => lower == "passthru",
                _ => false,
            };
            if invalid {
                return Err(self.error(
                    "NamedParameterNotFound",
                    "InvalidArgument",
                    "ParameterBindingException",
                    Value::Null,
                    "ParameterBinderStrings/NamedParameterNotFound",
                    &["", param],
                ));
            }
        }
        Ok(())
    }

    pub(super) fn extra_positional(self, value: &Value) -> EngineError {
        self.error(
            "PositionalParameterNotFound",
            "InvalidArgument",
            "ParameterBindingException",
            Value::Null,
            "ParameterBinderStrings/PositionalParameterNotFound",
            &["", &value.to_string()],
        )
    }
}

impl Engine {
    /// ShouldProcess messages go to the host even inside an assignment or pipeline.
    pub(super) fn drive_should_process(
        &mut self,
        op: Command,
        name: &str,
        value: &Value,
        named: &[(String, Value)],
    ) -> Result<bool, EngineError> {
        let what_if = option(named, &["WhatIf"])
            .cloned()
            .or_else(|| self.variables.get("WhatIfPreference"))
            .as_ref()
            .is_some_and(ops::is_true);
        if !what_if {
            if switch(named, "Confirm") {
                if self.non_interactive {
                    return Err(op.error(
                        "InvalidOperation",
                        "InvalidOperation",
                        "PSInvalidOperationException",
                        Value::Null,
                        "ConsoleHostUserInterfaceStrings/ReadFailsOnNonInteractiveFlag",
                        &[],
                    ));
                }
                return Err(RuntimeError::NotImplemented {
                    feature: "interactive Function provider confirmation",
                }
                .into());
            }
            return Ok(true);
        }
        let (action, target) = match op {
            Command::SetItem => (
                "Set Item",
                rpsh_messages::format(
                    "SessionStateProviderBaseStrings/SetItemResourceTemplate",
                    &[name, &value.to_string()],
                ),
            ),
            Command::NewItem => {
                let ty = option(named, &["ItemType", "Type"])
                    .map(ToString::to_string)
                    .unwrap_or_default();
                (
                    "New Item",
                    rpsh_messages::format(
                        "SessionStateProviderBaseStrings/NewItemResourceTemplate",
                        &[name, &ty, &value.to_string()],
                    ),
                )
            }
            Command::ClearItem | Command::RemoveItem => (
                if op == Command::ClearItem {
                    "Clear Item"
                } else {
                    "Remove Item"
                },
                rpsh_messages::format(
                    "SessionStateProviderBaseStrings/ClearItemResourceTemplate",
                    &[name],
                ),
            ),
            Command::AddContent | Command::SetContent => (
                if op == Command::SetContent {
                    "Set Content"
                } else {
                    "Add Content"
                },
                rpsh_messages::format(
                    "NavigationResources/SetContentTarget",
                    &[&format!("Function:{}{name}", std::path::MAIN_SEPARATOR)],
                ),
            ),
            _ => return Ok(true),
        };
        let text = rpsh_messages::format(
            "CommandBaseStrings/ShouldProcessMessage",
            &[action, &target],
        );
        self.host_notices.push(rpsh_messages::format(
            "CommandBaseStrings/ShouldProcessWhatIfMessage",
            &[&text],
        ));
        Ok(false)
    }

    pub(super) fn provider_input<'a>(
        &mut self,
        supplied: Option<&'a Value>,
        input: &'a [Value],
        op: Command,
        named: &[(String, Value)],
    ) -> Result<&'a [Value], EngineError> {
        if !input.is_empty() {
            if supplied.is_none() {
                return Ok(input);
            }
            for value in input {
                let result: Result<(), _> = Err(op.error(
                    "InputObjectNotBound",
                    "InvalidArgument",
                    "ParameterBindingException",
                    value.clone(),
                    "ParameterBinderStrings/InputObjectNotBound",
                    &[],
                ));
                self.cmdlet_result(result, named)?;
            }
            return Ok(&[]);
        }
        Ok(std::slice::from_ref(supplied.unwrap_or(&Value::Null)))
    }

    pub(super) fn function_names(&self, name: &str, literal: bool) -> Vec<String> {
        if literal || !name.contains(['*', '?', '[']) {
            return vec![name.into()];
        }
        let mut names: Vec<_> = self
            .functions
            .values()
            .filter(|f| ops::wildcard_match(&f.name, name, false))
            .map(|f| f.name.clone())
            .collect();
        names.sort_by_cached_key(|n| n.to_lowercase());
        names
    }

    pub(super) fn set_drive_function(
        &mut self,
        name: &str,
        value: &Value,
        op: Command,
        named: &[(String, Value)],
    ) -> Result<Option<&FunctionDef>, EngineError> {
        if name.is_empty() {
            return Err(op.root());
        }
        let key = name.to_ascii_lowercase();
        if op == Command::NewItem {
            if value.is_null_like() {
                return Err(op.error(
                    "NewItemValueNotSpecified",
                    "InvalidArgument",
                    "PSArgumentNullException",
                    Value::String(name.into()),
                    "AutomationExceptions/ArgumentNull",
                    &["value"],
                ));
            }
            if !switch(named, "Force") && self.functions.contains_key(&key) {
                return Err(op.error(
                    "Argument",
                    "InvalidArgument",
                    "PSArgumentException",
                    Value::Null,
                    "SessionStateStrings/NewItemAlreadyExists",
                    &[name],
                ));
            }
        }
        if matches!(op, Command::NewItem | Command::SetItem) {
            if !self.drive_should_process(op, name, value, named)? {
                return Ok(None);
            }
            if option(named, &["Options"]).is_some() {
                return Err(RuntimeError::NotImplemented {
                    feature: "setting Function provider item options",
                }
                .into());
            }
        }
        // Drive variable writes pass through IContentWriter.Write(IList).
        // Set-Item instead receives the bound object without unwrapping.
        let items = (op == Command::Variable)
            .then(|| value.sequence())
            .flatten();
        let value = match items.as_deref() {
            Some([only]) => only,
            _ => value,
        };
        if value.is_null_like() {
            self.functions.remove(&key);
            return Ok(None);
        }
        let mut function = match value {
            Value::ScriptBlock(block) => FunctionDef::from_block_value(block)?,
            Value::String(source) => FunctionDef::from_source(&source.to_string_lossy())?,
            value => FunctionDef::from_info(value)?.ok_or_else(|| op.argument("value"))?,
        };
        function.name = name.into();
        Ok(Some(
            self.functions
                .set_item(function, matches!(value, Value::Object(_))),
        ))
    }

    pub(super) fn clear_drive_function(
        &mut self,
        name: &str,
        op: Command,
        named: &[(String, Value)],
    ) -> Result<(), EngineError> {
        if name.is_empty() {
            return Err(op.root());
        }
        let key = name.to_ascii_lowercase();
        if !self.functions.contains_key(&key) {
            return Err(op.missing(name));
        }
        if op == Command::ClearContent {
            return Err(op.unsupported_content());
        }
        if self.drive_should_process(op, name, &Value::Null, named)? {
            self.functions.remove(&key);
        }
        Ok(())
    }

    pub(super) fn read_drive_function(
        &self,
        name: &str,
        named: &[(String, Value)],
    ) -> Result<Option<Value>, EngineError> {
        let op = Command::GetContent;
        if name.is_empty() {
            return Err(op.root());
        }
        let f = self
            .functions
            .get(&name.to_ascii_lowercase())
            .ok_or_else(|| op.missing(name))?;
        if let Some(count) = option(named, &["Tail", "Last"]) {
            let count = convert_int32(count)?;
            return Err(op.error(
                "TailNotSupported",
                "InvalidOperation",
                "InvalidOperationException",
                Value::Int32(count),
                "SessionStateStrings/GetContent_TailNotSupported",
                &[],
            ));
        }
        if option(named, &["TotalCount", "First", "Head"])
            .map(convert_int32)
            .transpose()?
            == Some(0)
        {
            return Ok(None);
        }
        let value = f.script_block();
        if option(named, &["ReadCount"])
            .map(convert_int32)
            .transpose()?
            .unwrap_or(1)
            == 1
        {
            Ok(Some(value))
        } else {
            Ok(Some(Value::array(vec![value])))
        }
    }
}
