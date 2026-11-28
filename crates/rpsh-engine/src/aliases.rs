//! Session aliases from PowerShell's InitialSessionState. Only changes to the
//! static defaults require storage in an engine instance.

use std::collections::HashMap;

use rpsh_runtime::{RuntimeError, Value, ops};

use super::{flatten_value, flatten_values};

pub(super) const TYPE_NAME: &str = "System.Management.Automation.AliasInfo";

#[derive(Debug, Clone, Copy)]
pub(super) struct Alias<'a> {
    pub name: &'a str,
    pub definition: &'a str,
    pub options: Options,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Options {
    None,
    ReadOnly,
    AllScope,
    ReadOnlyAllScope,
}

impl Options {
    pub fn read_only(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadOnlyAllScope)
    }

    pub fn all_scope(self) -> bool {
        matches!(self, Self::AllScope | Self::ReadOnlyAllScope)
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::ReadOnly => "ReadOnly",
            Self::AllScope => "AllScope",
            Self::ReadOnlyAllScope => "ReadOnly, AllScope",
        }
    }
}

impl Alias<'_> {
    pub fn value(self) -> Value {
        Value::object(vec![
            ("Name".into(), Value::String(self.name.into())),
            ("__TypeName".into(), Value::String(TYPE_NAME.into())),
            ("__Text".into(), Value::String(self.name.into())),
            ("CommandType".into(), Value::String("Alias".into())),
            (
                "DisplayName".into(),
                Value::String(format!("{} -> {}", self.name, self.definition).into()),
            ),
            ("Source".into(), Value::String("".into())),
            ("Version".into(), Value::Null),
            ("Definition".into(), Value::String(self.definition.into())),
            ("Options".into(), Value::String(self.options.label().into())),
            ("ModuleName".into(), Value::String("".into())),
        ])
    }
}

#[derive(Debug, Default)]
pub(super) struct Aliases {
    // Keys are case-insensitive. A removed default must stay hidden.
    changes: HashMap<String, Option<(String, String)>>,
}

impl Aliases {
    pub fn get(&self, name: &str) -> Option<Alias<'_>> {
        let key = name.to_ascii_lowercase();
        if let Some(change) = self.changes.get(&key) {
            return change.as_ref().map(custom_alias);
        }
        let index = DEFAULTS
            .binary_search_by_key(&key.as_str(), |entry| entry.0)
            .ok()?;
        Some(default_alias(DEFAULTS[index]))
    }

    pub fn definition(&self, name: &str) -> Option<String> {
        self.get(name).map(|alias| alias.definition.to_owned())
    }

    pub fn iter(&self) -> impl Iterator<Item = Alias<'_>> {
        let defaults = DEFAULTS
            .iter()
            .filter(|(name, _, _)| !self.changes.contains_key(*name))
            .copied()
            .map(default_alias);
        defaults.chain(self.changes.values().flatten().map(custom_alias))
    }

    /// Get-Alias writes a sorted group for each input pattern, including
    /// repeated matches. Exclude applies to the field being searched.
    pub fn query(
        &self,
        positional: Vec<Value>,
        named: &[(String, Value)],
    ) -> Result<Vec<Value>, RuntimeError> {
        let parameter = |name: &str| {
            named
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value)
        };
        let strings =
            |values: Vec<Value>| values.iter().map(ToString::to_string).collect::<Vec<_>>();
        let name = parameter("Name");
        let definition = parameter("Definition");
        if definition.is_some() && (name.is_some() || !positional.is_empty()) {
            return Err(RuntimeError::NotImplemented {
                feature: "Get-Alias with both Name and Definition",
            });
        }
        if parameter("Scope").is_some() {
            return Err(RuntimeError::NotImplemented {
                feature: "Get-Alias with an explicit scope",
            });
        }
        let patterns = match definition.or(name) {
            Some(value) => strings(flatten_value(value.clone())),
            None if positional.is_empty() => vec!["*".to_owned()],
            None => strings(flatten_values(positional)),
        };
        let excluded = parameter("Exclude")
            .map(|value| strings(flatten_value(value.clone())))
            .unwrap_or_default();
        let mut candidates = Vec::new();
        for alias in self.iter() {
            let text = if definition.is_some() {
                alias.definition
            } else {
                alias.name
            };
            if !excluded
                .iter()
                .any(|pattern| ops::wildcard_match(text, pattern, false))
            {
                candidates.push((alias, text));
            }
        }
        candidates.sort_by_cached_key(|(alias, _)| name_sort_key(alias));
        let mut outputs = Vec::new();
        for pattern in patterns {
            for (alias, text) in &candidates {
                if pattern == "*" || ops::wildcard_match(text, &pattern, false) {
                    outputs.push(alias.value());
                }
            }
        }
        Ok(outputs)
    }

    pub fn set(&mut self, name: String, definition: String) -> Alias<'_> {
        let change = self.changes.entry(name.to_ascii_lowercase()).or_default();
        custom_alias(change.insert((name, definition)))
    }

    pub fn remove(&mut self, name: &str) {
        let key = name.to_ascii_lowercase();
        if DEFAULTS
            .binary_search_by_key(&key.as_str(), |entry| entry.0)
            .is_ok()
        {
            self.changes.insert(key, None);
        } else {
            self.changes.remove(&key);
        }
    }
}

fn custom_alias((name, definition): &(String, String)) -> Alias<'_> {
    Alias {
        name,
        definition,
        options: Options::None,
    }
}

fn default_alias<'a>((name, definition, options): (&'a str, &'a str, Options)) -> Alias<'a> {
    Alias {
        name,
        definition,
        options,
    }
}

// PowerShell's reference culture sorts its two punctuation aliases before
// alphabetic names, with the question mark first.
pub(super) fn name_sort_key(alias: &Alias<'_>) -> (bool, String) {
    (alias.name != "?", alias.name.to_ascii_lowercase())
}

// Sorted by ASCII name for binary search.
const DEFAULTS: &[(&str, &str, Options)] = &[
    ("%", "ForEach-Object", Options::ReadOnlyAllScope),
    ("?", "Where-Object", Options::ReadOnlyAllScope),
    #[cfg(windows)]
    ("ac", "Add-Content", Options::ReadOnly),
    ("cd", "Set-Location", Options::AllScope),
    ("chdir", "Set-Location", Options::None),
    ("clc", "Clear-Content", Options::ReadOnly),
    ("clhy", "Clear-History", Options::ReadOnly),
    ("cli", "Clear-Item", Options::ReadOnly),
    ("clp", "Clear-ItemProperty", Options::ReadOnly),
    ("cls", "Clear-Host", Options::None),
    ("clv", "Clear-Variable", Options::ReadOnly),
    ("copy", "Copy-Item", Options::AllScope),
    #[cfg(windows)]
    ("cp", "Copy-Item", Options::AllScope),
    ("cpi", "Copy-Item", Options::ReadOnly),
    ("cvpa", "Convert-Path", Options::ReadOnly),
    ("dbp", "Disable-PSBreakpoint", Options::ReadOnly),
    ("del", "Remove-Item", Options::AllScope),
    #[cfg(windows)]
    ("diff", "Compare-Object", Options::ReadOnly),
    ("dir", "Get-ChildItem", Options::AllScope),
    ("ebp", "Enable-PSBreakpoint", Options::ReadOnly),
    ("echo", "Write-Output", Options::AllScope),
    ("epal", "Export-Alias", Options::ReadOnly),
    ("epcsv", "Export-Csv", Options::ReadOnly),
    ("erase", "Remove-Item", Options::None),
    ("etsn", "Enter-PSSession", Options::None),
    ("exsn", "Exit-PSSession", Options::None),
    ("fc", "Format-Custom", Options::ReadOnly),
    ("fl", "Format-List", Options::ReadOnly),
    ("foreach", "ForEach-Object", Options::ReadOnlyAllScope),
    ("ft", "Format-Table", Options::ReadOnly),
    ("fw", "Format-Wide", Options::ReadOnly),
    ("gal", "Get-Alias", Options::ReadOnly),
    ("gbp", "Get-PSBreakpoint", Options::ReadOnly),
    ("gc", "Get-Content", Options::ReadOnly),
    ("gci", "Get-ChildItem", Options::ReadOnly),
    ("gcm", "Get-Command", Options::ReadOnly),
    ("gcs", "Get-PSCallStack", Options::ReadOnly),
    ("gdr", "Get-PSDrive", Options::ReadOnly),
    ("gerr", "Get-Error", Options::ReadOnly),
    ("ghy", "Get-History", Options::ReadOnly),
    ("gi", "Get-Item", Options::ReadOnly),
    ("gjb", "Get-Job", Options::None),
    ("gl", "Get-Location", Options::ReadOnly),
    ("gm", "Get-Member", Options::ReadOnly),
    ("gmo", "Get-Module", Options::ReadOnly),
    ("gp", "Get-ItemProperty", Options::ReadOnly),
    ("gps", "Get-Process", Options::ReadOnly),
    ("gpv", "Get-ItemPropertyValue", Options::ReadOnly),
    ("group", "Group-Object", Options::ReadOnly),
    ("gsn", "Get-PSSession", Options::None),
    ("gu", "Get-Unique", Options::ReadOnly),
    ("gv", "Get-Variable", Options::ReadOnly),
    ("h", "Get-History", Options::None),
    ("history", "Get-History", Options::None),
    ("icm", "Invoke-Command", Options::None),
    ("iex", "Invoke-Expression", Options::ReadOnly),
    ("ihy", "Invoke-History", Options::ReadOnly),
    ("ii", "Invoke-Item", Options::ReadOnly),
    ("ipal", "Import-Alias", Options::ReadOnly),
    ("ipcsv", "Import-Csv", Options::ReadOnly),
    ("ipmo", "Import-Module", Options::ReadOnly),
    ("irm", "Invoke-RestMethod", Options::ReadOnly),
    ("iwr", "Invoke-WebRequest", Options::ReadOnly),
    #[cfg(windows)]
    ("ls", "Get-ChildItem", Options::None),
    ("md", "mkdir", Options::AllScope),
    ("measure", "Measure-Object", Options::ReadOnly),
    ("mi", "Move-Item", Options::ReadOnly),
    ("move", "Move-Item", Options::AllScope),
    ("mp", "Move-ItemProperty", Options::ReadOnly),
    #[cfg(windows)]
    ("mv", "Move-Item", Options::None),
    ("nal", "New-Alias", Options::ReadOnly),
    ("ndr", "New-PSDrive", Options::ReadOnly),
    ("ni", "New-Item", Options::ReadOnly),
    ("nmo", "New-Module", Options::ReadOnly),
    ("nsn", "New-PSSession", Options::None),
    ("nv", "New-Variable", Options::ReadOnly),
    ("oh", "Out-Host", Options::ReadOnly),
    ("popd", "Pop-Location", Options::AllScope),
    ("pushd", "Push-Location", Options::AllScope),
    ("pwd", "Get-Location", Options::None),
    ("r", "Invoke-History", Options::None),
    ("rbp", "Remove-PSBreakpoint", Options::ReadOnly),
    ("rcjb", "Receive-Job", Options::None),
    ("rcsn", "Receive-PSSession", Options::ReadOnly),
    ("rd", "Remove-Item", Options::None),
    ("rdr", "Remove-PSDrive", Options::ReadOnly),
    ("ren", "Rename-Item", Options::None),
    ("ri", "Remove-Item", Options::ReadOnly),
    ("rjb", "Remove-Job", Options::None),
    #[cfg(windows)]
    ("rm", "Remove-Item", Options::None),
    ("rmo", "Remove-Module", Options::ReadOnly),
    ("rni", "Rename-Item", Options::ReadOnly),
    ("rnp", "Rename-ItemProperty", Options::ReadOnly),
    ("rp", "Remove-ItemProperty", Options::ReadOnly),
    ("rsn", "Remove-PSSession", Options::None),
    ("rv", "Remove-Variable", Options::ReadOnly),
    ("rvpa", "Resolve-Path", Options::ReadOnly),
    ("sajb", "Start-Job", Options::None),
    ("sal", "Set-Alias", Options::ReadOnly),
    ("saps", "Start-Process", Options::ReadOnly),
    ("sbp", "Set-PSBreakpoint", Options::ReadOnly),
    ("select", "Select-Object", Options::ReadOnlyAllScope),
    ("set", "Set-Variable", Options::None),
    ("si", "Set-Item", Options::ReadOnly),
    ("sl", "Set-Location", Options::ReadOnly),
    ("sls", "Select-String", Options::None),
    ("sp", "Set-ItemProperty", Options::ReadOnly),
    ("spjb", "Stop-Job", Options::None),
    ("spps", "Stop-Process", Options::ReadOnly),
    ("sv", "Set-Variable", Options::ReadOnly),
    ("type", "Get-Content", Options::None),
    ("where", "Where-Object", Options::ReadOnlyAllScope),
    ("wjb", "Wait-Job", Options::None),
];
