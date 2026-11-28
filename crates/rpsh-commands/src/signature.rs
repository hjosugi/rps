//! Declarations used by the command binder, syntax display and inventory audit.
//! The checked-in upstream inventory is a separate reference for the audit.
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterType {
    Switch,
    Int32,
    String,
    StringArray,
    ObjectArray,
    PsObject,
}

impl ParameterType {
    pub const fn full_name(self) -> &'static str {
        match self {
            Self::Switch => "System.Management.Automation.SwitchParameter",
            Self::Int32 => "System.Int32",
            Self::String => "System.String",
            Self::StringArray => "System.String[]",
            Self::ObjectArray => "System.Object[]",
            Self::PsObject => "System.Management.Automation.PSObject",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Switch => "switch",
            Self::Int32 => "int",
            Self::String => "string",
            Self::StringArray => "string[]",
            Self::ObjectArray => "Object[]",
            Self::PsObject => "psobject",
        }
    }
}

#[derive(Debug)]
pub struct Parameter {
    pub name: &'static str,
    pub ty: ParameterType,
    /// An empty slice means the declaration applies to every set.
    pub sets: &'static [&'static str],
    pub aliases: &'static [&'static str],
    position: Option<u16>,
    mandatory: bool,
}

impl Parameter {
    const fn new(name: &'static str, ty: ParameterType) -> Self {
        Self {
            name,
            ty,
            sets: &[],
            aliases: &[],
            position: None,
            mandatory: false,
        }
    }

    const fn in_sets(mut self, sets: &'static [&'static str]) -> Self {
        self.sets = sets;
        self
    }
    const fn positional(mut self, position: u16) -> Self {
        self.position = Some(position);
        self
    }
    const fn mandatory(mut self) -> Self {
        self.mandatory = true;
        self
    }

    fn names(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name).chain(self.aliases.iter().copied())
    }

    fn write_syntax(&self, out: &mut String) {
        if !self.mandatory {
            out.push('[');
        }
        if self.position.is_some() {
            out.push('[');
        }
        out.push('-');
        out.push_str(self.name);
        if self.position.is_some() {
            out.push(']');
        }
        if self.ty != ParameterType::Switch {
            write!(out, " <{}>", self.ty.display_name()).unwrap();
        }
        if !self.mandatory {
            out.push(']');
        }
    }
}

#[derive(Debug)]
pub struct Signature {
    pub name: &'static str,
    pub module: &'static str,
    pub remoting_capability: &'static str,
    pub parameters: &'static [Parameter],
    sets: &'static [&'static str],
    unix_only: bool,
}

impl Signature {
    pub fn available_on(&self, platform: &str) -> bool {
        !self.unix_only || platform.starts_with("linux-") || platform.starts_with("macos-")
    }

    pub fn parameter(&self, name: &str) -> Option<&'static Parameter> {
        self.parameters
            .iter()
            .find(|p| p.names().any(|n| n.eq_ignore_ascii_case(name)))
    }

    pub fn matching<'a>(&self, name: &'a str) -> impl Iterator<Item = &'static Parameter> + 'a {
        self.parameters.iter().filter(move |p| {
            !name.is_empty()
                && p.names().any(|n| {
                    n.get(..name.len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
                })
        })
    }

    /// Common parameters are valid in all sets. Known command parameters must
    /// share at least one set, including when a switch is explicitly false.
    pub fn accepts_named<'a, I>(&self, names: I) -> bool
    where
        I: Iterator<Item = &'a str> + Clone,
    {
        self.sets.iter().any(|set| {
            names.clone().all(|name| {
                self.parameter(name)
                    .is_none_or(|p| p.sets.is_empty() || p.sets.contains(set))
            })
        })
    }

    /// PowerShell renders positioned parameters first, followed by mandatory
    /// named parameters and then the other named parameters in declaration order.
    pub fn syntax(&self, invoked_as: &str) -> String {
        let mut output = if self.name.eq_ignore_ascii_case(invoked_as) {
            String::new()
        } else {
            format!("{invoked_as} (alias) -> {}\n", self.name)
        };
        for set in self.sets {
            output.push('\n');
            output.push_str(invoked_as);
            let mut parameters: Vec<_> = self
                .parameters
                .iter()
                .filter(|p| p.sets.is_empty() || p.sets.contains(set))
                .collect();
            parameters.sort_by_key(|p| (p.position.unwrap_or(u16::MAX), !p.mandatory));
            for parameter in parameters {
                output.push(' ');
                parameter.write_syntax(&mut output);
            }
            output.push_str(" [<CommonParameters>]\n");
        }
        output
    }
}

use ParameterType::{Int32, ObjectArray, PsObject, String as Text, StringArray, Switch};

// Declaration order is also the order returned for ambiguous abbreviations.
pub static SORT_OBJECT: Signature = Signature {
    name: "Sort-Object",
    module: "Microsoft.PowerShell.Utility",
    remoting_capability: "None",
    sets: &["Default", "Top", "Bottom"],
    unix_only: false,
    parameters: &[
        Parameter::new("Stable", Switch).in_sets(&["Default"]),
        Parameter::new("Descending", Switch),
        Parameter::new("Unique", Switch),
        Parameter::new("Top", Int32).in_sets(&["Top"]).mandatory(),
        Parameter::new("Bottom", Int32)
            .in_sets(&["Bottom"])
            .mandatory(),
        Parameter::new("Property", ObjectArray).positional(0),
        Parameter::new("InputObject", PsObject),
        Parameter::new("Culture", Text),
        Parameter::new("CaseSensitive", Switch),
    ],
};

pub static SWITCH_PROCESS: Signature = Signature {
    name: "Switch-Process",
    module: "Microsoft.PowerShell.Core",
    remoting_capability: "PowerShell",
    sets: &["__AllParameterSets"],
    unix_only: true,
    parameters: &[Parameter::new("WithCommand", StringArray).positional(0)],
};

pub static SIGNATURES: &[&Signature] = &[&SORT_OBJECT, &SWITCH_PROCESS];

pub fn lookup(name: &str) -> Option<&'static Signature> {
    SIGNATURES
        .iter()
        .copied()
        .find(|s| (!s.unix_only || cfg!(unix)) && s.name.eq_ignore_ascii_case(name))
}
