//! PSVariable objects passed to ScriptBlock.InvokeWithContext or returned by the variable cmdlets.
use super::{enum_table::EnumTable, errors, get_type, hidden_type, metadata, method_error};
use rpsh_runtime::{ArrayList, CollectionType, RuntimeError, Value};

pub(super) const TYPE: &str = "System.Management.Automation.PSVariable";
pub(super) const LOCAL: &str = "System.Management.Automation.LocalVariable";
const OPTIONS: EnumTable<()> = EnumTable {
    ty: "System.Management.Automation.ScopedItemOptions",
    entries: &[
        ("None", 0, ()),
        ("ReadOnly", 1, ()),
        ("Constant", 2, ()),
        ("Private", 4, ()),
        ("AllScope", 8, ()),
        ("Unspecified", 16, ()),
    ],
};
const ATTR_TYPE: &str = "System.Management.Automation.PSVariableAttributeCollection";
static ATTRIBUTES: CollectionType = CollectionType {
    full_name: ATTR_TYPE,
    display_name: ATTR_TYPE,
    element: "System.Attribute",
    read_only: false,
    backing: Some(&metadata::ATTRIBUTE_LIST),
};

pub(super) fn is_type(name: &str) -> bool {
    matches!(
        name.strip_prefix("system.").unwrap_or(name),
        "psvariable" | "management.automation.psvariable"
    )
}

pub(super) fn is_list(name: &str) -> bool {
    name.strip_prefix("system.")
        .unwrap_or(name)
        .strip_prefix("collections.generic.list[")
        .and_then(|n| n.strip_suffix(']'))
        .is_some_and(is_type)
}

fn string(s: &str) -> Value {
    Value::String(s.into())
}

pub(super) fn is_instance(value: &Value) -> bool {
    matches!(hidden_type(value).as_deref(), Some(TYPE | LOCAL))
}

pub(super) fn make(name: &str, value: Value, local: bool) -> Value {
    let ty = if local { LOCAL } else { TYPE };
    let mut fields = vec![
        ("__TypeName".into(), string(ty)),
        ("__Text".into(), string(ty)),
        (
            "__VariableValue".into(),
            Value::ArrayList(ArrayList::from_values(vec![value])),
        ),
        ("Name".into(), string(name)),
        ("Description".into(), string("")),
        ("Value".into(), Value::Null),
        (
            "Visibility".into(),
            errors::enum_value(
                "System.Management.Automation.SessionStateEntryVisibility",
                "Public",
                &["Public", "Private"],
            ),
        ),
        ("Module".into(), Value::Null),
        ("ModuleName".into(), string("")),
        ("Options".into(), OPTIONS.value(&OPTIONS.entries[0])),
        (
            "Attributes".into(),
            Value::collection(&ATTRIBUTES, Vec::new()),
        ),
    ];
    if local {
        fields.push(("__BaseClasses".into(), Value::array(vec![string(TYPE)])));
    }
    Value::object(fields)
}

fn storage(object: &Value) -> Option<&ArrayList> {
    let Value::Object(fields) = object else {
        return None;
    };
    fields.iter().find_map(|(key, v)| match v {
        Value::ArrayList(list) if key == "__VariableValue" => Some(list),
        _ => None,
    })
}

pub(super) fn value(object: &Value) -> Option<Value> {
    storage(object).and_then(|s| s.items().first().cloned())
}

pub(super) fn replace(object: &Value, value: Value) -> Option<Value> {
    storage(object).and_then(|s| {
        s.items_mut()
            .first_mut()
            .map(|slot| std::mem::replace(slot, value))
    })
}

pub(super) fn set_value(object: &Value, value: Value) -> Result<(), RuntimeError> {
    let options = super::member(object, "Options")?.to_string();
    if options
        .split(", ")
        .any(|o| matches!(o, "ReadOnly" | "Constant"))
    {
        let name = super::member(object, "Name")?.to_string();
        let reason = rpsh_messages::format("SessionStateStrings/VariableNotWritable", &[&name]);
        return Err(RuntimeError::Script {
            id: "ExceptionWhenSetting",
            category: "NotSpecified: (:) [], SetValueInvocationException".into(),
            exception_type: "System.Management.Automation.SetValueInvocationException",
            message: format!("Exception setting \"Value\": \"{reason}\""),
        });
    }
    replace(object, value);
    Ok(())
}

fn options(v: &Value) -> Result<Value, RuntimeError> {
    let text = v.to_string();
    let integer = text.parse::<i32>().ok().filter(|n| (0..32).contains(n));
    let parts = || {
        text.split(',')
            .map(|s| OPTIONS.entry(s.trim()).map(|e| e.1))
            .try_fold(0, |i, x| Some(i | x?))
    };
    if let Some(bits) = integer.or_else(parts) {
        let label = OPTIONS
            .entries
            .iter()
            .filter(|(_, bit, _)| {
                if bits == 0 {
                    *bit == 0
                } else {
                    bits & *bit != 0
                }
            })
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        Ok(errors::enum_number(OPTIONS.ty, &label, bits))
    } else {
        OPTIONS
            .parse(v)
            .map(|e| OPTIONS.value(e))
            .map_err(|(_, reason)| {
                method_error::argument_error(".ctor", "options", OPTIONS.ty, &text, &reason)
            })
    }
}

pub(super) fn construct(args: &[Value]) -> Result<Value, RuntimeError> {
    let [name, rest @ ..] = args else {
        return Err(method_error::overload_error("new", 0));
    };
    if rest.len() > 3 {
        return Err(method_error::overload_error("new", args.len()));
    }
    let name = name.to_string();
    if name.is_empty() {
        let detail = rpsh_messages::format("AutomationExceptions/Argument", &["name"]);
        return Err(
            method_error::invocation("PSArgumentException", ".ctor", args.len(), &detail)
                .caused_by("System.Management.Automation.PSArgumentException", detail),
        );
    }
    let mut object = make(&name, rest.first().cloned().unwrap_or(Value::Null), false);
    if let Value::Object(fields) = &mut object {
        for (k, v) in std::rc::Rc::make_mut(fields) {
            if k == "Options"
                && let Some(o) = rest.get(1)
            {
                *v = options(o)?;
            }
            if k == "Attributes"
                && let Some(attrs) = rest.get(2).filter(|v| !v.is_null_like())
            {
                let items = super::flatten_value(attrs.clone());
                if items
                    .iter()
                    .any(|a| !hidden_type(a).is_some_and(|n| n.ends_with("Attribute")))
                {
                    return Err(method_error::overload_error("new", args.len()));
                }
                *v = Value::collection(&ATTRIBUTES, items);
            }
        }
    }
    Ok(object)
}

fn convertible_error(v: &Value, display: &str, full_name: &str) -> Option<RuntimeError> {
    let kind = match v {
        Value::Bool(_) => "System.Boolean",
        _ if hidden_type(v).as_deref() == Some("System.DateTime") => "System.DateTime",
        _ => return None,
    };
    let reason = format!("Invalid cast from '{kind}' to '{full_name}'.");
    let message =
        format!("Cannot convert value \"{v}\" to type \"{display}\". Error: \"{reason}\"");
    Some(
        method_error::cast_error("InvalidCastIConvertible", message.clone()).caused_by(
            "System.Management.Automation.PSInvalidCastException",
            message,
        ),
    )
}

pub(super) fn cast(v: Value) -> Result<Value, RuntimeError> {
    if v.is_null_like() || storage(&v).is_some() {
        return Ok(v);
    }
    if is_instance(&v) {
        return construct(&[super::member(&v, "Name")?, super::member(&v, "Value")?]);
    }
    if let Some(error) = convertible_error(&v, TYPE, TYPE) {
        return Err(error);
    }
    if matches!(
        v,
        Value::Object(_)
            | Value::Array(_)
            | Value::StringArray(_)
            | Value::ArrayList(_)
            | Value::ObjectList(_)
            | Value::Collection(_)
    ) {
        let ty = get_type(&v).to_string();
        let text =
            format!("Cannot convert the \"{ty}\" value of type \"{ty}\" to type \"{TYPE}\".");
        return Err(
            method_error::cast_error("ConvertToFinalInvalidCastException", text.clone())
                .caused_by("System.Management.Automation.PSInvalidCastException", text),
        );
    }
    let name = v.to_string();
    if name.is_empty() {
        let reason = rpsh_messages::format("AutomationExceptions/Argument", &["name"]);
        return Err(method_error::cast_error(
            "InvalidCastConstructorException",
            format!("Cannot convert value \"\" to type \"{TYPE}\". Error: \"{reason}\""),
        ));
    }
    Ok(make(&name, Value::Null, false))
}

pub(super) fn list_cast(v: &Value) -> Result<Value, RuntimeError> {
    if v.is_null_like() || matches!(v,Value::Collection(c) if c.ty.element == TYPE) {
        return Ok(v.clone());
    }
    let ty = &metadata::VARIABLE_LIST;
    if let Some(err) = convertible_error(v, ty.display_name, ty.full_name) {
        return Err(err);
    }
    let items = match v {
        Value::Array(items) | Value::StringArray(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items.iter() {
                values.push(cast(item.clone())?)
            }
            values
        }
        _ => vec![cast(v.clone())?],
    };
    Ok(Value::collection(&metadata::VARIABLE_LIST, items))
}

pub(super) fn argument(v: &Value, method: &str, arity: usize) -> Result<Value, RuntimeError> {
    let (canonical, parameter) = match method {
        "set_item" => ("set_Item", "value"),
        "insert" => ("Insert", "item"),
        "remove" => ("Remove", "item"),
        "contains" => ("Contains", "item"),
        "indexof" => ("IndexOf", "item"),
        _ => ("Add", "item"),
    };
    cast(v.clone()).map_err(|err| {
        if err.feature() == "ConvertToFinalInvalidCastException" {
            return method_error::overload_error(canonical, arity);
        }
        let why = err.to_string();
        method_error::argument_error(canonical, parameter, TYPE, &v.to_string(), &why)
            .caused_by("System.Management.Automation.PSInvalidCastException", why)
    })
}

pub(super) fn list_new(args: &[Value]) -> Result<Value, RuntimeError> {
    let list = match args {
        [Value::Collection(c)] if c.ty.element == TYPE => {
            ArrayList::from_values(c.items.items().to_vec())
        }
        [Value::Array(_) | Value::StringArray(_) | Value::Collection(_) | Value::ObjectList(_)] => {
            return Err(method_error::overload_error("new", 1));
        }
        _ => {
            let Value::ObjectList(list) =
                super::array_list::construct(super::array_list::Kind::ObjectList, args)?
            else {
                unreachable!()
            };
            list
        }
    };
    Ok(Value::Collection(std::rc::Rc::new(
        rpsh_runtime::TypedCollection {
            ty: &metadata::VARIABLE_LIST,
            items: list,
            sync_root: None,
        },
    )))
}

pub(super) fn field_text(v: &Value) -> String {
    fn braced(mut fields: impl Iterator<Item = String>) -> String {
        let mut text = fields.by_ref().take(4).collect::<Vec<_>>().join(", ");
        if fields.next().is_some() {
            text.push('…')
        }
        format!("{{{text}}}")
    }
    match v {
        Value::Array(a) | Value::StringArray(a) => braced(a.iter().map(ToString::to_string)),
        Value::ArrayList(a) | Value::ObjectList(a) => {
            braced(a.items().iter().map(ToString::to_string))
        }
        Value::Collection(c) => braced(c.items.items().iter().map(ToString::to_string)),
        Value::Object(fields)
            if matches!(
                hidden_type(v).as_deref(),
                Some(
                    "System.Collections.Hashtable"
                        | "System.Collections.Specialized.OrderedDictionary"
                )
            ) =>
        {
            braced(
                fields
                    .iter()
                    .filter(|(k, _)| !super::is_hidden(k))
                    .map(|(k, v)| format!("[{k}, {v}]")),
            )
        }
        _ => v.to_string(),
    }
}

pub(super) fn table(objects: &[Value], width: usize) -> Option<String> {
    use rpsh_format::CommandTableItem;
    let first = objects.first()?;
    if !is_instance(first)
        || super::format_marker(first).as_deref() == Some("List")
        || !super::format_table_properties(first).is_empty()
    {
        return None;
    }
    let flag = |name| matches!(first,Value::Object(fields) if fields.iter().any(|(k,v)|k==name && rpsh_runtime::ops::is_true(v)));
    let rows = objects
        .iter()
        .filter(|v| !v.is_null_like())
        .map(|v| {
            if is_instance(v) {
                let name = super::member(v, "Name").unwrap_or(Value::Null).to_string();
                let val = super::member(v, "Value").unwrap_or(Value::Null);
                let displayed = field_text(&val);
                CommandTableItem::Command([name, displayed])
            } else {
                CommandTableItem::Text(v.to_string())
            }
        })
        .collect::<Vec<_>>();
    Some(rpsh_format::variable_table(
        &rows,
        width,
        flag("__AutoSize"),
        flag("__HideHeaders"),
    ))
}
