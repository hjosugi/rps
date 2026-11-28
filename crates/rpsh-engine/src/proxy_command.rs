//! Render PowerShell proxy functions from the same CommandMetadata exposed to scripts.
use super::{hidden_type, metadata, method_error, overload_error};
use rpsh_runtime::{RuntimeError, Value, ops};
use std::fmt::Write as _;

fn property<'a>(object: &'a Value, name: &str) -> Option<&'a Value> {
    let Value::Object(fields) = object else {
        return None;
    };
    fields
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

fn text(object: &Value, name: &str) -> String {
    property(object, name)
        .map(ToString::to_string)
        .unwrap_or_default()
}

fn boolean(object: &Value, name: &str) -> bool {
    property(object, name).is_some_and(ops::is_true)
}

/// PowerShell accepts all five of these characters as single quote delimiters.
fn quoted(value: &str) -> String {
    let mut result = String::from("'");
    for c in value.chars() {
        result.push(c);
        if matches!(c, '\'' | '‘' | '’' | '‚' | '‛') {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

fn binding(meta: &Value) -> String {
    let mut options = Vec::new();
    let default_set = text(meta, "DefaultParameterSetName");
    if !default_set.is_empty() {
        options.push(format!("DefaultParameterSetName={}", quoted(&default_set)));
    }
    if boolean(meta, "SupportsShouldProcess") {
        options.push("SupportsShouldProcess=$true".into());
        options.push(format!(
            "ConfirmImpact={}",
            quoted(&text(meta, "ConfirmImpact"))
        ));
    }
    for name in ["SupportsPaging", "SupportsTransactions"] {
        if boolean(meta, name) {
            options.push(format!("{name}=$true"));
        }
    }
    if !boolean(meta, "PositionalBinding") {
        options.push("PositionalBinding=$false".into());
    }
    let help = text(meta, "HelpUri");
    if !help.is_empty() {
        options.push(format!("HelpUri={}", quoted(&help)));
    }
    let remoting = text(meta, "RemotingCapability");
    if remoting != "PowerShell" {
        options.push(format!("RemotingCapability={}", quoted(&remoting)));
    }
    format!("[CmdletBinding({})]", options.join(", "))
}

fn parameter_set(name: &str, set: &Value) -> Option<String> {
    let mut options = Vec::new();
    if name != "__AllParameterSets" {
        options.push(format!("ParameterSetName={}", quoted(name)));
    }
    if boolean(set, "IsMandatory") {
        options.push("Mandatory=$true".into());
    }
    if let Some(n) = property(set, "Position")
        && *n != Value::Int32(i32::MIN)
    {
        options.push(format!("Position={n}"));
    }
    for flag in [
        "ValueFromPipeline",
        "ValueFromPipelineByPropertyName",
        "ValueFromRemainingArguments",
    ] {
        if boolean(set, flag) {
            options.push(format!("{flag}=$true"));
        }
    }
    let help = text(set, "HelpMessage");
    if !help.is_empty() {
        options.push(format!("HelpMessage={}", quoted(&help)));
    }
    (!options.is_empty()).then(|| format!("[Parameter({})]", options.join(", ")))
}

fn type_literal(name: &str) -> String {
    if let Some(element) = name.strip_suffix("[]") {
        return format!("{}[]", type_literal(element));
    }
    match name {
        "System.Management.Automation.SwitchParameter" => "switch",
        "System.Management.Automation.PSObject" => "psobject",
        "System.Management.Automation.ScriptBlock" => "scriptblock",
        "System.Collections.Hashtable" => "hashtable",
        "System.Boolean" => "bool",
        "System.Byte" => "byte",
        "System.SByte" => "sbyte",
        "System.Char" => "char",
        "System.Decimal" => "decimal",
        "System.Double" => "double",
        "System.Int16" => "short",
        "System.Int32" => "int",
        "System.Int64" => "long",
        "System.UInt16" => "ushort",
        "System.UInt32" => "uint",
        "System.UInt64" => "ulong",
        "System.String" => "string",
        other => other,
    }
    .to_owned()
}

fn parameter(out: &mut String, name: &str, param: &Value) {
    let mut line = |value: &str| {
        out.push_str("\n    ");
        out.push_str(value);
    };
    if let Some(Value::Object(sets)) = property(param, "ParameterSets") {
        for (name, set) in metadata::dictionary_entries(sets) {
            if let Some(value) = parameter_set(name, set) {
                line(&value);
            }
        }
    }
    if let Some(aliases) = property(param, "Aliases").and_then(Value::sequence)
        && !aliases.is_empty()
    {
        let names: Vec<_> = aliases.iter().map(|v| quoted(&v.to_string())).collect();
        line(&format!("[Alias({})]", names.join(",")));
    }
    if let Some(attrs) = property(param, "Attributes").and_then(Value::sequence) {
        for attr in attrs.iter() {
            let ty = hidden_type(attr).unwrap_or_default();
            let simple = ty.rsplit('.').next().unwrap_or(&ty);
            match simple {
                "AllowEmptyCollectionAttribute"
                | "AllowNullAttribute"
                | "AllowEmptyStringAttribute"
                | "ValidateNotNullAttribute"
                | "ValidateNotNullOrEmptyAttribute" => {
                    line(&format!("[{}()]", simple.trim_end_matches("Attribute")));
                }
                "ValidateRangeAttribute" => {
                    line(&format!(
                        "[ValidateRange({}, {})]",
                        text(attr, "MinRange"),
                        text(attr, "MaxRange")
                    ));
                }
                _ => {}
            }
        }
    }
    let ty = text(param, "ParameterType");
    if !ty.is_empty() {
        line(&format!("[{}]", type_literal(&ty)));
    }
    let variable = name
        .replace('`', "``")
        .replace('}', "`}")
        .replace('{', "`{");
    line(&format!("${{{variable}}}"));
}

fn parameters(meta: &Value) -> String {
    let mut result = String::new();
    if let Some(Value::Object(fields)) = property(meta, "Parameters") {
        for (name, param) in metadata::dictionary_entries(fields) {
            if !result.is_empty() {
                result.push_str(",\n");
            }
            parameter(&mut result, name, param);
        }
    }
    result
}

fn begin(target: &str, method: &str, count: usize) -> Result<String, RuntimeError> {
    if target.is_empty() {
        return Err(method_error::invocation(
            "InvalidOperationException",
            method,
            count,
            rpsh_messages::text("ProxyCommandStrings/CommandMetadataMissingCommandName"),
        ));
    }
    Ok(BEGIN.replacen("{target}", &quoted(target), 1))
}

fn create(meta: &Value, args: &[Value], target: &str) -> Result<String, RuntimeError> {
    let mut out = format!("{}\nparam({})\n\n", binding(meta), parameters(meta));
    for (name, block) in [
        ("begin", begin(target, "Create", args.len())?.as_str()),
        ("process", PROCESS),
        ("end", END),
        ("clean", CLEAN),
    ] {
        writeln!(out, "{name}\n{{{block}}}").unwrap();
        if name != "clean" {
            out.push('\n');
        }
    }
    let comment = args
        .get(1)
        .map(|v| v.to_ps_string().to_string_lossy())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            format!("\n.ForwardHelpTargetName {target}\n.ForwardHelpCategory Cmdlet\n")
        });
    let comment = comment.replace("<#", "<`#").replace("#>", "#`>");
    writeln!(out, "<#\n{comment}\n#>").unwrap();
    Ok(out)
}

fn to_metadata(source: &Value, method: &str) -> Result<Value, RuntimeError> {
    let ty = metadata::COMMAND_METADATA;
    let clr_type = super::get_type(source).to_string();
    let displayed = if source.sequence().is_some() {
        clr_type.clone()
    } else {
        source.to_string()
    };
    let cause = match source.base_value() {
        Value::Type(_) => return metadata::construct(std::slice::from_ref(source)),
        Value::Object(_)
            if matches!(
                hidden_type(source).as_deref(),
                Some(
                    super::functions::CMDLET
                        | super::functions::FUNCTION
                        | super::functions::FILTER
                        | "System.Management.Automation.AliasInfo"
                )
            ) =>
        {
            return metadata::construct(std::slice::from_ref(source));
        }
        Value::Bool(_) | Value::Char(_) => {
            Some(format!("Invalid cast from '{clr_type}' to '{ty}'."))
        }
        value if matches!(value, Value::String(_)) || super::is_numeric_value(value) => {
            let path =
                std::path::absolute(&displayed).map_err(|_| RuntimeError::NotImplemented {
                    feature: "resolving a script metadata path",
                })?;
            if path.exists() {
                return metadata::construct(&[Value::String(displayed.into())]);
            }
            Some(format!("Could not find file '{}'.", path.display()))
        }
        _ => None,
    };
    let detail = match cause {
        Some(reason) => rpsh_messages::format(
            "ExtendedTypeSystem/InvalidCastExceptionWithInnerException",
            &[&displayed, ty, &reason],
        ),
        None => rpsh_messages::format(
            "ExtendedTypeSystem/InvalidCastException",
            &[&displayed, &clr_type, ty],
        ),
    };
    Err(method_error::argument_error(
        method,
        "commandMetadata",
        ty,
        &displayed,
        &detail,
    ))
}

pub(super) fn invoke(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    let method = match name {
        "create" => "Create",
        "getcmdletbindingattribute" => "GetCmdletBindingAttribute",
        "getparamblock" => "GetParamBlock",
        "getbegin" => "GetBegin",
        "getdynamicparam" => "GetDynamicParam",
        "getprocess" => "GetProcess",
        "getend" => "GetEnd",
        "getclean" => "GetClean",
        "gethelpcomments" => "GetHelpComments",
        _ => {
            return Err(RuntimeError::NotImplemented {
                feature: "this ProxyCommand method",
            });
        }
    };
    if args.is_empty() || args.len() > if method == "Create" { 3 } else { 1 } {
        return Err(overload_error(method, args.len()));
    }
    let source = &args[0];
    if method == "GetHelpComments" {
        let (id, message) = if source.is_null_like() {
            (
                "ArgumentNullException",
                "Value cannot be null. (Parameter 'help')",
            )
        } else {
            (
                "InvalidOperationException",
                rpsh_messages::text("ProxyCommandStrings/HelpInfoObjectRequired"),
            )
        };
        return Err(method_error::invocation(id, method, 1, message));
    }
    if source.is_null_like() {
        let param = if method == "GetClean" {
            "commandMetadata"
        } else {
            "commandMetaData"
        };
        let message = rpsh_messages::format("AutomationExceptions/ArgumentNull", &[param]);
        return Err(method_error::invocation(
            "PSArgumentNullException",
            method,
            args.len(),
            &message,
        ));
    }
    let converted;
    let meta = if hidden_type(source).as_deref() == Some(metadata::COMMAND_METADATA) {
        source
    } else {
        converted = to_metadata(source, method)?;
        &converted
    };
    let target = text(meta, "__WrappedCommand");
    let value = match method {
        "Create" => create(meta, args, &target)?,
        "GetCmdletBindingAttribute" => binding(meta),
        "GetParamBlock" => parameters(meta),
        "GetBegin" => begin(&target, method, 1)?,
        "GetDynamicParam" => DYNAMIC.replacen("{target}", &quoted(&target), 1),
        "GetProcess" => PROCESS.into(),
        "GetEnd" => END.into(),
        "GetClean" => CLEAN.into(),
        _ => unreachable!(),
    };
    Ok(Value::String(value.into()))
}

const BEGIN: &str = r"
    try {
        $outBuffer = $null
        if ($PSBoundParameters.TryGetValue('OutBuffer', [ref]$outBuffer))
        {
            $PSBoundParameters['OutBuffer'] = 1
        }

        $wrappedCmd = $ExecutionContext.InvokeCommand.GetCommand({target}, [System.Management.Automation.CommandTypes]::Cmdlet)
        $scriptCmd = {& $wrappedCmd @PSBoundParameters }

        $steppablePipeline = $scriptCmd.GetSteppablePipeline($myInvocation.CommandOrigin)
        $steppablePipeline.Begin($PSCmdlet)
    } catch {
        throw
    }
";

const DYNAMIC: &str = r"
    try {
        $targetCmd = $ExecutionContext.InvokeCommand.GetCommand({target}, [System.Management.Automation.CommandTypes]::Cmdlet, $PSBoundParameters)
        $dynamicParams = @($targetCmd.Parameters.GetEnumerator() | Microsoft.PowerShell.Core\Where-Object { $_.Value.IsDynamic })
        if ($dynamicParams.Length -gt 0)
        {
            $paramDictionary = [Management.Automation.RuntimeDefinedParameterDictionary]::new()
            foreach ($param in $dynamicParams)
            {
                $param = $param.Value

                if(-not $MyInvocation.MyCommand.Parameters.ContainsKey($param.Name))
                {
                    $dynParam = [Management.Automation.RuntimeDefinedParameter]::new($param.Name, $param.ParameterType, $param.Attributes)
                    $paramDictionary.Add($param.Name, $dynParam)
                }
            }

            return $paramDictionary
        }
    } catch {
        throw
    }
";

const PROCESS: &str = r"
    try {
        $steppablePipeline.Process($_)
    } catch {
        throw
    }
";

const END: &str = r"
    try {
        $steppablePipeline.End()
    } catch {
        throw
    }
";

const CLEAN: &str = r"
    if ($null -ne $steppablePipeline) {
        $steppablePipeline.Clean()
    }
";
