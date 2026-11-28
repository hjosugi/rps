//! Sort-Object evaluates each key once per input and compares the typed columns
//! from left to right. A missing property follows every present property.
mod indices;

use super::{
    Engine, EngineError, cast_to, convert_int32, datetime_fields, datetime_ticks, flatten_value,
    member, timespan_ticks, visible_entries,
};
use rpsh_runtime::{PsString, RuntimeError, Value, culture, ops, wildcard};
use rpsh_syntax::{ArgumentSeparator, CommandArgument, Statement, parse};
use std::cell::OnceCell;
use std::cmp::Ordering;

enum Expression {
    Input,
    Property(String),
    Script(Vec<Statement>),
}

struct Column {
    expression: Expression,
    descending: bool,
}

struct Field {
    value: Value,
    text_key: OnceCell<Result<Vec<u8>, RuntimeError>>,
}

impl Field {
    fn new(value: Value) -> Self {
        Self {
            value,
            text_key: OnceCell::new(),
        }
    }

    fn text_key(&self, locale: &str, sensitive: bool) -> Result<&[u8], RuntimeError> {
        self.text_key
            .get_or_init(|| {
                let text = match &self.value {
                    // PowerShell compares the unwrapped PSCustomObjects, whose
                    // ToString() is empty when they have no adapted text.
                    Value::Object(entries) if !entries.iter().any(|(k, _)| k == "__Text") => {
                        PsString::from("")
                    }
                    value => value.to_ps_string(),
                };
                culture::sort_key(&text, locale, sensitive)
            })
            .as_deref()
            .map_err(Clone::clone)
    }

    fn compare(
        &self,
        other: &Self,
        locale: &str,
        sensitive: bool,
    ) -> Result<Ordering, RuntimeError> {
        let (a, b) = (&self.value, &other.value);
        if a.is_null_like() || b.is_null_like() {
            let relative_to_null = |value: &Value| {
                if value.is_null_like() {
                    Ordering::Equal
                } else if !matches!(value, Value::BigInteger(_))
                    && ops::number_order(value, &Value::Int32(0)) == Some(Ordering::Less)
                {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            };
            return Ok(if b.is_null_like() {
                relative_to_null(a)
            } else {
                relative_to_null(b).reverse()
            });
        }
        if let Some(order) = ops::number_order(a, b) {
            return Ok(order);
        }
        match (a, b) {
            (Value::Char(a), Value::Char(b)) => return Ok(a.cmp(b)),
            (Value::Bool(a), Value::Bool(b)) => return Ok(a.cmp(b)),
            (Value::Object(_), Value::Object(_)) => {
                if let (Some(a), Some(b)) = (datetime_fields(a), datetime_fields(b)) {
                    return Ok(datetime_ticks(a).cmp(&datetime_ticks(b)));
                }
                if let (Some(a), Some(b)) = (timespan_ticks(a), timespan_ticks(b)) {
                    return Ok(a.cmp(&b));
                }
            }
            (a, _) if !matches!(a, Value::String(_)) && a.type_name() != b.type_name() => {
                if let Ok(converted) = cast_to(a.type_name(), b.clone()) {
                    if let Some(order) = ops::number_order(a, &converted) {
                        return Ok(order);
                    }
                    if let (Value::Bool(a), Value::Bool(b)) = (a, &converted) {
                        return Ok(a.cmp(b));
                    }
                    if let (Value::Char(a), Value::Char(b)) = (a, &converted) {
                        return Ok(a.cmp(b));
                    }
                }
            }
            _ => {}
        }
        Ok(self
            .text_key(locale, sensitive)?
            .cmp(other.text_key(locale, sensitive)?))
    }
}

const PARAMETER_NAMES: &[&str] = &[
    "Stable",
    "Descending",
    "Unique",
    "Top",
    "Bottom",
    "Property",
    "InputObject",
    "Culture",
    "CaseSensitive",
];

fn binding_error(id: &'static str, message: String) -> RuntimeError {
    RuntimeError::Script {
        id,
        message,
        category: "InvalidArgument: (:) [Sort-Object], ParameterBindingException".into(),
        exception_type: "System.Management.Automation.ParameterBindingException",
    }
}

// As in the cmdlet binder, a switch followed by whitespace leaves the next
// value positional. Only a colon binds an explicit boolean to that switch.
fn sort_parameter(name: &str) -> Result<&str, RuntimeError> {
    let lower = name.to_ascii_lowercase();
    let mut matches = PARAMETER_NAMES
        .iter()
        .copied()
        .filter(|key| key.to_ascii_lowercase().starts_with(&lower));
    let Some(first) = matches.next() else {
        // Common parameter names are valid for every cmdlet.
        if crate::functions::common_binding(&lower).is_some() {
            return Ok(name);
        }
        return Err(binding_error(
            "NamedParameterNotFound,Microsoft.PowerShell.Commands.SortObjectCommand",
            format!("A parameter cannot be found that matches parameter name '{name}'."),
        ));
    };
    if let Some(second) = matches.next() {
        let options = std::iter::once(first)
            .chain(std::iter::once(second))
            .chain(matches)
            .map(|s| format!("-{s}"))
            .collect::<Vec<_>>()
            .join(" ");
        return Err(binding_error(
            "AmbiguousParameter,Microsoft.PowerShell.Commands.SortObjectCommand",
            format!(
                "Parameter cannot be processed because the parameter name '{name}' is ambiguous. Possible matches include: {options}."
            ),
        ));
    }
    Ok(first)
}

impl Engine {
    fn bind_sort_arguments<'a>(
        &mut self,
        arguments: &'a [CommandArgument],
    ) -> Result<(Vec<Value>, std::collections::HashMap<&'a str, Value>), EngineError> {
        let mut positional = Vec::new();
        let mut named = std::collections::HashMap::new();
        for argument in arguments {
            match argument {
                CommandArgument::Value(expr) => positional.push(self.eval_expression(expr)?),
                CommandArgument::Bareword(name) => {
                    positional.push(Value::String(name.as_str().into()))
                }
                CommandArgument::Parameter {
                    name,
                    value,
                    separator,
                } => {
                    let name = sort_parameter(name)?;
                    let switch =
                        matches!(name, "Stable" | "Descending" | "Unique" | "CaseSensitive");
                    let bound = match value {
                        Some(value) if switch && *separator == ArgumentSeparator::Space => {
                            positional.push(self.eval_expression(value)?);
                            Value::Bool(true)
                        }
                        Some(value) => self.eval_expression(value)?,
                        None if switch || !PARAMETER_NAMES.contains(&name) => Value::Bool(true),
                        None => {
                            let ty = match name {
                                "Top" | "Bottom" => "System.Int32",
                                "Culture" => "System.String",
                                "InputObject" => "System.Management.Automation.PSObject",
                                _ => "System.Object[]",
                            };
                            return Err(binding_error("MissingArgument,Microsoft.PowerShell.Commands.SortObjectCommand",format!("Missing an argument for parameter '{name}'. Specify a parameter of type '{ty}' and try again.")).into());
                        }
                    };
                    if named.insert(name, bound).is_some() {
                        return Err(binding_error("ParameterAlreadyBound,Microsoft.PowerShell.Commands.SortObjectCommand",format!("Cannot bind parameter because parameter '{name}' is specified more than once. To provide multiple values to parameters that can accept multiple values, use the array syntax. For example, \"-parameter value1,value2,value3\".")).into());
                    }
                }
            }
        }
        Ok((positional, named))
    }

    pub(super) fn sort_objects(
        &mut self,
        arguments: &[CommandArgument],
        input: &[Value],
    ) -> Result<Vec<Value>, EngineError> {
        let (positional, named) = self.bind_sort_arguments(arguments)?;
        let parameter = |name: &str| named.get(name);
        let flag = |name: &str| parameter(name).is_some_and(ops::is_true);
        let top = parameter("Top");
        let bottom = parameter("Bottom");
        if usize::from(top.is_some())
            + usize::from(bottom.is_some())
            + usize::from(parameter("Stable").is_some())
            > 1
        {
            return Err(RuntimeError::Script {
                id: "AmbiguousParameterSet,Microsoft.PowerShell.Commands.SortObjectCommand",
                category: "InvalidArgument: (:) [Sort-Object], ParameterBindingException".into(),
                exception_type: "System.Management.Automation.ParameterBindingException",
                message: "Parameter set cannot be resolved using the specified named parameters. One or more parameters issued cannot be used together or an insufficient number of parameters were provided.".into(),
            }.into());
        }
        let limit = match top.or(bottom) {
            Some(value) => {
                let n = convert_int32(value)?;
                if n < 1 {
                    let name = if top.is_some() { "Top" } else { "Bottom" };
                    return Err(RuntimeError::Script {
                        id: "ParameterArgumentValidationError,Microsoft.PowerShell.Commands.SortObjectCommand",
                        category: "InvalidData: (:) [Sort-Object], ParameterBindingValidationException".into(),
                        exception_type: "System.Management.Automation.ParameterBindingValidationException",
                        message: format!("Cannot validate argument on parameter '{name}'. The {n} argument is less than the minimum allowed range of 1. Supply an argument that is greater than or equal to 1 and then try the command again."),
                    }.into());
                }
                Some(n as usize)
            }
            None => None,
        };
        let locale = sort_locale(parameter("Culture"))?;
        let input: Vec<_> = input
            .iter()
            .chain(parameter("InputObject"))
            .filter(|v| !v.is_null_like())
            .collect();
        let descending = flag("Descending");
        let sensitive = flag("CaseSensitive");
        let stable = flag("Stable") || limit.is_some();
        let mut columns = Vec::new();
        let properties = parameter("Property")
            .into_iter()
            .chain(&positional)
            .cloned()
            .flat_map(flatten_value);
        for property in properties {
            columns.extend(sort_columns(property, descending, &input)?);
        }
        if columns.is_empty() {
            columns.push(Column {
                expression: Expression::Input,
                descending,
            });
        }
        let mut rows = Vec::with_capacity(input.len());
        for item in &input {
            let mut fields = Vec::with_capacity(columns.len());
            for column in &columns {
                let value = match &column.expression {
                    Expression::Input => Some((*item).clone()),
                    Expression::Property(name) => property_value(item, name),
                    Expression::Script(statements) => {
                        let previous = self.variables.insert("_".into(), (*item).clone());
                        let result = self.eval_group(statements);
                        if let Some(value) = previous {
                            self.variables.insert("_".into(), value);
                        } else {
                            self.variables.remove("_");
                        }
                        Some(result?)
                    }
                };
                fields.push(value.map(Field::new));
            }
            rows.push(fields);
        }
        let compare = |a: usize, b: usize| -> Result<Ordering, RuntimeError> {
            for ((a, b), column) in rows[a].iter().zip(&rows[b]).zip(&columns) {
                let order = match (a, b) {
                    (Some(a), Some(b)) => {
                        let value = a.compare(b, &locale, sensitive)?;
                        if column.descending {
                            value.reverse()
                        } else {
                            value
                        }
                    }
                    (None, Some(_)) => Ordering::Greater,
                    (Some(_), None) => Ordering::Less,
                    (None, None) => Ordering::Equal,
                };
                if !order.is_eq() {
                    return Ok(order);
                }
            }
            Ok(Ordering::Equal)
        };
        let order = indices::ordered(
            rows.len(),
            &compare,
            stable,
            flag("Unique"),
            limit,
            top.is_some(),
        )?;
        Ok(order.into_iter().map(|i| input[i].clone()).collect())
    }
}

fn sort_locale(value: Option<&Value>) -> Result<String, RuntimeError> {
    let Some(name) = value.map(ToString::to_string).filter(|s| !s.is_empty()) else {
        return Ok(culture::current_locale());
    };
    let trimmed = name.trim();
    let lcid = if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse().ok()
    };
    let detail = if let Some(id) = lcid {
        if let Some(locale) = culture::locale_for_lcid(id) {
            return Ok(locale);
        }
        format!("(Parameter 'culture')\n{id} (0x{id:04x}) is an invalid culture identifier.")
    } else if name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Ok(name);
    } else {
        format!("(Parameter 'name')\n{name} is an invalid culture identifier.")
    };
    Err(RuntimeError::Script {
        id: "ParameterBindingFailed,Microsoft.PowerShell.Commands.SortObjectCommand",
        category: "WriteError: (:) [Sort-Object], ParameterBindingException".into(),
        exception_type: "System.Management.Automation.ParameterBindingException",
        message: format!(
            "Cannot bind parameter 'Culture' to the target. Exception setting \"Culture\": \"Culture is not supported. {detail}\""
        ),
    })
}

fn property_value(item: &Value, name: &str) -> Option<Value> {
    let value = member(item, name).ok()?;
    if value.is_null_like()
        && let Value::Object(entries) = item
        && !entries.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
    {
        None
    } else {
        Some(value)
    }
}

fn sort_columns(
    property: Value,
    mut descending: bool,
    input: &[&Value],
) -> Result<Vec<Column>, EngineError> {
    let mut expression = &property;
    if let Value::Object(entries) = &property {
        let entry = |full: &str| {
            entries
                .iter()
                .find(|(key, _)| !key.is_empty() && full.starts_with(&key.to_ascii_lowercase()))
                .map(|(_, value)| value)
        };
        let asc = entry("ascending").map(ops::is_true);
        let desc = entry("descending").map(ops::is_true);
        descending = match (asc, desc) {
            (Some(a), Some(d)) if a == d => descending,
            (Some(a), _) => !a,
            (None, Some(d)) => d,
            _ => descending,
        };
        expression = entry("expression").ok_or(RuntimeError::Script {
            id: "DictionaryKeyIllegal,Microsoft.PowerShell.Commands.SortObjectCommand",
            category: "InvalidArgument: (:) [Sort-Object], NotSupportedException".into(),
            exception_type: "System.NotSupportedException",
            message: "Specified method is not supported.".into(),
        })?;
    }
    let columns = match expression {
        Value::ScriptBlock(source) => vec![Column {
            expression: Expression::Script(parse(&source.to_string_lossy())?.statements),
            descending,
        }],
        value => {
            let name = value.to_ps_string();
            let names = if wildcard::contains_wildcard_characters(name.units()) {
                let matcher = wildcard::Matcher::new(&name, 2)?;
                let mut names = Vec::new();
                for item in input {
                    match item {
                        Value::Object(entries) => {
                            for (name, _) in visible_entries(entries) {
                                if matcher.is_match(Some(&name.as_str().into()))? {
                                    names.push(name.clone());
                                }
                            }
                        }
                        Value::String(_) if matcher.is_match(Some(&"Length".into()))? => {
                            names.push("Length".into())
                        }
                        _ => {}
                    }
                }
                names.sort_by(|a, b| {
                    culture::ordinal_compare(
                        &a.encode_utf16().collect::<Vec<_>>(),
                        &b.encode_utf16().collect::<Vec<_>>(),
                        false,
                    )
                });
                names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
                names
            } else {
                vec![name.to_string_lossy()]
            };
            names
                .into_iter()
                .map(|name| Column {
                    expression: Expression::Property(name),
                    descending,
                })
                .collect()
        }
    };
    Ok(columns)
}
