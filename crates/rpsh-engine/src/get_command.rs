//! Command discovery and the syntax rendered from executable parameter declarations.
use super::{
    ArgumentBinding, BUILTIN_COMMANDS, Engine, EngineError, PreparedArguments, aliases, functions,
    invocation, member, native, string_parameter, wildcards,
};
use rpsh_commands::{MODULES, signature};
use rpsh_runtime::{RuntimeError, Value, ops};

fn binding(key: &str) -> ArgumentBinding {
    match key.to_ascii_lowercase().as_str() {
        "syntax" | "all" | "listimported" | "showcommandinfo" => ArgumentBinding::Switch,
        other => functions::common_binding(other).unwrap_or(ArgumentBinding::Named),
    }
}

impl Engine {
    pub(super) fn get_command(
        &mut self,
        arguments: &PreparedArguments<'_>,
    ) -> Result<Vec<Value>, EngineError> {
        let (positional, named) = self.evaluate_arguments_matching(arguments, binding)?;
        let names = string_parameter(&positional, &named, &["name"]);
        let modules = string_parameter(&[], &named, &["module"]);
        if !named
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("syntax") && ops::is_true(v))
        {
            return self.discover_commands(&names, &modules, &named);
        }

        // PowerShell emits one String for each distinct target, in requested order.
        let mut syntaxes: Vec<(String, String)> = Vec::new();
        let mut collect =
            |engine: &mut Self, info: Value, qualified: Option<&str>| -> Result<(), EngineError> {
                let (target, rendered, direct) = engine.render_command_syntax(&info, qualified)?;
                if let Some((_, syntax)) = syntaxes
                    .iter_mut()
                    .find(|(key, _)| key.eq_ignore_ascii_case(&target))
                {
                    if direct {
                        *syntax = rendered;
                    }
                } else {
                    syntaxes.push((target, rendered));
                }
                Ok(())
            };

        if names.is_empty() {
            for info in self.discover_commands(&[], &modules, &named)? {
                collect(self, info, None)?;
            }
        }
        for name in &names {
            let qualified = name.split_once('\\').filter(|(module, _)| {
                module.eq_ignore_ascii_case("Microsoft.PowerShell.Core")
                    || MODULES.iter().any(|m| m.name.eq_ignore_ascii_case(module))
                    || self
                        .module_functions
                        .keys()
                        .any(|m| m.eq_ignore_ascii_case(module))
            });
            let (requested, matching_modules) = match qualified {
                Some((module, command)) => {
                    let filters = wildcards::Command::GetCommand.patterns(&modules)?;
                    if !filters.is_empty() && !wildcards::matches_any(&filters, module)? {
                        continue;
                    }
                    (command.to_owned(), vec![module.to_owned()])
                }
                None => (name.clone(), modules.clone()),
            };
            let entries = self.discover_commands(
                std::slice::from_ref(&requested),
                &matching_modules,
                &named,
            )?;
            for info in entries {
                collect(self, info, qualified.map(|_| name.as_str()))?;
            }
        }
        Ok(syntaxes
            .into_iter()
            .map(|(_, s)| Value::String(s.into()))
            .collect())
    }

    fn render_command_syntax(
        &mut self,
        info: &Value,
        qualified: Option<&str>,
    ) -> Result<(String, String, bool), EngineError> {
        let name = member(info, "Name")?.to_string();
        let kind = member(info, "CommandType")?.to_string();
        if kind == "Application" {
            let path = member(info, "Path")?.to_string();
            return Ok((path.clone(), path, true));
        }
        let invoked = qualified.unwrap_or(&name);
        let mut target = name.clone();
        if kind == "Alias" {
            let mut visited = std::collections::HashSet::new();
            while let Some(next) = self.discover_alias(&target) {
                if !visited.insert(target.to_ascii_lowercase()) {
                    return Err(RuntimeError::NotImplemented {
                        feature: "Get-Command -Syntax for a cyclic alias",
                    }
                    .into());
                }
                target = next;
            }
        }
        if (kind == "Cmdlet"
            || (kind == "Alias" && !self.functions.contains_key(&target.to_ascii_lowercase())))
            && let Some(signature) = signature::lookup(&target)
        {
            return Ok((
                signature.name.to_owned(),
                signature.syntax(invoked),
                invoked.eq_ignore_ascii_case(signature.name),
            ));
        }
        Err(RuntimeError::NotImplemented {
            feature: "Get-Command -Syntax for a command without a parameter declaration",
        }
        .into())
    }

    fn discover_commands(
        &mut self,
        requested_names: &[String],
        modules: &[String],
        named: &[(String, Value)],
    ) -> Result<Vec<Value>, EngineError> {
        let modules = wildcards::Command::GetCommand.patterns(modules)?;
        let command_type = named
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("commandtype"))
            .map(|(_, value)| value.to_string());
        if command_type
            .as_deref()
            .is_none_or(|kind| kind.eq_ignore_ascii_case("Alias"))
        {
            for name in requested_names {
                if MODULES.iter().any(|module| module.exports_alias(name)) {
                    self.discover_alias(name);
                }
            }
        }
        let wanted = wildcards::Command::GetCommand.patterns(requested_names)?;
        let word_patterns = |field: &str| {
            wildcards::Command::GetCommand.patterns(&string_parameter(&[], named, &[field]))
        };
        let verb = word_patterns("verb")?;
        let noun = word_patterns("noun")?;
        let mut names: Vec<(String, &'static str)> = self
            .functions
            .values()
            .map(|function| (function.name.clone(), function.kind()))
            .collect();
        for builtin in BUILTIN_COMMANDS {
            names.push(((*builtin).to_owned(), "Cmdlet"));
        }
        names.extend(
            self.aliases
                .iter()
                .map(|alias| (alias.name.to_owned(), "Alias")),
        );
        let imported_only = named
            .iter()
            .any(|(key, value)| key.eq_ignore_ascii_case("ListImported") && ops::is_true(value));
        if !imported_only && self.autoload_unqualified() {
            for module in MODULES.iter().filter(|module| !self.builtin_loaded(module)) {
                for &(name, _) in module.aliases {
                    if self.aliases.get(name).is_none() {
                        names.push((name.to_owned(), "Alias"));
                    }
                }
            }
        }
        names.sort_by(|left, right| left.0.cmp(&right.0));
        let mut outputs = Vec::new();
        for (name, kind) in names {
            let matches = wanted.is_empty() || wildcards::matches_any(&wanted, &name)?;
            let type_matches = command_type
                .as_deref()
                .is_none_or(|wanted| wanted.eq_ignore_ascii_case(kind));
            let module_match = if modules.is_empty() {
                true
            } else {
                let module = match kind {
                    "Cmdlet" => rpsh_commands::signature::lookup(&name)
                        .map(|signature| signature.module)
                        .or_else(|| {
                            MODULES
                                .iter()
                                .find(|module| module.exports_cmdlet(&name.to_ascii_lowercase()))
                                .map(|module| module.name)
                        }),
                    "Function" | "Filter" => self
                        .module_functions
                        .iter()
                        .find(|(_, functions)| {
                            functions.iter().any(|f| f.eq_ignore_ascii_case(&name))
                        })
                        .map(|(module, _)| module.as_str()),
                    "Alias" => self
                        .aliases
                        .get(&name)
                        .and_then(|alias| alias.module)
                        .map(|m| m.name),
                    _ => None,
                };
                match module {
                    Some(module) => wildcards::matches_any(&modules, module)?,
                    None => false,
                }
            };
            let (name_verb, name_noun) = match name.split_once('-') {
                Some((verb, noun)) => (verb, noun),
                None => ("", name.as_str()),
            };
            let verb_matches = verb.is_empty() || wildcards::matches_any(&verb, name_verb)?;
            let noun_matches = noun.is_empty() || wildcards::matches_any(&noun, name_noun)?;
            if matches && type_matches && verb_matches && noun_matches && module_match {
                if kind == "Alias" {
                    if let Some(alias) = self.aliases.get(&name) {
                        outputs.push(alias.value());
                    } else if let Some(module) =
                        MODULES.iter().find(|module| module.exports_alias(&name))
                    {
                        outputs.push(aliases::available_export(&name, module));
                    }
                } else if matches!(kind, "Function" | "Filter")
                    && let Some(function) = self.functions.info(&name.to_ascii_lowercase())
                {
                    outputs.push(function.info());
                } else if let Some(info) = functions::declared_info(&name, kind) {
                    outputs.push(info);
                } else {
                    outputs.push(functions::command_info(&name, kind));
                }
            }
        }
        if !requested_names.is_empty()
            && modules.is_empty()
            && verb.is_empty()
            && noun.is_empty()
            && command_type.as_deref().is_none_or(|kind| {
                kind.eq_ignore_ascii_case("Application")
                    || kind.eq_ignore_ascii_case("ExternalScript")
            })
        {
            let all = command_type.is_some()
                || named
                    .iter()
                    .any(|(key, value)| key.eq_ignore_ascii_case("All") && ops::is_true(value));
            for name in requested_names {
                let already_found = !all
                    && outputs.iter().any(|entry| {
                        member(entry, "Name")
                            .is_ok_and(|value| value.to_string().eq_ignore_ascii_case(name))
                    });
                if already_found
                    || rpsh_runtime::wildcard::contains_wildcard_characters(name.encode_utf16())
                {
                    continue;
                }
                let script = invocation::script_name(name);
                let kind = if script {
                    "ExternalScript"
                } else {
                    "Application"
                };
                if command_type
                    .as_deref()
                    .is_some_and(|wanted| !wanted.eq_ignore_ascii_case(kind))
                {
                    continue;
                }
                if script {
                    outputs.extend(
                        self.script_paths(name, all)
                            .into_iter()
                            .map(invocation::script_info),
                    );
                } else {
                    outputs.extend(
                        self.application_paths(name, all)
                            .into_iter()
                            .map(native::application_info),
                    );
                }
            }
        }
        Ok(outputs)
    }
}
