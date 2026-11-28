//! Binding of the public System.Management.Automation wildcard types.
use rpsh_runtime::{PsString, RuntimeError, Value, wildcard};

use super::{dotnet_string_argument, hidden_type, overload_error};

const PATTERN_TYPE: &str = "System.Management.Automation.WildcardPattern";
const OPTIONS_TYPE: &str = "System.Management.Automation.WildcardOptions";
const FLAGS: &[(&str, i32)] = &[
    ("None", 0),
    ("Compiled", 1),
    ("IgnoreCase", 2),
    ("CultureInvariant", 4),
];

#[derive(Clone, Copy)]
pub(super) enum Command {
    GetAlias,
    GetCommand,
}

impl Command {
    pub(super) fn patterns(
        self,
        patterns: &[String],
    ) -> Result<Vec<wildcard::Matcher>, RuntimeError> {
        let (name, id) = match self {
            Self::GetAlias => (
                "Get-Alias",
                "RuntimeException,Microsoft.PowerShell.Commands.GetAliasCommand",
            ),
            Self::GetCommand => (
                "Get-Command",
                "RuntimeException,Microsoft.PowerShell.Commands.GetCommandCommand",
            ),
        };
        patterns
            .iter()
            .map(|pattern| {
                wildcard::Matcher::new(&pattern.as_str().into(), 2).map_err(|error| match error {
                    RuntimeError::Script {
                        id: "WildcardPatternException",
                        exception_type,
                        message,
                        ..
                    } => RuntimeError::Script {
                        id,
                        category: format!("NotSpecified: (:) [{name}], WildcardPatternException"),
                        exception_type,
                        message,
                    },
                    other => other,
                })
            })
            .collect()
    }
}

pub(super) fn matches_any(
    patterns: &[wildcard::Matcher],
    text: &str,
) -> Result<bool, RuntimeError> {
    let text = PsString::from(text);
    for pattern in patterns {
        if pattern.is_match(Some(&text))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn invocation_error(method: &str, arity: usize, error: RuntimeError) -> RuntimeError {
    match error {
        RuntimeError::Script { id, message, .. } => RuntimeError::Script {
            id,
            category: "NotSpecified: (:) [], MethodInvocationException".into(),
            exception_type: "System.Management.Automation.MethodInvocationException",
            message: format!(
                "Exception calling \"{method}\" with \"{arity}\" argument(s): \"{message}\""
            ),
        },
        other => other,
    }
}

fn flag(name: &str) -> Option<i32> {
    FLAGS
        .iter()
        .find(|(candidate, _)| name.eq_ignore_ascii_case(candidate))
        .map(|(_, bit)| *bit)
}

pub(super) fn option_member(name: &str) -> Result<Value, RuntimeError> {
    flag(name)
        .map(options_value)
        .ok_or(RuntimeError::NotImplemented {
            feature: "unknown WildcardOptions member",
        })
}

pub(super) fn options_value(bits: i32) -> Value {
    let label = if bits == 0 {
        "None".into()
    } else if bits & !7 != 0 {
        bits.to_string()
    } else {
        FLAGS
            .iter()
            .filter(|(_, bit)| bits & bit != 0)
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    Value::object(vec![
        ("__TypeName".into(), Value::String(OPTIONS_TYPE.into())),
        ("__Text".into(), Value::String(label.into())),
        ("__EnumValue".into(), Value::Int64(i64::from(bits))),
        ("value__".into(), Value::Int32(bits)),
    ])
}

pub(super) fn typed_options(value: &Value) -> Option<i32> {
    let Value::Object(entries) = value else {
        return None;
    };
    if hidden_type(value).as_deref() != Some(OPTIONS_TYPE) {
        return None;
    }
    entries.iter().find_map(|(key, value)| match value {
        Value::Int32(bits) if key == "value__" => Some(*bits),
        _ => None,
    })
}

fn parse_options(value: &Value, method: &str) -> Result<i32, RuntimeError> {
    if let Some(bits) = typed_options(value) {
        return Ok(bits);
    }
    let text = value.to_string();
    let numeric = text.parse::<i32>().ok();
    if let Some(bits) = numeric.filter(|bits| (0..=7).contains(bits)) {
        return Ok(bits);
    }
    if let Some(bits) = text
        .split(',')
        .try_fold(0, |bits, name| flag(name.trim()).map(|bit| bits | bit))
    {
        return Ok(bits);
    }
    let detail = if numeric.is_some() || value.is_null_like() {
        let source = if value.is_null_like() {
            "null".into()
        } else {
            format!("value \"{text}\"")
        };
        format!(
            "Cannot convert {source} to type \"{OPTIONS_TYPE}\" due to enumeration values that are not valid. Specify one of the following enumeration values and try again. The possible enumeration values are \"None,Compiled,IgnoreCase,CultureInvariant\"."
        )
    } else {
        format!(
            "Cannot convert value \"{text}\" to type \"{OPTIONS_TYPE}\". Error: \"Unable to match the identifier name {text} to a valid enumerator name. Specify one of the following enumerator names and try again:\nNone, Compiled, IgnoreCase, CultureInvariant\""
        )
    };
    Err(RuntimeError::Script {
        id: "MethodArgumentConversionInvalidCastArgument",
        category: "NotSpecified: (:) [], MethodException".into(),
        exception_type: "System.Management.Automation.MethodException",
        message: format!(
            "Cannot convert argument \"options\", with value: \"{text}\", for \"{method}\" to type \"{OPTIONS_TYPE}\": \"{detail}\""
        ),
    })
}

pub(super) fn static_method(name: &str, arguments: &[Value]) -> Result<Value, RuntimeError> {
    match (name, arguments) {
        ("escape" | "unescape", [value]) => {
            let pattern = dotnet_string_argument(value);
            let (method, result) = if name == "escape" {
                ("Escape", wildcard::escape(pattern.as_ref(), &[]))
            } else {
                ("Unescape", wildcard::unescape(pattern.as_ref()))
            };
            result
                .map(Value::String)
                .map_err(|err| invocation_error(method, 1, err))
        }
        ("containswildcardcharacters", [value]) => Ok(Value::Bool(
            dotnet_string_argument(value)
                .is_some_and(|pattern| wildcard::contains_wildcard_characters(pattern.units())),
        )),
        ("new", [first] | [first, _]) | ("get", [first, _]) => {
            let method = if name == "new" { ".ctor" } else { "Get" };
            let pattern = dotnet_string_argument(first);
            let options = match arguments.get(1) {
                Some(value) if name == "new" && value.is_null_like() => {
                    return Err(overload_error(name, arguments.len()));
                }
                Some(value) => parse_options(value, method)?,
                None => 0,
            };
            let pattern = wildcard::require_pattern(pattern.as_ref())
                .map_err(|err| invocation_error(method, arguments.len(), err))?;
            Ok(Value::object(vec![
                ("__TypeName".into(), Value::String(PATTERN_TYPE.into())),
                ("__Text".into(), Value::String(PATTERN_TYPE.into())),
                ("__WildcardPattern".into(), Value::String(pattern.clone())),
                ("__WildcardOptions".into(), Value::Int32(options)),
            ]))
        }
        _ => Err(overload_error(name, arguments.len())),
    }
}

pub(super) fn pattern_method(
    entries: &[(String, Value)],
    name: &str,
    arguments: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    let (pattern, bits) = wildcard::stored(entries)?;
    match (name.to_ascii_lowercase().as_str(), arguments) {
        ("ismatch", [input]) => {
            let input = dotnet_string_argument(input);
            Some(
                wildcard::Matcher::new(pattern, bits)
                    .and_then(|matcher| matcher.is_match(input.as_ref()))
                    .map(Value::Bool)
                    .map_err(|err| invocation_error("IsMatch", 1, err)),
            )
        }
        ("ismatch", _) => Some(Err(overload_error("IsMatch", arguments.len()))),
        ("towql", []) => Some(
            wildcard::to_wql(pattern)
                .map(Value::String)
                .map_err(|err| invocation_error("ToWql", 0, err)),
        ),
        ("tostring", []) => Some(Ok(Value::String(PATTERN_TYPE.into()))),
        _ => None,
    }
}
