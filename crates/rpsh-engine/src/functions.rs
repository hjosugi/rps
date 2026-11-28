use super::{ArgumentBinding, FunctionDef};
use rpsh_runtime::Value;
use std::cell::LazyCell;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub(super) struct Functions(LazyCell<HashMap<String, FunctionDef>>);

impl Default for Functions {
    fn default() -> Self {
        Self(LazyCell::new(|| {
            if cfg!(unix) {
                let function = FunctionDef {
                    name: "exec".into(),
                    definition: Some("\nSwitch-Process -WithCommand $args\n"),
                    parameters: Vec::new(),
                    begin: Vec::new(),
                    process: Vec::new(),
                    end: Vec::new(),
                    body: Vec::new(),
                    filter: false,
                };
                HashMap::from([("exec".into(), function)])
            } else {
                HashMap::new()
            }
        }))
    }
}

impl Deref for Functions {
    type Target = HashMap<String, FunctionDef>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Functions {
    fn deref_mut(&mut self) -> &mut Self::Target {
        LazyCell::force_mut(&mut self.0)
    }
}

pub(super) const FUNCTION: &str = "System.Management.Automation.FunctionInfo";
pub(super) const CMDLET: &str = "System.Management.Automation.CmdletInfo";

pub(super) fn command_info(name: &str, kind: &str) -> Value {
    let ty = if kind == "Function" { FUNCTION } else { CMDLET };
    Value::object(vec![
        ("Name".into(), Value::String(name.into())),
        ("CommandType".into(), Value::String(kind.into())),
        ("__Text".into(), Value::String(name.into())),
        ("__TypeName".into(), Value::String(ty.into())),
        (
            "__BaseClasses".into(),
            Value::array(vec![Value::String(
                "System.Management.Automation.CommandInfo".into(),
            )]),
        ),
    ])
}

pub(super) fn common_binding(lower: &str) -> Option<ArgumentBinding> {
    match lower {
        "verbose" | "vb" | "debug" | "db" => Some(ArgumentBinding::Switch),
        "erroraction"
        | "ea"
        | "errorvariable"
        | "ev"
        | "warningaction"
        | "wa"
        | "warningvariable"
        | "wv"
        | "informationaction"
        | "infa"
        | "informationvariable"
        | "iv"
        | "progressaction"
        | "proga"
        | "outvariable"
        | "ov"
        | "outbuffer"
        | "ob"
        | "pipelinevariable"
        | "pv" => Some(ArgumentBinding::Named),
        _ => None,
    }
}

pub(super) fn session_info(name: &str, kind: &str, definition: Option<&str>) -> Option<Value> {
    let (ty, kind, module, version, options, definition) = match definition {
        Some(text) => (FUNCTION, "Function", "", Value::Null, "None", text),
        None if cfg!(unix) && kind == "Cmdlet" && name == "Switch-Process" => (
            CMDLET,
            "Cmdlet",
            "Microsoft.PowerShell.Core",
            super::version_value("7.7.0.500".into()),
            "ReadOnly",
            "\nSwitch-Process [[-WithCommand] <string[]>] [<CommonParameters>]\n",
        ),
        None => return None,
    };
    Some(Value::object(vec![
        ("Name".into(), Value::String(name.into())),
        ("__Text".into(), Value::String(name.into())),
        ("__TypeName".into(), Value::String(ty.into())),
        (
            "__BaseClasses".into(),
            Value::array(vec![Value::String(
                "System.Management.Automation.CommandInfo".into(),
            )]),
        ),
        ("CommandType".into(), Value::String(kind.into())),
        ("Source".into(), Value::String(module.into())),
        ("ModuleName".into(), Value::String(module.into())),
        ("Version".into(), version),
        ("Definition".into(), Value::String(definition.into())),
        ("Visibility".into(), Value::String("Public".into())),
        ("Options".into(), Value::String(options.into())),
        (
            "RemotingCapability".into(),
            Value::String("PowerShell".into()),
        ),
    ]))
}
