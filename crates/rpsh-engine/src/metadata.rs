//! PowerShell's public command metadata, built on first inspection from the binder declarations.
use super::{array_list, hidden_type, member, member_enumeration, method_error, overload_error};
use rpsh_commands::signature::{self, COMMON_PARAMETERS, Parameter, ParameterType, Signature};
use rpsh_runtime::{CollectionType, RuntimeError, TypedCollection, Value, ops};
use std::path::Path;

pub(super) const COMMAND_METADATA: &str = "System.Management.Automation.CommandMetadata";
const PARAMETER_METADATA: &str = "System.Management.Automation.ParameterMetadata";
const SET_METADATA: &str = "System.Management.Automation.ParameterSetMetadata";
const ALL: &[&str] = &["__AllParameterSets"];

macro_rules! core {
    ($name:literal) => { concat!($name, ", System.Private.CoreLib, Version=11.0.0.0, Culture=neutral, PublicKeyToken=7cec85d7bea7798e") };
}
macro_rules! sma {
    ($name:literal) => { concat!($name, ", System.Management.Automation, Version=7.7.0.500, Culture=neutral, PublicKeyToken=31bf3856ad364e35") };
}
macro_rules! collection {
    ($class:literal,$element:literal,$backing:expr) => {
        CollectionType {
            full_name: concat!($class, "`1[[", core!($element), "]]"),
            display_name: concat!($class, "`1[", $element, "]"),
            element: $element,
            read_only: false,
            backing: $backing,
        }
    };
}
static STRING_LIST: CollectionType =
    collection!("System.Collections.Generic.List", "System.String", None);
static ATTRIBUTE_LIST: CollectionType =
    collection!("System.Collections.Generic.List", "System.Attribute", None);
static STRINGS: CollectionType = collection!(
    "System.Collections.ObjectModel.Collection",
    "System.String",
    Some(&STRING_LIST)
);
static ATTRIBUTES: CollectionType = collection!(
    "System.Collections.ObjectModel.Collection",
    "System.Attribute",
    Some(&ATTRIBUTE_LIST)
);

macro_rules! view_type {
    ($class:literal,$value:literal,$element:literal) => {
        CollectionType {
            full_name: concat!(
                "System.Collections.Generic.Dictionary`2+",
                $class,
                "[[",
                core!("System.String"),
                "],[",
                sma!($value),
                "]]"
            ),
            display_name: concat!(
                "System.Collections.Generic.Dictionary`2+",
                $class,
                "[System.String,",
                $value,
                "]"
            ),
            element: $element,
            read_only: true,
            backing: None,
        }
    };
}
static PARAM_KEYS: CollectionType = view_type!(
    "KeyCollection",
    "System.Management.Automation.ParameterMetadata",
    "System.String"
);
static PARAM_VALUES: CollectionType = view_type!(
    "ValueCollection",
    "System.Management.Automation.ParameterMetadata",
    "System.Management.Automation.ParameterMetadata"
);
static SET_KEYS: CollectionType = view_type!(
    "KeyCollection",
    "System.Management.Automation.ParameterSetMetadata",
    "System.String"
);
static SET_VALUES: CollectionType = view_type!(
    "ValueCollection",
    "System.Management.Automation.ParameterSetMetadata",
    "System.Management.Automation.ParameterSetMetadata"
);

fn string(text: &str) -> Value {
    Value::String(text.into())
}

fn object(ty: &str, display: &str, mut properties: Vec<(String, Value)>) -> Value {
    properties.push(("__TypeName".into(), string(ty)));
    properties.push(("__Text".into(), string(display)));
    Value::object(properties)
}

fn dictionary(ty: &'static str, mut props: Vec<(String, Value)>) -> Value {
    let value_type = if ty == SET_METADATA {
        sma!("System.Management.Automation.ParameterSetMetadata")
    } else {
        sma!("System.Management.Automation.ParameterMetadata")
    };
    let full = format!(
        "System.Collections.Generic.Dictionary`2[[{}],[{value_type}]]",
        core!("System.String")
    );
    let display = format!("System.Collections.Generic.Dictionary`2[System.String,{ty}]");
    props.push(("__MetadataValueType".into(), string(ty)));
    object(&full, &display, props)
}

pub(super) fn dictionary_entries(
    props: &[(String, Value)],
) -> impl Iterator<Item = &(String, Value)> {
    props
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "__MetadataValueType" | "__TypeName" | "__Text"))
}

/// Metadata dictionaries can contain real keys beginning with two underscores,
/// including the upstream __AllParameterSets sentinel.
pub(super) fn dictionary_view(value: &Value, name: &str) -> Option<Value> {
    let keys = name.eq_ignore_ascii_case("Keys");
    if !keys && !name.eq_ignore_ascii_case("Values") && !name.eq_ignore_ascii_case("Count") {
        return None;
    }
    let Value::Object(props) = value else {
        return None;
    };
    let (_, kind) = props.iter().find(|(k, _)| k == "__MetadataValueType")?;
    let set = kind.to_string() == SET_METADATA;
    let entries = dictionary_entries(props);
    if name.eq_ignore_ascii_case("Count") {
        return Some(Value::Int32(entries.count() as i32));
    }
    let ty = match (set, keys) {
        (true, true) => &SET_KEYS,
        (true, false) => &SET_VALUES,
        (false, true) => &PARAM_KEYS,
        (false, false) => &PARAM_VALUES,
    };
    let items = entries
        .map(|(k, v)| if keys { string(k) } else { v.clone() })
        .collect();
    Some(Value::Collection(std::rc::Rc::new(TypedCollection {
        ty,
        items: rpsh_runtime::ArrayList::from_values(items),
        sync_root: Some(value.clone()),
    })))
}

fn enumeration(ty: &str, name: &str, number: i32) -> Value {
    object(ty, name, vec![("value__".into(), Value::Int32(number))])
}

fn set_properties(p: &Parameter) -> Vec<(String, Value)> {
    vec![
        ("IsMandatory".into(), Value::Bool(p.mandatory)),
        (
            "Position".into(),
            Value::Int32(p.position.map_or(i32::MIN, i32::from)),
        ),
        ("ValueFromPipeline".into(), Value::Bool(p.from_pipeline)),
        (
            "ValueFromPipelineByPropertyName".into(),
            Value::Bool(p.from_property),
        ),
        (
            "ValueFromRemainingArguments".into(),
            Value::Bool(p.remaining),
        ),
        ("HelpMessage".into(), Value::Null),
        ("HelpMessageBaseName".into(), Value::Null),
        ("HelpMessageResourceId".into(), Value::Null),
    ]
}

fn attribute(name: &str, mut fields: Vec<(String, Value)>) -> Value {
    let full = format!("System.Management.Automation.{name}");
    fields.push(("TypeId".into(), Value::Type(full.as_str().into())));
    fields.push((
        "__BaseClasses".into(),
        Value::array(vec![string("System.Attribute")]),
    ));
    object(&full, &full, fields)
}

fn parameter(p: &Parameter) -> Value {
    let sets = if p.sets.is_empty() { ALL } else { p.sets };
    let mut attrs = Vec::new();
    let mut set_entries = Vec::new();
    for &set in sets {
        let mut props = set_properties(p);
        set_entries.push((
            set.to_owned(),
            object(SET_METADATA, SET_METADATA, props.clone()),
        ));
        props[0].0 = "Mandatory".into();
        props.extend([
            ("ParameterSetName".into(), string(set)),
            ("DontShow".into(), Value::Bool(false)),
            ("ExperimentName".into(), Value::Null),
            (
                "ExperimentAction".into(),
                enumeration("System.Management.Automation.ExperimentAction", "None", 0),
            ),
        ]);
        attrs.push(attribute("ParameterAttribute", props));
    }
    if p.not_null_or_empty {
        attrs.push(attribute("ValidateNotNullOrEmptyAttribute", vec![]));
    }
    if !p.valid_values.is_empty() {
        attrs.push(attribute(
            "ValidateSetAttribute",
            vec![
                (
                    "ValidValues".into(),
                    Value::string_array(p.valid_values.iter().map(|&v| Some(v.into()))),
                ),
                ("IgnoreCase".into(), Value::Bool(true)),
                ("ErrorMessage".into(), Value::Null),
            ],
        ));
    }
    if p.allow_null {
        attrs.push(attribute("AllowNullAttribute", vec![]));
    }
    if p.allow_empty_collection {
        attrs.push(attribute("AllowEmptyCollectionAttribute", vec![]));
    }
    if let Some((min, max)) = p.numeric_range {
        attrs.push(attribute(
            "ValidateRangeAttribute",
            vec![
                ("MinRange".into(), Value::Int32(min)),
                ("MaxRange".into(), Value::Int32(max)),
            ],
        ));
    }
    if !p.aliases.is_empty() {
        attrs.push(attribute(
            "AliasAttribute",
            vec![(
                "AliasNames".into(),
                Value::string_array(p.aliases.iter().map(|&s| Some(s.into()))),
            )],
        ));
    }
    if p.variable_name {
        attrs.push(attribute(
            "Internal.CommonParameters+ValidateVariableName",
            vec![],
        ));
    }
    object(
        PARAMETER_METADATA,
        PARAMETER_METADATA,
        vec![
            ("Name".into(), string(p.name)),
            ("ParameterType".into(), Value::Type(p.ty.full_name().into())),
            (
                "ParameterSets".into(),
                dictionary(SET_METADATA, set_entries),
            ),
            ("IsDynamic".into(), Value::Bool(false)),
            (
                "Aliases".into(),
                Value::collection(&STRINGS, p.aliases.iter().map(|s| string(s)).collect()),
            ),
            ("Attributes".into(), Value::collection(&ATTRIBUTES, attrs)),
            (
                "SwitchParameter".into(),
                Value::Bool(p.ty == ParameterType::Switch),
            ),
        ],
    )
}

fn parameters(sig: &Signature, common: bool) -> Value {
    let additional = if common { COMMON_PARAMETERS } else { &[] };
    dictionary(
        PARAMETER_METADATA,
        sig.parameters
            .iter()
            .chain(additional)
            .map(|p| (p.name.to_owned(), parameter(p)))
            .collect(),
    )
}

pub(super) fn command_parameters(info: &Value) -> Option<Value> {
    let Value::Object(props) = info else {
        return None;
    };
    let (_, Value::ArrayList(cache)) =
        props.iter().find(|(k, _)| k == "__CommandParameterCache")?
    else {
        return None;
    };
    if let Some(value) = cache.items().first() {
        return Some(value.clone());
    }
    let name = props.iter().find(|(k, _)| k == "Name")?.1.to_string();
    let result = parameters(signature::lookup(&name)?, true);
    cache.items_mut().push(result.clone());
    Some(result)
}

fn remoting(capability: &str) -> Value {
    enumeration(
        "System.Management.Automation.RemotingCapability",
        capability,
        i32::from(capability == "PowerShell"),
    )
}

fn metadata(ty: &str, signature: Option<&Signature>, common: bool, qualified: bool) -> Value {
    let wrapped = signature.map_or(Value::Null, |sig| {
        if qualified {
            string(&format!("{}\\{}", sig.module, sig.name))
        } else {
            string(sig.name)
        }
    });
    let default_set = signature.map_or(Some("__AllParameterSets"), |s| s.default_set);
    let (impact, number) = if signature.is_some() {
        ("None", 0)
    } else {
        ("Medium", 2)
    };
    object(
        COMMAND_METADATA,
        COMMAND_METADATA,
        vec![
            (
                "Name".into(),
                signature.map_or(Value::Null, |s| string(s.name)),
            ),
            ("__WrappedCommand".into(), wrapped),
            ("CommandType".into(), Value::Type(ty.into())),
            (
                "DefaultParameterSetName".into(),
                default_set.map_or(Value::Null, string),
            ),
            ("SupportsShouldProcess".into(), Value::Bool(false)),
            ("SupportsPaging".into(), Value::Bool(false)),
            ("PositionalBinding".into(), Value::Bool(true)),
            ("SupportsTransactions".into(), Value::Bool(false)),
            (
                "HelpUri".into(),
                string(signature.map_or("", |s| s.help_uri)),
            ),
            (
                "RemotingCapability".into(),
                remoting(signature.map_or("PowerShell", |s| s.remoting_capability)),
            ),
            (
                "ConfirmImpact".into(),
                enumeration("System.Management.Automation.ConfirmImpact", impact, number),
            ),
            (
                "Parameters".into(),
                signature.map_or_else(
                    || dictionary(PARAMETER_METADATA, vec![]),
                    |s| parameters(s, common),
                ),
            ),
        ],
    )
}

fn copy(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let command = hidden_type(value).as_deref() == Some(COMMAND_METADATA);
            let fields = fields
                .iter()
                .map(|(name, field)| {
                    let copied = match name.as_str() {
                        "RemotingCapability" if command => remoting("PowerShell"),
                        "HelpUri" if command => string(""),
                        "PositionalBinding" if command => Value::Bool(true),
                        _ => copy(field),
                    };
                    (name.clone(), copied)
                })
                .collect();
            Value::object(fields)
        }
        // Each copy owns its alias and attribute collections. Attribute instances are shared.
        Value::Collection(c) => Value::collection(c.ty, c.items.items().to_vec()),
        other => other.clone(),
    }
}

pub(super) fn construct(args: &[Value]) -> Result<Value, RuntimeError> {
    let (source, common) = match args {
        [source] => (source, false),
        [source, common] => (source, ops::is_true(common)),
        _ => return Err(overload_error("new", args.len())),
    };
    if source.is_null_like() {
        let parameter = if args.len() == 1 {
            "other"
        } else {
            "commandInfo"
        };
        return Err(method_error::invocation(
            "PSArgumentNullException",
            ".ctor",
            args.len(),
            &rpsh_messages::format("AutomationExceptions/ArgumentNull", &[parameter]),
        ));
    }
    if let Value::String(path) = source
        && args.len() == 1
    {
        let path = path.to_string_lossy();
        let absolute = std::path::absolute(Path::new(&path)).ok();
        if let Some(path) = absolute
            && !path.exists()
        {
            return Err(method_error::invocation(
                "CommandNotFoundException",
                ".ctor",
                1,
                &format!("Could not find file '{}'.", path.display()),
            ));
        }
        return Err(RuntimeError::NotImplemented {
            feature: "CommandMetadata for script files",
        });
    }
    if let Value::Type(name) = source
        && args.len() == 1
    {
        let name = name.to_string_lossy();
        if let Some(sig) = signature::SIGNATURES
            .iter()
            .find(|s| s.implementation.eq_ignore_ascii_case(&name))
        {
            return Ok(metadata(sig.implementation, Some(sig), false, false));
        }
        if matches!(
            name.as_str(),
            "System.Object"
                | "System.String"
                | "System.Boolean"
                | "System.Char"
                | "System.Int16"
                | "System.Int32"
                | "System.Int64"
                | "System.UInt16"
                | "System.UInt32"
                | "System.UInt64"
                | "System.Byte"
                | "System.SByte"
                | "System.Decimal"
                | "System.Double"
                | "System.Array"
        ) {
            return Ok(metadata(&name, None, false, false));
        }
        return Err(RuntimeError::NotImplemented {
            feature: "CommandMetadata for this CLR type",
        });
    }
    let name = match hidden_type(source).as_deref() {
        Some(COMMAND_METADATA) if args.len() == 1 => return Ok(copy(source)),
        Some(super::functions::CMDLET) => member(source, "Name")?.to_string(),
        Some("System.Management.Automation.AliasInfo") => member(source, "Definition")?.to_string(),
        Some(super::functions::FUNCTION | super::functions::FILTER) => {
            return Err(RuntimeError::NotImplemented {
                feature: "CommandMetadata for script functions",
            });
        }
        _ => return Err(overload_error("new", args.len())),
    };
    signature::lookup(&name)
        .map(|s| metadata(s.implementation, Some(s), common, true))
        .ok_or(RuntimeError::NotImplemented {
            feature: "CommandMetadata for a command without a parameter declaration",
        })
}

pub(super) fn collection_property(
    c: &std::rc::Rc<TypedCollection>,
    name: &str,
) -> Result<Value, RuntimeError> {
    match name.to_ascii_lowercase().as_str() {
        "count" => Ok(Value::Int32(c.items.items().len() as i32)),
        "isreadonly" => Ok(Value::Bool(c.ty.read_only)),
        "isfixedsize" if c.sync_root.is_none() => Ok(Value::Bool(false)),
        "isfixedsize" => Ok(Value::Null),
        "capacity" if c.ty.backing.is_none() && !c.ty.read_only => {
            Ok(Value::Int32(c.items.items().capacity() as i32))
        }
        "issynchronized" => Ok(Value::Bool(false)),
        "syncroot" => Ok(c.sync_root.clone().unwrap_or_else(|| match c.ty.backing {
            Some(ty) => Value::Collection(std::rc::Rc::new(TypedCollection {
                ty,
                items: c.items.clone(),
                sync_root: None,
            })),
            None => Value::Collection(c.clone()),
        })),
        _ => member_enumeration(&c.items.items(), name),
    }
}

pub(super) fn assign_index(
    c: &std::rc::Rc<TypedCollection>,
    index: &Value,
    value: Value,
) -> Result<(), RuntimeError> {
    let n = super::convert_int32(index)?;
    let len = c.items.items().len();
    if let Some(at) = array_list::position(n, len) {
        collection_method(c, "set_Item", &[Value::Int32(at as i32), value])?;
        Ok(())
    } else {
        array_list::set_index(&c.items, index, value)
    }
}

pub(super) fn collection_method(
    c: &std::rc::Rc<TypedCollection>,
    name: &str,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let lower = name.to_ascii_lowercase();
    if c.sync_root.is_some() {
        let keys = c.ty.full_name.contains("+KeyCollection");
        match (lower.as_str(), args) {
            ("add" | "remove", [_]) | ("clear", []) => {
                let (target, method) = (
                    if keys { "key" } else { "value" },
                    match lower.as_str() {
                        "add" => "Add",
                        "remove" => "Remove",
                        _ => "Clear",
                    },
                );
                return Err(method_error::invocation(
                    "NotSupportedException",
                    method,
                    args.len(),
                    &format!(
                        "Mutating a {target} collection derived from a dictionary is not allowed."
                    ),
                ));
            }
            ("contains", [value]) if keys => {
                let text = value.to_string();
                return Ok(Value::Bool(
                    c.items
                        .items()
                        .iter()
                        .any(|x| x.to_string().eq_ignore_ascii_case(&text)),
                ));
            }
            _ => (),
        }
    }
    match (lower.as_str(), args) {
        ("tostring", []) => return Ok(string(c.ty.display_name)),
        ("equals", [other]) => {
            return Ok(Value::Bool(
                matches!(other,Value::Collection(right) if c.same_instance(right)),
            ));
        }
        ("contains" | "indexof" | "get_item", [_]) => (),
        ("clear", []) | ("add" | "remove" | "removeat", [_]) | ("insert" | "set_item", [_, _])
            if !c.ty.read_only => {}
        _ => {
            return Err(RuntimeError::NotImplemented {
                feature: "this metadata collection method",
            });
        }
    }
    let mut args = args.to_vec();
    if let Some(last) = args.last_mut()
        && matches!(
            lower.as_str(),
            "add" | "remove" | "contains" | "indexof" | "insert" | "set_item"
        )
    {
        if c.ty.element == "System.String" && !last.is_null_like() {
            *last = Value::String(last.to_ps_string());
        } else if c.ty.element == "System.Attribute"
            && !last.is_null_like()
            && !hidden_type(last).is_some_and(|name| {
                name.ends_with("Attribute") || name.ends_with("+ValidateVariableName")
            })
        {
            let canonical = match lower.as_str() {
                "add" => "Add",
                "remove" => "Remove",
                "contains" => "Contains",
                "indexof" => "IndexOf",
                "insert" => "Insert",
                _ => "set_Item",
            };
            return Err(overload_error(canonical, args.len()));
        }
    }
    array_list::method(array_list::Kind::ObjectList, &c.items, name, &args)
}
