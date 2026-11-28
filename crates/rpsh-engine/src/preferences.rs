//! Typed action preferences and the opt-in native exit error policy.
use super::{enum_table::EnumTable, variables::Variables};
use rpsh_runtime::{RuntimeError, Value, ops};

pub(super) const TYPE: &str = "System.Management.Automation.ActionPreference";
const NATIVE: &str = "PSNativeCommandUseErrorActionPreference";
const PASSING: &str = "PSNativeCommandArgumentPassing";
pub(super) const ARGUMENT_TYPE: &str = "System.Management.Automation.NativeArgumentPassingStyle";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ArgumentStyle {
    Legacy,
    Standard,
    Windows,
}

const STYLES: EnumTable<ArgumentStyle> = EnumTable {
    ty: ARGUMENT_TYPE,
    entries: &[
        ("Legacy", 0, ArgumentStyle::Legacy),
        ("Standard", 1, ArgumentStyle::Standard),
        ("Windows", 2, ArgumentStyle::Windows),
    ],
};

pub(super) fn argument_style(variables: &Variables) -> ArgumentStyle {
    variables
        .get(PASSING)
        .and_then(|value| STYLES.parse(&value).ok())
        .map_or_else(
            || STYLES.entries[if cfg!(windows) { 2 } else { 1 }].2,
            |entry| entry.2,
        )
}

pub(super) fn argument_member(name: &str) -> Option<Value> {
    STYLES.entry(name).map(|entry| STYLES.value(entry))
}

pub(super) fn argument_cast(value: &Value) -> Result<Value, RuntimeError> {
    STYLES.cast(value)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Silent,
    Stop,
    Continue,
    Inquire,
    Ignore,
    Suspend,
    Break,
}
const TABLE: EnumTable<Action> = EnumTable {
    ty: TYPE,
    entries: &[
        ("SilentlyContinue", 0, Action::Silent),
        ("Stop", 1, Action::Stop),
        ("Continue", 2, Action::Continue),
        ("Inquire", 3, Action::Inquire),
        ("Ignore", 4, Action::Ignore),
        ("Suspend", 5, Action::Suspend),
        ("Break", 6, Action::Break),
    ],
};
const VARIABLES: &[(&str, Action)] = &[
    ("ErrorActionPreference", Action::Continue),
    ("WarningPreference", Action::Continue),
    ("ProgressPreference", Action::Continue),
    ("VerbosePreference", Action::Silent),
    ("DebugPreference", Action::Silent),
    ("InformationPreference", Action::Silent),
];

pub(super) fn variable_name(key: &str) -> Option<&'static str> {
    if key.eq_ignore_ascii_case(NATIVE) {
        return Some(NATIVE);
    }
    if key.eq_ignore_ascii_case(PASSING) {
        return Some(PASSING);
    }
    VARIABLES
        .iter()
        .find(|(k, _)| key.eq_ignore_ascii_case(k))
        .map(|(k, _)| *k)
}

pub(super) fn names() -> impl Iterator<Item = &'static str> {
    [NATIVE, PASSING]
        .into_iter()
        .chain(VARIABLES.iter().map(|(name, _)| *name))
}

pub(super) fn variable_info(name: &str, value: Value) -> Value {
    let description = if name == PASSING {
        "NativeCommandArgumentPassing"
    } else {
        name
    };
    let mut variable = super::ps_variable::make(name, value, false);
    let Value::Object(fields) = &mut variable else {
        unreachable!()
    };
    if let Some((_, slot)) = std::rc::Rc::make_mut(fields)
        .iter_mut()
        .find(|(k, _)| k == "Description")
    {
        *slot = Value::String(
            rpsh_messages::text(&format!("RunspaceInit/{description}Description")).into(),
        );
    }
    variable
}

pub(super) fn default_value(key: &str) -> Option<Value> {
    if key == NATIVE {
        return Some(Value::Bool(false));
    }
    if key == PASSING {
        return Some(STYLES.value(&STYLES.entries[if cfg!(windows) { 2 } else { 1 }]));
    }
    VARIABLES
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, action)| TABLE.value(&TABLE.entries[*action as usize]))
}

pub(super) fn member(key: &str) -> Option<Value> {
    TABLE.entry(key).map(|entry| TABLE.value(entry))
}
pub(super) fn cast(value: &Value) -> Result<Value, RuntimeError> {
    TABLE.cast(value)
}

fn metadata_error(message: String) -> RuntimeError {
    RuntimeError::Script {
        id: "RuntimeException",
        category: "MetadataError: (:) [], ArgumentTransformationMetadataException".into(),
        exception_type: "System.Management.Automation.ArgumentTransformationMetadataException",
        message,
    }
}

pub(super) fn assignment(name: &str, value: &Value) -> Result<Value, RuntimeError> {
    if name == PASSING {
        return STYLES
            .parse(value)
            .map(|entry| STYLES.value(entry))
            .map_err(|(_, detail)| metadata_error(detail));
    }
    if name == NATIVE {
        return match value {
            Value::Bool(_)
            | Value::Int32(_)
            | Value::Int64(_)
            | Value::UInt32(_)
            | Value::UInt64(_)
            | Value::Int16(_)
            | Value::UInt16(_)
            | Value::Byte(_)
            | Value::SByte(_)
            | Value::Double(_) => Ok(Value::Bool(ops::is_true(value))),
            Value::Decimal(_) => Ok(Value::Bool(super::convert_double(value)? != 0.0)),
            _ => {
                let ty = if value.is_null_like() {
                    ""
                } else {
                    value.type_name()
                };
                Err(metadata_error(format!(
                    "Cannot convert value \"{ty}\" to type \"System.Boolean\". Boolean parameters accept only Boolean values and numbers, such as $True, $False, 1 or 0."
                )))
            }
        };
    }
    let entry = TABLE
        .parse(value)
        .map_err(|(_, detail)| metadata_error(detail))?;
    if entry.2 == Action::Suspend {
        return Err(metadata_error("The Suspend ActionPreference value is reserved for future use and is not supported at this time. For more information about preference variables, see the Help topic, \"about_Preference_Variables.\"".into()));
    }
    Ok(TABLE.value(entry))
}

pub(super) fn native_action(variables: &Variables) -> Option<Action> {
    if !matches!(variables.get(NATIVE), Some(Value::Bool(true))) {
        return None;
    }
    Some(
        variables
            .get("ErrorActionPreference")
            .and_then(|v| TABLE.parse(&v).ok())
            .map_or(Action::Continue, |entry| entry.2),
    )
}
