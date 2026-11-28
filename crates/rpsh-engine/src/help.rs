//! Default cmdlet help synthesized from registered PowerShell parameter declarations.
pub(super) mod format;
pub(super) mod topics;

use super::{
    CallArgument, Engine, EngineError, PreparedArguments, ScriptException, cmdlet, command_name,
    environment, error_record, member, version_value,
};
use rpsh_commands::signature::{self, GET_HELP, Parameter, ParameterType, Signature};
use rpsh_runtime::{CollectionType, RuntimeError, Value, ops};

const EXTENDED: &str = "ExtendedCmdletHelpInfo";
static TYPE_NAMES: CollectionType = CollectionType {
    full_name: "System.Management.Automation.Runspaces.ConsolidatedString",
    display_name: "System.Management.Automation.Runspaces.ConsolidatedString",
    element: "System.String",
    read_only: false,
    backing: None,
};

fn string(s: impl AsRef<str>) -> Value {
    Value::String(s.as_ref().into())
}

fn node(kind: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut fields: Vec<_> = fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    let names = vec![string(format!("{EXTENDED}#{kind}"))];
    fields.push((
        "__PSTypeNames".into(),
        Value::collection(&TYPE_NAMES, names),
    ));
    Value::object(fields)
}

fn ty(name: &str) -> Value {
    node("type", vec![("name", string(name))])
}

fn pipeline_input(param: &Parameter) -> String {
    let mut modes = Vec::new();
    if param.from_pipeline {
        modes.push("ByValue");
    }
    if param.from_property {
        modes.push("ByPropertyName");
    }
    if param.remaining {
        modes.push("FromRemainingArguments");
    }
    if modes.is_empty() {
        "false".into()
    } else {
        format!("true ({})", modes.join(", "))
    }
}

fn parameter(param: &Parameter, syntax: bool) -> Value {
    let name = param.ty.display_name();
    let mut fields = vec![
        ("name", string(param.name)),
        ("required", string(param.mandatory.to_string())),
        ("pipelineInput", string(pipeline_input(param))),
        ("isDynamic", string("false")),
        ("globbing", string("false")),
        (
            "parameterSetName",
            string(if param.sets.is_empty() {
                "(All)".into()
            } else {
                param.sets.join(", ")
            }),
        ),
    ];
    if param.ty != ParameterType::Switch {
        // An extended System.String keeps these PowerShell note properties on the scalar.
        fields.push((
            "parameterValue",
            Value::object(vec![
                ("__TypeName".into(), string("System.String")),
                (
                    "__PSTypeNames".into(),
                    Value::collection(
                        &TYPE_NAMES,
                        vec![string("System.String"), string("System.Object")],
                    ),
                ),
                ("__Text".into(), string(name)),
                ("variableLength".into(), Value::Bool(name.ends_with("[]"))),
                ("required".into(), string("true")),
                ("Length".into(), Value::Int32(name.len() as i32)),
            ]),
        ));
    }
    fields.push(("type", ty(name)));
    fields.push((
        "position",
        string(
            param
                .position
                .map_or_else(|| "Named".into(), |n| n.to_string()),
        ),
    ));
    fields.push((
        "aliases",
        string(if param.aliases.is_empty() {
            "None".into()
        } else {
            param.aliases.join(", ")
        }),
    ));
    if syntax && !param.valid_values.is_empty() {
        let values = param.valid_values.iter().map(string).collect();
        fields.push((
            "parameterValueGroup",
            node(
                "parameterValueGroup",
                vec![("parameterValue", Value::array(values))],
            ),
        ));
    }
    node("parameter", fields)
}

fn core_snapin() -> Value {
    let home = environment::ps_home();
    let list = |entries: &[&str]| Value::array(entries.iter().map(string).collect());
    let version = |text: &str, build: i32| {
        let Value::Object(entries) = version_value(text.into()) else {
            unreachable!()
        };
        let mut fields = entries.as_ref().clone();
        fields.push(("MajorRevision".into(), Value::Int16((build >> 16) as i16)));
        fields.push(("MinorRevision".into(), Value::Int16(build as i16)));
        Value::object(fields)
    };
    Value::object(vec![
        (
            "__TypeName".into(),
            string("System.Management.Automation.PSSnapInInfo"),
        ),
        ("__Text".into(), string("Microsoft.PowerShell.Core")),
        ("Name".into(), string("Microsoft.PowerShell.Core")),
        ("IsDefault".into(), Value::Bool(true)),
        ("ApplicationBase".into(), string(home.to_string_lossy())),
        (
            "AssemblyName".into(),
            string(
                "System.Management.Automation, Version=7.7.0.500, Culture=neutral, PublicKeyToken=31bf3856ad364e35",
            ),
        ),
        (
            "ModuleName".into(),
            string(
                home.join("System.Management.Automation.dll")
                    .to_string_lossy(),
            ),
        ),
        ("PSVersion".into(), version("7.7.0", -1)),
        ("Version".into(), version("7.7.0.500", 500)),
        ("Types".into(), list(&["types.ps1xml", "typesv3.ps1xml"])),
        (
            "Formats".into(),
            list(&[
                "Certificate.format.ps1xml",
                "DotNetTypes.format.ps1xml",
                "FileSystem.format.ps1xml",
                "Help.format.ps1xml",
                "HelpV3.format.ps1xml",
                "PowerShellCore.format.ps1xml",
                "PowerShellTrace.format.ps1xml",
                "Registry.format.ps1xml",
            ]),
        ),
        ("Description".into(), Value::Null),
        ("Vendor".into(), Value::Null),
        ("LogPipelineExecutionDetails".into(), Value::Bool(false)),
    ])
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum View {
    Default,
    Detailed,
    Examples,
    Full,
}
impl View {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Detailed => "Detailed",
            Self::Examples => "Examples",
            Self::Full => "Full",
        }
    }
}

fn help_info(sig: &Signature, aliases: &str, view: View) -> Value {
    let (verb, noun) = sig.name.split_once('-').unwrap_or(("", sig.name));
    let syntax = sig
        .sets()
        .iter()
        .map(|set| {
            let params = sig
                .parameters_in_display_order(set)
                .into_iter()
                .map(|p| parameter(p, true))
                .collect();
            node(
                "syntaxItem",
                vec![
                    ("name", string(sig.name)),
                    ("CommonParameters", Value::Bool(true)),
                    ("parameter", Value::array(params)),
                ],
            )
        })
        .collect();
    let mut params: Vec<_> = sig.parameters.iter().collect();
    params.sort_by_key(|p| p.name);
    let details = Value::array(params.into_iter().map(|p| parameter(p, false)).collect());
    let mut input_types = Vec::new();
    for p in sig.parameters {
        if (p.from_pipeline || p.from_property) && !input_types.contains(&p.ty.full_name()) {
            input_types.push(p.ty.full_name());
        }
    }
    if input_types.is_empty() {
        input_types.push("None");
    }
    let inputs = format!("{}\n", input_types.join("\n"));
    let remarks = rpsh_messages::format(
        "HelpDisplayStrings/GetLatestHelpContent",
        &[sig.name, sig.help_uri],
    );
    let types = [EXTENDED, "CmdletHelpInfo", "HelpInfo"];
    let mut names = Vec::new();
    if view != View::Default {
        names.extend(
            types
                .iter()
                .map(|n| string(format!("{n}#{}View", view.label()))),
        );
    }
    names.extend(types.iter().map(string));
    let snapin = if sig.module == "Microsoft.PowerShell.Core" {
        core_snapin()
    } else {
        Value::Null
    };
    let fields = [
        ("CommonParameters", Value::Bool(true)),
        (
            "details",
            node(
                "details",
                vec![
                    ("name", string(sig.name)),
                    ("noun", string(noun)),
                    ("verb", string(verb)),
                ],
            ),
        ),
        (
            "Syntax",
            node("syntax", vec![("syntaxItem", Value::array(syntax))]),
        ),
        (
            "parameters",
            node("parameters", vec![("parameter", details)]),
        ),
        (
            "inputTypes",
            node(
                "inputTypes",
                vec![("inputType", node("inputType", vec![("type", ty(&inputs))]))],
            ),
        ),
        (
            "relatedLinks",
            node(
                "relatedLinks",
                vec![(
                    "navigationLink",
                    Value::array(vec![node(
                        "navigationLinks",
                        vec![("uri", string(sig.help_uri))],
                    )]),
                )],
            ),
        ),
        (
            "returnValues",
            node(
                "returnValues",
                vec![(
                    "returnValue",
                    node("returnValue", vec![("type", ty("System.Object"))]),
                )],
            ),
        ),
        ("aliases", string(aliases)),
        ("remarks", string(remarks)),
        ("PSSnapIn", snapin),
        ("alertSet", Value::Null),
        ("description", Value::Null),
        ("examples", Value::Null),
        ("Synopsis", string(sig.syntax(sig.name))),
        ("ModuleName", string(sig.module)),
        ("nonTerminatingErrors", string("")),
        (
            "xmlns:command",
            string("http://schemas.microsoft.com/maml/dev/command/2004/10"),
        ),
        (
            "xmlns:dev",
            string("http://schemas.microsoft.com/maml/dev/2004/10"),
        ),
        (
            "xmlns:maml",
            string("http://schemas.microsoft.com/maml/2004/10"),
        ),
        ("Name", string(sig.name)),
        ("Category", string("Cmdlet")),
        ("Component", Value::Null),
        ("Role", Value::Null),
        ("Functionality", Value::Null),
        ("__HelpView", string(view.label())),
        ("__PSTypeNames", Value::collection(&TYPE_NAMES, names)),
    ];
    Value::object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

pub(super) fn requested(args: &PreparedArguments<'_>) -> bool {
    args.expanded
        .iter()
        .any(|(_, arg)| matches!(arg,CallArgument::Parameter{name,..} if name=="?"))
}

pub(super) fn proxy_comments(value: &Value) -> Option<String> {
    format::comments(value)
}

fn strings(value: &Value) -> Vec<String> {
    value.sequence().map_or_else(
        || vec![value.to_string()],
        |items| items.iter().map(ToString::to_string).collect(),
    )
}

fn allows_topic(bound: &rpsh_commands::framework::BoundParameters, alias: bool) -> bool {
    if let Some(value) = bound.get("Category") {
        let categories = strings(value);
        if !categories.is_empty()
            && !categories.iter().any(|s| {
                s.eq_ignore_ascii_case("Cmdlet")
                    || s.eq_ignore_ascii_case("All")
                    || (alias && s.eq_ignore_ascii_case("Alias"))
            })
        {
            return false;
        }
    }
    ["Component", "Role", "Functionality"].iter().all(|key| {
        bound.get(key).is_none_or(|v| {
            let patterns = strings(v);
            patterns.is_empty() || patterns.iter().any(|s| ops::wildcard_match("", s, false))
        })
    })
}

impl Engine {
    fn cmdlet_help(&self, sig: &Signature, view: View) -> Value {
        let aliases = self.aliases.names_for_command(sig.name);
        let names = if aliases.is_empty() {
            "None".into()
        } else {
            aliases.join("\n")
        };
        help_info(sig, &format!("{names}\n"), view)
    }

    pub(super) fn shorthand_help(
        &mut self,
        sig: &Signature,
        input: Option<&[Value]>,
    ) -> Result<Vec<Value>, EngineError> {
        if let Some(input) = input {
            for item in input {
                self.help_input_error(item, &[])?;
            }
            return Ok(vec![]);
        }
        Ok(vec![self.cmdlet_help(sig, View::Default)])
    }

    fn help_input_error(
        &mut self,
        value: &Value,
        common: &[(String, Value)],
    ) -> Result<(), EngineError> {
        let error = cmdlet::binding_record(&GET_HELP, "InputObjectNotBound", &[], value.clone());
        self.cmdlet_result::<()>(
            Err(EngineError::Script(ScriptException::new(error))),
            common,
        )?;
        Ok(())
    }

    pub(super) fn get_help(
        &mut self,
        args: &PreparedArguments<'_>,
        pipeline: Option<&[Value]>,
    ) -> Result<Vec<Value>, EngineError> {
        let call = cmdlet::bind(&GET_HELP, args)?;
        if !GET_HELP.accepts_named(
            GET_HELP
                .parameters
                .iter()
                .filter(|p| call.bound.get(p.name).is_some())
                .map(|p| p.name),
        ) {
            return Err(cmdlet::binding_error(
                &GET_HELP,
                "AmbiguousParameterSet",
                &[],
            ));
        }
        let enabled = |name| call.bound.get(name).is_some_and(ops::is_true);
        let view = if enabled("Detailed") {
            View::Detailed
        } else if enabled("Examples") {
            View::Examples
        } else if enabled("Full") {
            View::Full
        } else {
            View::Default
        };
        let categories = call.bound.get("Category").map(strings).unwrap_or_default();
        let category_is = |category: &str| {
            categories
                .iter()
                .any(|name| name.eq_ignore_ascii_case(category))
        };
        let files_allowed = categories.is_empty() || category_is("All") || category_is("HelpFile");
        let files_only = !categories.is_empty()
            && categories.iter().all(|s| {
                s.eq_ignore_ascii_case("HelpFile") || s.eq_ignore_ascii_case("DefaultHelp")
            });
        let filters = ["Component", "Role", "Functionality"];
        if !categories.is_empty()
            && ![
                "All",
                "Alias",
                "Cmdlet",
                "ExternalScript",
                "Filter",
                "Function",
                "ScriptCommand",
            ]
            .iter()
            .any(|c| category_is(c))
            && let Some(key) = ["Parameter", "Component", "Role", "Functionality"]
                .iter()
                .find(|name| call.bound.get(name).is_some())
        {
            let reason =
                rpsh_messages::format("HelpErrors/ParamNotSupported", &[&format!("-{key}")]);
            return Err(EngineError::Script(ScriptException::new(error_record(
                "Argument,Microsoft.PowerShell.Commands.GetHelpCommand",
                "InvalidArgument: (:) [Get-Help], PSArgumentException",
                "System.Management.Automation.PSArgumentException",
                &reason,
                &reason,
            ))));
        }
        let bound = call.bound.get("Name");
        let mut names = Vec::new();
        match pipeline {
            Some(items) => {
                for item in items {
                    let property = match item {
                        Value::Object(_) | Value::Type(_) => member(item, "Name")?,
                        _ => Value::Null,
                    };
                    if bound.is_some() || property.is_null_like() {
                        self.help_input_error(item, &call.common)?;
                    } else {
                        names.push(property.to_string());
                    }
                }
            }
            None => {
                names.push(bound.map_or_else(
                    || {
                        if categories.is_empty() {
                            "default".into()
                        } else {
                            "*".into()
                        }
                    },
                    ToString::to_string,
                ));
            }
        }
        let mut result = Vec::new();
        for name in names {
            let wildcard = name.contains(['*', '?', '[']);
            if wildcard && files_allowed && !files_only {
                let matches = |s: &str| ops::wildcard_match(s, &name, false);
                if rpsh_commands::CORE_CMDLETS.iter().any(|s| matches(s))
                    || rpsh_commands::MODULES
                        .iter()
                        .any(|m| m.cmdlet_names().iter().any(|s| matches(s)))
                    || self.functions.keys().any(|s| matches(s))
                    || self.aliases.iter().any(|a| matches(a.name))
                {
                    return Err(RuntimeError::NotImplemented {
                        feature: "combined command and file help search",
                    }
                    .into());
                }
            }
            let alias = self.resolve_alias(&name)?;
            let allowed = allows_topic(&call.bound, alias.is_some());
            let target = alias.unwrap_or_else(|| name.to_owned());
            let (module, target) = target
                .rsplit_once('\\')
                .map_or((None, target.as_str()), |(m, n)| (Some(m), n));
            let sig = signature::lookup(target)
                .filter(|s| module.is_none_or(|m| s.module.eq_ignore_ascii_case(m)));
            let sig = sig.filter(|_| allowed);
            if sig.is_none() && files_allowed && filters.iter().all(|k| call.bound.get(k).is_none())
            {
                let cache = self.help_topics.get_or_insert_with(Default::default);
                let found = cache.search(&name, &self.variables, view);
                if !found.is_empty() {
                    if enabled("Online") {
                        if found.len() > 1 {
                            return Err(RuntimeError::NotImplemented {
                                feature: "streaming help matches before an online-help error",
                            }
                            .into());
                        }
                        let message = rpsh_messages::text("HelpErrors/NoURIFound");
                        return Err(EngineError::Script(ScriptException::new(error_record(
                            "InvalidOperation,Microsoft.PowerShell.Commands.GetHelpCommand",
                            "InvalidOperation: (:) [Get-Help], PSInvalidOperationException",
                            "System.Management.Automation.PSInvalidOperationException",
                            message,
                            message,
                        ))));
                    }
                    if let Some(patterns) = call.bound.get("Parameter") {
                        if found.len() == 1
                            && let Some(first) = strings(patterns).first()
                        {
                            let reason = rpsh_messages::format("HelpErrors/NoParmsFound", &[first]);
                            self.help_error("NoParmsFound","InvalidArgument: (System.Management.A…on.HelpFileHelpInfo:HelpFileHelpInfo) [Get-Help], PSArgumentException",
                                "System.Management.Automation.PSArgumentException",&reason,&call.common)?;
                        }
                    } else {
                        result.extend(found);
                    }
                    continue;
                }
            }
            let about = name.to_ascii_lowercase().starts_with("about_");
            if wildcard && (files_only || about) {
                continue;
            }
            let Some(sig) = sig else {
                if !about
                    && !files_only
                    && allowed
                    && (super::BUILTIN_COMMANDS
                        .iter()
                        .any(|n| n.eq_ignore_ascii_case(command_name(&name)))
                        || rpsh_commands::MODULES
                            .iter()
                            .any(|m| m.exports_cmdlet(&target.to_ascii_lowercase()))
                        || self.functions.contains_key(&name.to_ascii_lowercase())
                        || name.contains(['*', '?', '[']))
                {
                    return Err(RuntimeError::NotImplemented {
                        feature: "help for this topic or search",
                    }
                    .into());
                }
                self.help_error(
                    "HelpNotFound",
                    "ResourceUnavailable: (:) [Get-Help], HelpNotFoundException",
                    "System.Management.Automation.HelpNotFoundException",
                    &rpsh_messages::format("HelpErrors/HelpNotFound", &[&name]),
                    &call.common,
                )?;
                continue;
            };
            if enabled("Online") {
                return Err(RuntimeError::NotImplemented {
                    feature: "Get-Help -Online for a web topic",
                }
                .into());
            }
            let info = self.cmdlet_help(sig, view);
            if let Some(patterns) = call.bound.get("Parameter") {
                let patterns = strings(patterns);
                let mut matches = 0;
                let list = member(&member(&info, "Parameters")?, "Parameter")?;
                let params = list.sequence().expect("help parameter list");
                for pattern in &patterns {
                    for p in params.iter() {
                        let name = member(p, "name")?.to_string();
                        if ops::wildcard_match(&name, pattern, false) {
                            result.push(p.clone());
                            matches += 1;
                        }
                    }
                }
                if matches == 0
                    && let Some(first) = patterns.first()
                {
                    let detail = rpsh_messages::format("HelpErrors/NoParmsFound", &[first]);
                    self.help_error("NoParmsFound",
                        "InvalidArgument: (System.Management.A…iderCommandHelpInfo:ProviderCommandHelpInfo) [Get-Help], PSArgumentException",
                        "System.Management.Automation.PSArgumentException", &detail, &call.common)?;
                }
            } else {
                result.push(info);
            }
        }
        Ok(result)
    }

    fn help_error(
        &mut self,
        id: &str,
        category: &str,
        ty: &str,
        message: &str,
        common: &[(String, Value)],
    ) -> Result<(), EngineError> {
        let record = error_record(
            &format!("{id},{}", GET_HELP.implementation),
            category,
            ty,
            message,
            message,
        );
        self.cmdlet_result::<()>(
            Err(EngineError::Script(ScriptException::new(record))),
            common,
        )?;
        Ok(())
    }
}
