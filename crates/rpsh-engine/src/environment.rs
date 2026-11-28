//! The Env: drive, System.Environment and native children share this process
//! environment within a shell. Deletions mask inherited values in child scopes.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use rpsh_runtime::{RuntimeError, Value};

type Variables = HashMap<String, Value>;

pub(super) fn drive_name(path: &str) -> Option<&str> {
    let (prefix, name) = path.split_once(':')?;
    prefix.eq_ignore_ascii_case("env").then_some(name)
}

pub(super) fn ps_home() -> &'static Path {
    static HOME_PATH: OnceLock<PathBuf> = OnceLock::new();
    HOME_PATH.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_owned))
            .unwrap_or_default()
    })
}

fn module_path(personal: &Path, shared: &Path, system: &Path, inherited: &str) -> String {
    let mut prepend: Vec<String> = Vec::new();
    let existing: Vec<_> = inherited
        .split(':')
        .map(|part| part.trim().trim_end_matches('/'))
        .collect();
    for path in [personal, shared, system] {
        let path = path.to_string_lossy();
        if !existing.iter().any(|item| item.eq_ignore_ascii_case(&path))
            && (inherited.is_empty()
                || !prepend.iter().any(|item| item.eq_ignore_ascii_case(&path)))
        {
            prepend.push(path.into_owned());
        }
    }
    if !inherited.is_empty() {
        prepend.push(inherited.to_owned());
    }
    prepend.join(":")
}

fn startup() -> &'static [(String, String); 2] {
    static STARTUP: OnceLock<[(String, String); 2]> = OnceLock::new();
    STARTUP.get_or_init(|| {
        let shell = ps_home();
        let path = std::env::var("PATH").unwrap_or_default();
        let prefix = format!("{}:", shell.display());
        let path = if path.starts_with(&prefix) {
            path
        } else {
            prefix + &path
        };
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
            });
        let modules = module_path(
            &data.join("powershell/Modules"),
            Path::new("/usr/local/share/powershell/Modules"),
            &shell.join("Modules"),
            &std::env::var("PSModulePath").unwrap_or_default(),
        );
        [("PATH".into(), path), ("PSModulePath".into(), modules)]
    })
}

pub(super) fn get(variables: &Variables, name: &str) -> Option<Value> {
    let name = name.split('\0').next().unwrap_or("");
    if let Some(value) = variables.get(&format!("env:{name}")) {
        return (!value.is_null_like()).then(|| value.clone());
    }
    if matches!(name, "PATH" | "PSModulePath")
        && let Some((_, value)) = startup().iter().find(|(key, _)| key == name)
    {
        return Some(Value::String(value.as_str().into()));
    }
    std::env::var_os(name).map(|value| Value::String(value.to_string_lossy().as_ref().into()))
}

pub(super) fn set(variables: &mut Variables, name: &str, value: &Value) {
    let value = if value.is_null_like() {
        Value::Null
    } else {
        let text = value.to_string();
        Value::String(text.split('\0').next().unwrap_or("").into())
    };
    variables.insert(format!("env:{name}"), value);
}

pub(super) fn entries(variables: &Variables) -> BTreeMap<String, Value> {
    let mut entries: BTreeMap<_, _> = std::env::vars_os()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                Value::String(value.to_string_lossy().as_ref().into()),
            )
        })
        .collect();
    for (name, value) in startup() {
        entries.insert(name.clone(), Value::String(value.as_str().into()));
    }
    for (key, value) in variables {
        if let Some(name) = key.strip_prefix("env:") {
            if value.is_null_like() {
                entries.remove(name);
            } else {
                entries.insert(name.to_owned(), value.clone());
            }
        }
    }
    entries
}

pub(super) fn search_path(variables: &Variables) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    match variables.get("env:PATH") {
        None => Cow::Borrowed(&startup()[0].1),
        Some(Value::String(s)) => s
            .as_ascii()
            .map_or_else(|| Cow::Owned(s.to_string_lossy()), Cow::Borrowed),
        Some(value) => Cow::Owned(value.to_string()),
    }
}

pub(super) fn configure_child(command: &mut Command, variables: &Variables) {
    for (key, value) in startup() {
        command.env(key, value);
    }
    for (key, value) in variables {
        if let Some(name) = key.strip_prefix("env:") {
            if value.is_null_like() {
                command.env_remove(name);
            } else {
                command.env(name, value.to_string());
            }
        }
    }
}

const TARGET: &str = "System.EnvironmentVariableTarget";
const TARGET_NAMES: [&str; 3] = ["Process", "User", "Machine"];

pub(super) fn target_member(name: &str) -> Option<Value> {
    let (index, canonical) = TARGET_NAMES
        .iter()
        .enumerate()
        .find(|(_, candidate)| candidate.eq_ignore_ascii_case(name))?;
    Some(Value::object(vec![
        ("__TypeName".into(), Value::String(TARGET.into())),
        ("__Text".into(), Value::String((*canonical).into())),
        ("__EnumValue".into(), Value::Int64(index as i64)),
        ("value__".into(), Value::Int32(index as i32)),
    ]))
}

fn process_target(
    target: Option<&Value>,
    method: &str,
    arity: usize,
) -> Result<bool, RuntimeError> {
    let Some(target) = target else {
        return Ok(true);
    };
    if target.is_null_like() {
        return Err(super::overload_error(method, arity));
    }
    let text = target.to_string();
    let numeric = text.parse::<i64>().ok();
    if let Some(index) = numeric
        .and_then(|i| usize::try_from(i).ok())
        .filter(|i| *i < TARGET_NAMES.len())
        .or_else(|| {
            TARGET_NAMES
                .iter()
                .position(|name| name.eq_ignore_ascii_case(&text))
        })
    {
        return Ok(index == 0);
    }
    let detail = if is_null_string(target) {
        format!(
            "Cannot convert the \"\" value of type \"System.Management.Automation.Language.NullString\" to type \"{TARGET}\"."
        )
    } else if numeric.is_some() {
        format!(
            "Cannot convert value \"{text}\" to type \"{TARGET}\" due to enumeration values that are not valid. Specify one of the following enumeration values and try again. The possible enumeration values are \"Process,User,Machine\"."
        )
    } else {
        format!(
            "Cannot convert value \"{text}\" to type \"{TARGET}\". Error: \"Unable to match the identifier name {text} to a valid enumerator name. Specify one of the following enumerator names and try again:\nProcess, User, Machine\""
        )
    };
    Err(RuntimeError::Script {
        id: "MethodArgumentConversionInvalidCastArgument",
        category: "NotSpecified: (:) [], MethodException".into(),
        exception_type: "System.Management.Automation.MethodException",
        message: format!(
            "Cannot convert argument \"target\", with value: \"{text}\", for \"{method}\" to type \"{TARGET}\": \"{detail}\""
        ),
    })
}

fn is_null_string(value: &Value) -> bool {
    super::hidden_type(value).as_deref() == Some("System.Management.Automation.Language.NullString")
}

pub(super) fn table(vars: Vec<(String, Value)>) -> Value {
    Value::object(vec![
        (
            "__TypeName".into(),
            Value::String("System.Collections.Hashtable".into()),
        ),
        ("__EnvironmentTable".into(), Value::object(vars)),
    ])
}

pub(super) fn table_entries(object: &[(String, Value)]) -> Option<&[(String, Value)]> {
    object.iter().find_map(|(key, value)| match value {
        Value::Object(entries) if key == "__EnvironmentTable" => Some(entries.as_slice()),
        _ => None,
    })
}

fn argument_error(method: &str, arity: usize, id: &'static str, message: &str) -> RuntimeError {
    RuntimeError::Script {
        id,
        category: "NotSpecified: (:) [], MethodInvocationException".into(),
        exception_type: "System.Management.Automation.MethodInvocationException",
        message: format!(
            "Exception calling \"{method}\" with \"{arity}\" argument(s): \"{message}\""
        ),
    }
}

pub(super) fn invoke(
    name: &str,
    args: &[Value],
    variables: &mut Variables,
) -> Result<Value, RuntimeError> {
    let method = match (name, args.len()) {
        ("getenvironmentvariable", 1 | 2) => "GetEnvironmentVariable",
        ("setenvironmentvariable", 2 | 3) => "SetEnvironmentVariable",
        ("getenvironmentvariables", 0 | 1) => {
            let vars = if process_target(args.first(), "GetEnvironmentVariables", args.len())? {
                entries(variables).into_iter().collect()
            } else {
                Vec::new()
            };
            return Ok(table(vars));
        }
        ("getenvironmentvariable", _) => {
            return Err(super::overload_error("GetEnvironmentVariable", args.len()));
        }
        ("setenvironmentvariable", _) => {
            return Err(super::overload_error("SetEnvironmentVariable", args.len()));
        }
        _ => return Err(super::overload_error("GetEnvironmentVariables", args.len())),
    };
    if is_null_string(&args[0]) {
        return Err(argument_error(
            method,
            args.len(),
            "ArgumentNullException",
            "Value cannot be null. (Parameter 'variable')",
        ));
    }
    let key = args[0].to_string();
    if method == "GetEnvironmentVariable" {
        return if process_target(args.get(1), method, args.len())? {
            Ok(get(variables, &key).unwrap_or(Value::Null))
        } else {
            Ok(Value::Null)
        };
    }
    let process = process_target(args.get(2), method, args.len())?;
    if key.is_empty() || key.starts_with('\0') {
        return Err(argument_error(
            method,
            args.len(),
            "ArgumentException",
            if key.is_empty() {
                "The value cannot be an empty string. (Parameter 'variable')"
            } else {
                "The first char in the string is the null character. (Parameter 'variable')"
            },
        ));
    }
    if key.contains('=') {
        return Err(argument_error(
            method,
            args.len(),
            "ArgumentException",
            "Environment variable name cannot contain equal character. (Parameter 'variable')",
        ));
    }
    if process {
        let value = if is_null_string(&args[1]) {
            Value::Null
        } else {
            Value::String(args[1].to_ps_string())
        };
        set(variables, key.split('\0').next().unwrap_or(""), &value);
    }
    Ok(Value::AutomationNull)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_paths_keep_existing_entries_and_position() {
        let personal = Path::new("/user");
        let shared = Path::new("/shared");
        let system = Path::new("/shell/Modules");
        assert_eq!(
            module_path(personal, shared, system, "/extra:/SHARED/:/extra"),
            "/user:/shell/Modules:/extra:/SHARED/:/extra"
        );
        assert_eq!(
            module_path(shared, shared, system, "/extra"),
            "/shared:/shell/Modules:/extra"
        );
        assert_eq!(
            module_path(shared, shared, system, ""),
            "/shared:/shared:/shell/Modules"
        );
    }
}
