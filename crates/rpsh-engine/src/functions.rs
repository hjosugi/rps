use super::startup::Stage;
use super::{ArgumentBinding, FunctionDef};
use rpsh_runtime::Value;
use std::cell::OnceCell;
use std::collections::HashMap;

#[derive(Debug, Default)]
struct FunctionScope {
    entries: HashMap<String, FunctionDef>,
    script: bool,
}

#[derive(Debug, Default)]
pub(super) struct Functions {
    global: OnceCell<HashMap<String, FunctionDef>>,
    scopes: Vec<FunctionScope>,
}

fn initial_functions() -> HashMap<String, FunctionDef> {
    Stage::Functions.enter();
    if cfg!(unix) {
        let function = FunctionDef {
            name: "exec".into(),
            source: "\nSwitch-Process -WithCommand $args\n".into(),
            parameters: Vec::new(),
            block: Default::default(),
            filter: false,
            private: false,
            live_source: Default::default(),
        };
        HashMap::from([("exec".into(), function)])
    } else {
        HashMap::new()
    }
}

impl Functions {
    fn scope<'a>(&self, name: &'a str) -> (Option<usize>, bool, &'a str) {
        let Some((prefix, rest)) = name.split_once(':') else {
            return (None, false, name);
        };
        if prefix.eq_ignore_ascii_case("global") {
            (Some(0), false, rest)
        } else if prefix.eq_ignore_ascii_case("script") {
            let depth = self
                .scopes
                .iter()
                .rposition(|scope| scope.script)
                .map_or(0, |i| i + 1);
            (Some(depth), false, rest)
        } else if prefix.eq_ignore_ascii_case("local") || prefix.eq_ignore_ascii_case("private") {
            (
                Some(self.scopes.len()),
                prefix.eq_ignore_ascii_case("private"),
                rest,
            )
        } else {
            (None, false, name)
        }
    }

    fn table(&self, depth: usize) -> &HashMap<String, FunctionDef> {
        if depth == 0 {
            self.global.get_or_init(initial_functions)
        } else {
            &self.scopes[depth - 1].entries
        }
    }

    fn table_mut(&mut self, depth: usize) -> &mut HashMap<String, FunctionDef> {
        if depth == 0 {
            self.global.get_or_init(initial_functions);
            self.global.get_mut().expect("initialized global scope")
        } else {
            &mut self.scopes[depth - 1].entries
        }
    }

    fn entry(&self, depth: usize, key: &str) -> Option<&FunctionDef> {
        // Before the first definition the only possible global entry on Unix
        // is exec. A miss during ordinary command discovery needs no table.
        if depth == 0 && self.global.get().is_none() && !(cfg!(unix) && key == "exec") {
            return None;
        }
        self.table(depth).get(key)
    }

    fn resolve(&self, name: &str) -> Option<(usize, &FunctionDef)> {
        let (depth, _, key) = self.scope(name);
        let current = self.scopes.len();
        let visible = |d| {
            self.entry(d, key)
                .filter(|f| !f.private || d == current)
                .map(|f| (d, f))
        };
        match depth {
            Some(d) => visible(d),
            None => (0..=current).rev().find_map(visible),
        }
    }

    pub(super) fn get(&self, name: &str) -> Option<&FunctionDef> {
        self.resolve(name).map(|(_, f)| f)
    }

    pub(super) fn contains_key(&self, name: &str) -> bool {
        self.resolve(name).is_some()
    }

    pub(super) fn enter_scope(&mut self, script: bool) {
        self.scopes.push(FunctionScope {
            script,
            entries: HashMap::new(),
        });
    }

    pub(super) fn leave_scope(&mut self) {
        self.scopes.pop().expect("active function scope");
    }

    pub(super) fn insert(&mut self, function: FunctionDef) {
        let (depth, private, name) = self.scope(&function.name);
        self.install(
            depth.unwrap_or(self.scopes.len()),
            private,
            name.to_owned(),
            function,
        );
    }

    pub(super) fn set_item(&mut self, function: FunctionDef, copy_options: bool) -> &FunctionDef {
        let (depth, private, name) = self.scope(&function.name);
        let name = name.to_owned();
        // The overload that copies a FunctionInfo also supplies its options
        // and writes in the requested scope, even when a parent has this name.
        if copy_options {
            return self.install(
                depth.unwrap_or(self.scopes.len()),
                private || function.private,
                name,
                function,
            );
        }
        let existing = self.resolve(&function.name.to_ascii_lowercase());
        let depth = depth
            .or_else(|| existing.map(|(depth, _)| depth))
            .unwrap_or(self.scopes.len());
        let private = private || function.private || existing.is_some_and(|(_, f)| f.private);
        self.install(depth, private, name, function)
    }

    fn install(
        &mut self,
        depth: usize,
        private: bool,
        name: String,
        mut function: FunctionDef,
    ) -> &FunctionDef {
        Stage::Functions.enter();
        let key = name.to_ascii_lowercase();
        function.name = name;
        function.private = private;
        let slot = self.table_mut(depth).entry(key);
        match slot {
            std::collections::hash_map::Entry::Vacant(slot) => slot.insert(function),
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                let old = slot.get_mut();
                if old.filter == function.filter {
                    function.name.clone_from(&old.name);
                    if let Some(source) = old.live_source.take() {
                        source.items_mut()[0] = function.script_block();
                        function.live_source = source.into();
                    }
                }
                slot.insert(function);
                slot.into_mut()
            }
        }
    }

    pub(super) fn remove(&mut self, name: &str) {
        if let Some((depth, _)) = self.resolve(name) {
            // SessionStateInternal.RemoveFunction resolves the scope but passes
            // the original provider name to the scope dictionary.
            self.table_mut(depth).remove(name);
        }
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &String> {
        let scopes = self.scopes.len();
        (0..=scopes).rev().flat_map(move |depth| {
            self.table(depth).keys().filter(move |key| {
                (depth + 1..=scopes).all(|inner| !self.table(inner).contains_key(*key))
            })
        })
    }

    pub(super) fn info(&self, name: &str) -> Option<&FunctionDef> {
        (0..=self.scopes.len())
            .rev()
            .find_map(|depth| self.entry(depth, name))
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &FunctionDef> {
        self.keys().filter_map(|key| self.info(key))
    }
}

pub(super) const FILTER: &str = "System.Management.Automation.FilterInfo";
pub(super) const FUNCTION: &str = "System.Management.Automation.FunctionInfo";
pub(super) const CMDLET: &str = "System.Management.Automation.CmdletInfo";

fn command_properties(name: &str, kind: &str, ty: &str) -> Vec<(String, Value)> {
    let mut bases = Vec::new();
    if ty == FILTER {
        bases.push(Value::String(FUNCTION.into()));
    }
    bases.push(Value::String(
        "System.Management.Automation.CommandInfo".into(),
    ));
    vec![
        ("Name".into(), Value::String(name.into())),
        ("CommandType".into(), Value::String(kind.into())),
        ("__Text".into(), Value::String(name.into())),
        ("__TypeName".into(), Value::String(ty.into())),
        ("__BaseClasses".into(), Value::array(bases)),
    ]
}

pub(super) fn command_info(name: &str, kind: &str) -> Value {
    Value::object(command_properties(
        name,
        kind,
        if kind == "Function" { FUNCTION } else { CMDLET },
    ))
}

impl FunctionDef {
    pub(super) fn from_source(source: &str) -> Result<Self, rpsh_syntax::ParseError> {
        let mut function = Self::from_script_block(rpsh_syntax::parse(source)?);
        function.source = source.into();
        Ok(function)
    }

    pub(super) fn from_info(info: &Value) -> Result<Option<Self>, super::EngineError> {
        let kind = super::hidden_type(info);
        let Some(kind @ (FUNCTION | FILTER)) = kind.as_deref() else {
            return Ok(None);
        };
        let Value::ScriptBlock(source) = super::member(info, "ScriptBlock")? else {
            return Ok(None);
        };
        let mut function = Self::from_source(&source.to_string_lossy())?;
        function.name = super::member(info, "Name")?.to_string();
        function.filter = kind == FILTER;
        function.private = super::member(info, "Options")?
            .to_string()
            .split(", ")
            .any(|v| v == "Private");
        Ok(Some(function))
    }

    pub(super) fn syntax(&self) -> Result<&rpsh_syntax::ScriptBlockAst, rpsh_syntax::ParseError> {
        if let Some(block) = self.block.get() {
            return Ok(block);
        }
        let block = rpsh_syntax::parse(&self.source)?;
        Ok(self.block.get_or_init(|| block))
    }

    pub(super) fn kind(&self) -> &'static str {
        if self.filter { "Filter" } else { "Function" }
    }

    pub(super) fn script_block(&self) -> Value {
        Value::ScriptBlock(self.source.as_ref().into())
    }

    pub(super) fn info(&self) -> Value {
        let script = self.script_block();
        let reference = self
            .live_source
            .get_or_init(|| rpsh_runtime::ArrayList::from_values(vec![script.clone()]));
        let ty = if self.filter { FILTER } else { FUNCTION };
        let mut props = command_properties(&self.name, self.kind(), ty);
        props.extend([
            (
                "__FunctionSource".into(),
                Value::ArrayList(reference.clone()),
            ),
            (
                "Definition".into(),
                Value::String(self.source.as_ref().into()),
            ),
            ("ScriptBlock".into(), script),
            ("Source".into(), Value::String("".into())),
            ("ModuleName".into(), Value::String("".into())),
            ("Version".into(), Value::Null),
            ("Visibility".into(), Value::String("Public".into())),
            (
                "Options".into(),
                Value::String(if self.private { "Private" } else { "None" }.into()),
            ),
            (
                "RemotingCapability".into(),
                Value::String("PowerShell".into()),
            ),
        ]);
        Value::object(props)
    }
}

pub(super) fn common_binding(lower: &str) -> Option<ArgumentBinding> {
    use rpsh_commands::signature::{COMMON_PARAMETERS, ParameterType};
    COMMON_PARAMETERS
        .iter()
        .find(|p| p.is_named(lower))
        .map(|p| {
            if p.ty == ParameterType::Switch {
                ArgumentBinding::Switch
            } else {
                ArgumentBinding::Named
            }
        })
}

pub(super) fn declared_info(name: &str, kind: &str) -> Option<Value> {
    if kind != "Cmdlet" {
        return None;
    }
    let signature = rpsh_commands::signature::lookup(name)?;
    let module = rpsh_commands::MODULES
        .iter()
        .find(|m| m.name == signature.module);
    let mut props = command_properties(name, kind, CMDLET);
    let (verb, noun) = name.split_once('-').unwrap_or(("", name));
    props.extend([
        (
            "ImplementingType".into(),
            Value::Type(signature.implementation.into()),
        ),
        ("Verb".into(), Value::String(verb.into())),
        ("Noun".into(), Value::String(noun.into())),
        ("HelpUri".into(), Value::String(signature.help_uri.into())),
        (
            "DefaultParameterSet".into(),
            signature
                .default_set
                .map_or(Value::Null, |s| Value::String(s.into())),
        ),
        (
            "__CommandParameterCache".into(),
            Value::ArrayList(rpsh_runtime::ArrayList::default()),
        ),
        ("Source".into(), Value::String(signature.module.into())),
        ("ModuleName".into(), Value::String(signature.module.into())),
        (
            "Version".into(),
            super::version_value(module.map_or("7.7.0.500", |m| m.version).into()),
        ),
        (
            "Definition".into(),
            Value::String(signature.syntax(name).into()),
        ),
        ("Visibility".into(), Value::String("Public".into())),
        ("Options".into(), Value::String("ReadOnly".into())),
        (
            "RemotingCapability".into(),
            Value::String(signature.remoting_capability.into()),
        ),
    ]);
    Some(Value::object(props))
}
