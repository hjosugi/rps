//! Bind the two public InvokeWithContext context arguments. A caller-supplied
//! List<PSVariable> is shared: PowerShell removes this, _, and input from it.
use super::{
    EngineError, FunctionDef, hidden_type, is_hidden, member, metadata, method_error, ps_variable,
    script_methods,
};
use rpsh_runtime::{ArrayList, Value};
use std::collections::HashSet;

const METHOD: &str = "InvokeWithContext";
const INVALID: &str = "System.Management.Automation.PSInvalidOperationException";
const HASHTABLE: &str = "System.Collections.Hashtable";
const ORDERED: &str = "System.Collections.Specialized.OrderedDictionary";

#[derive(Default)]
pub(super) struct Context {
    pub functions: Vec<FunctionDef>,
    pub variables: Vec<(String, Value)>,
    pub automatic: Vec<(&'static str, Value)>,
}

fn failure(count: usize, ty: &str, message: &str) -> EngineError {
    script_methods::wrapped(
        METHOD,
        count,
        Value::object(vec![
            ("__TypeName".into(), Value::String(ty.into())),
            ("Message".into(), Value::String(message.into())),
        ]),
    )
}

fn bad_list(v: &Value, detail: &str) -> EngineError {
    let displayed = match v {
        Value::Object(_) if matches!(hidden_type(v).as_deref(), Some(HASHTABLE | ORDERED)) => {
            hidden_type(v).unwrap()
        }
        Value::Array(_) | Value::StringArray(_) => super::get_type(v).to_string(),
        _ => v.to_string(),
    };
    let runtime = method_error::argument_error(
        METHOD,
        "variablesToDefine",
        metadata::VARIABLE_LIST.display_name,
        &displayed,
        detail,
    );
    runtime
        .caused_by(
            "System.Management.Automation.PSInvalidCastException",
            detail,
        )
        .into()
}

fn variables(source: &Value) -> Result<ArrayList, EngineError> {
    if let Value::Object(fields) = source
        && matches!(hidden_type(source).as_deref(), Some(HASHTABLE | ORDERED))
    {
        let result = ArrayList::default();
        for (name, v) in fields.iter().filter(|(k, _)| !is_hidden(k)) {
            if name.eq_ignore_ascii_case("Capacity") {
                super::array_list::set_capacity(super::array_list::Kind::ObjectList, &result, v)?;
            } else {
                let ty = &metadata::VARIABLE_LIST;
                let detail = format!(
                    "Cannot create object of type \"{}\". The property '{name}' was not found for the '{}' object. The settable properties are: [Capacity <System.Int32>].",
                    ty.display_name, ty.full_name
                );
                return Err(bad_list(source, &detail));
            }
        }
        return Ok(result);
    }
    match ps_variable::list_cast(source).map_err(|e| bad_list(source, &e.to_string()))? {
        Value::Collection(c) => Ok(c.items.clone()),
        _ => Ok(ArrayList::default()),
    }
}

impl Context {
    pub fn bind(args: &[Value]) -> Result<Self, EngineError> {
        let [funcs, vars, ..] = args else {
            return Err(method_error::overload_error(METHOD, args.len()).into());
        };
        let count = args.len();
        if !(funcs.is_null_like()
            || matches!(hidden_type(funcs).as_deref(), Some(HASHTABLE | ORDERED)))
        {
            return Err(method_error::overload_error(METHOD, count).into());
        }
        let list = variables(vars)?;
        let mut definitions = Vec::new();
        if let Value::Object(fields) = funcs {
            let nonstring = fields.iter().any(|(name, _)| name == "__NumericKeys");
            for (name, v) in fields.iter().filter(|(name, _)| !is_hidden(name)) {
                if nonstring || name.trim().is_empty() {
                    return Err(failure(
                        count,
                        INVALID,
                        rpsh_messages::text(
                            "ParserStrings/EmptyFunctionNameInFunctionDefinitionDictionary",
                        ),
                    ));
                }
                definitions.push((name, v));
            }
        }
        let mut result = Self::default();
        let mut items = list.items_mut();
        for special in ["this", "_", "input"] {
            let mut at = None;
            for (index, obj) in items.iter().enumerate() {
                if obj.is_null_like() {
                    return Err(failure(
                        count,
                        "System.NullReferenceException",
                        "Object reference not set to an instance of an object.",
                    ));
                }
                if member(obj, "Name")?
                    .to_string()
                    .eq_ignore_ascii_case(special)
                {
                    at = Some(index);
                    break;
                }
            }
            if let Some(at) = at {
                result
                    .automatic
                    .push((special, member(&items.remove(at), "Value")?));
            }
        }
        for (name, v) in definitions {
            let Value::ScriptBlock(block) = v else {
                return Err(failure(
                    count,
                    INVALID,
                    &rpsh_messages::format(
                        "ParserStrings/NullFunctionBodyInFunctionDefinitionDictionary",
                        &[name],
                    ),
                ));
            };
            let mut function = FunctionDef::from_block_value(block)?;
            function.name = name.to_owned();
            result.functions.push(function);
        }
        let mut names = HashSet::new();
        for obj in items.iter() {
            let name = member(obj, "Name")?.to_string();
            if !names.insert(name.to_ascii_lowercase()) {
                return Err(failure(
                    count,
                    "System.ArgumentException",
                    &format!("An item with the same key has already been added. Key: {name}"),
                ));
            }
            result.variables.push((name, obj.clone()));
        }
        Ok(result)
    }
}
