//! Expand a splatted variable at the invocation site. Upstream PipelineOps.Splat
//! treats IDictionary first and enumerates strings as UTF-16 characters.
use super::{
    BoundArguments, Engine, EngineError, FunctionDef, ScriptException, error_record, hidden_type,
    is_hidden, lookup,
};
use rpsh_runtime::{PsString, Value};
use rpsh_syntax::{ArgumentSeparator, CommandArgument};

#[derive(Clone)]
pub(super) enum CallArgument {
    Positional(Value),
    Parameter {
        name: String,
        text: String,
        separator: ArgumentSeparator,
        value: Option<Value>,
        from_hashtable: bool,
    },
}

/// A pipeline evaluates each command name and its arguments, in source order,
/// before it runs any of the stages. Native binding also needs the original
/// quoting, comma spacing and parameter separators.
pub(super) struct PreparedArguments<'a> {
    pub source: &'a [CommandArgument],
    pub expanded: Vec<(Option<&'a CommandArgument>, CallArgument)>,
}

impl PreparedArguments<'_> {
    pub(super) fn for_binding(&self) -> impl Iterator<Item = CallArgument> + '_ {
        self.expanded.iter().filter_map(|(source, value)| {
            let end =
                matches!(source, Some(CommandArgument::Parameter { name, .. }) if name == "-");
            (!end).then(|| value.clone())
        })
    }
}

impl CallArgument {
    fn splatted_item(value: Value) -> Self {
        if let Value::String(text) = &value
            && let Some(name) = text.command_parameter_name()
        {
            return Self::Parameter {
                name: name.to_owned(),
                text: text.to_string(),
                separator: ArgumentSeparator::Space,
                value: None,
                from_hashtable: false,
            };
        }
        Self::Positional(value)
    }
}

fn dictionary_parameter_text(name: &str) -> String {
    let trimmed = name.trim_end();
    if trimmed.is_empty() || trimmed.ends_with(':') {
        format!("-{name}")
    } else {
        format!("-{trimmed}:{}", &name[trimmed.len()..])
    }
}

impl Engine {
    pub(super) fn splat(&self, name: &str) -> Vec<CallArgument> {
        let value = lookup(name, &self.variables, !self.last_failed).unwrap_or(Value::Null);
        if let Value::Object(entries) = &value
            && hidden_type(&value)
                .is_some_and(|ty| ty.contains("Hashtable") || ty.contains("Dictionary"))
        {
            return entries
                .iter()
                .filter(|(name, _)| !is_hidden(name))
                .map(|(name, value)| CallArgument::Parameter {
                    text: dictionary_parameter_text(name),
                    name: name.clone(),
                    separator: ArgumentSeparator::Colon,
                    value: Some(value.clone()),
                    from_hashtable: true,
                })
                .collect();
        }
        if let Value::String(text) = &value {
            return text
                .units()
                .map(|unit| CallArgument::Positional(Value::Char(unit)))
                .collect();
        }
        if let Some(items) = value.sequence() {
            return items
                .iter()
                .cloned()
                .map(CallArgument::splatted_item)
                .collect();
        }
        vec![CallArgument::splatted_item(value)]
    }

    pub(super) fn prepare_arguments<'a>(
        &mut self,
        args: &'a [CommandArgument],
    ) -> Result<PreparedArguments<'a>, EngineError> {
        let mut result = Vec::with_capacity(args.len());
        for arg in args {
            let value = match arg {
                CommandArgument::Splat(name) => {
                    result.extend(self.splat(name).into_iter().map(|value| (None, value)));
                    continue;
                }
                CommandArgument::Value(expr) => {
                    CallArgument::Positional(self.eval_expression(expr)?)
                }
                CommandArgument::Bareword(word) => {
                    CallArgument::Positional(Value::String(word.as_str().into()))
                }
                CommandArgument::Parameter {
                    name,
                    value,
                    separator,
                } => {
                    let colon = if *separator == ArgumentSeparator::Space {
                        ""
                    } else {
                        ":"
                    };
                    CallArgument::Parameter {
                        name: name.clone(),
                        text: format!("-{name}{colon}"),
                        separator: *separator,
                        value: value
                            .as_ref()
                            .map(|expr| self.eval_expression(expr))
                            .transpose()?,
                        from_hashtable: false,
                    }
                }
            };
            result.push((Some(arg), value));
        }
        Ok(PreparedArguments {
            source: args,
            expanded: result,
        })
    }
}

impl Engine {
    /// Keep unrecognized named arguments and their values in $args. Only
    /// standalone positional arguments can fill unbound declared parameters.
    pub(super) fn function_arguments(
        &mut self,
        arguments: &PreparedArguments<'_>,
        function: &FunctionDef,
    ) -> Result<BoundArguments, EngineError> {
        let mut args: Vec<_> = arguments.for_binding().collect();
        args.sort_by_key(|a| {
            matches!(
                a,
                CallArgument::Parameter {
                    from_hashtable: true,
                    ..
                }
            )
        });
        let mut args = args.into_iter().peekable();
        let mut bound = vec![None; function.parameters.len()];
        let mut order = Vec::new();
        let mut rest = Vec::new();
        while let Some(arg) = args.next() {
            let (name, text, mut value, separator, from_hashtable) = match arg {
                CallArgument::Positional(value) => {
                    rest.push((true, value));
                    continue;
                }
                CallArgument::Parameter {
                    name,
                    text,
                    value,
                    separator,
                    from_hashtable,
                } => (name, text, value, separator, from_hashtable),
            };
            let index = function
                .parameters
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(&name));
            if let Some(index) = index {
                if let Some((_, explicit)) = &bound[index] {
                    if from_hashtable && *explicit {
                        continue;
                    }
                    let message = rpsh_messages::format(
                        "ParameterBinderStrings/ParameterAlreadyBound",
                        &["", &name],
                    );
                    let record = error_record(
                        &format!("ParameterAlreadyBound,{}", function.name),
                        &format!(
                            "InvalidArgument: (:) [{}], ParameterBindingException",
                            function.name
                        ),
                        "System.Management.Automation.ParameterBindingException",
                        &message,
                        &message,
                    );
                    return Err(EngineError::Script(ScriptException::new(record)));
                }
                if value.is_none()
                    && matches!(args.peek(), Some(CallArgument::Positional(_)))
                    && let Some(CallArgument::Positional(next)) = args.next()
                {
                    value = Some(next);
                }
                bound[index] = Some((value.unwrap_or(Value::Bool(true)), !from_hashtable));
                order.push(index);
                continue;
            }
            rest.push((
                false,
                Value::String(PsString::command_parameter(&text, &name)),
            ));
            let take_next = value.is_none() || separator != ArgumentSeparator::Space;
            if let Some(value) = value {
                rest.push((false, value));
            }
            if take_next
                && matches!(args.peek(), Some(CallArgument::Positional(_)))
                && let Some(CallArgument::Positional(next)) = args.next()
            {
                rest.push((false, next));
            }
        }
        let mut unbound = Vec::new();
        let mut slots = bound
            .iter_mut()
            .enumerate()
            .filter(|(_, slot)| slot.is_none());
        for (positional, value) in rest {
            if positional && let Some((index, slot)) = slots.next() {
                *slot = Some((value, true));
                order.push(index);
            } else {
                unbound.push(value);
            }
        }
        let named = order
            .into_iter()
            .map(|index| {
                (
                    function.parameters[index].name.clone(),
                    bound[index].take().expect("bound parameter").0,
                )
            })
            .collect();
        Ok((unbound, named))
    }
}
