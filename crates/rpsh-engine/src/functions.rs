use super::{ArgumentBinding, FunctionDef};
use rpsh_runtime::Value;
use std::cell::LazyCell;
use std::collections::HashMap;

#[derive(Debug, Default)]
struct FunctionScope {
    entries: HashMap<String, FunctionDef>,
    script: bool,
}

#[derive(Debug)]
pub(super) struct Functions {
    global: LazyCell<HashMap<String, FunctionDef>>,
    scopes: Vec<FunctionScope>,
}

impl Default for Functions {
    fn default() -> Self {
        Self {
            global: LazyCell::new(|| {
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
            }),
            scopes: Vec::new(),
        }
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
            &self.global
        } else {
            &self.scopes[depth - 1].entries
        }
    }

    fn table_mut(&mut self, depth: usize) -> &mut HashMap<String, FunctionDef> {
        if depth == 0 {
            LazyCell::force_mut(&mut self.global)
        } else {
            &mut self.scopes[depth - 1].entries
        }
    }

    fn resolve(&self, name: &str) -> Option<(usize, &FunctionDef)> {
        let (depth, _, key) = self.scope(name);
        let current = self.scopes.len();
        let visible = |d| {
            self.table(d)
                .get(key)
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

    pub(super) fn insert(&mut self, mut function: FunctionDef) {
        let (depth, private, name) = self.scope(&function.name);
        let name = name.to_owned();
        let key = name.to_ascii_lowercase();
        function.name = name;
        function.private = private;
        let depth = depth.unwrap_or(self.scopes.len());
        let table = self.table_mut(depth);
        if let Some(old) = table.get_mut(&key)
            && old.filter == function.filter
            && let Some(source) = old.live_source.take()
        {
            source.items_mut()[0] = Value::ScriptBlock(function.source.as_ref().into());
            function.live_source = source.into();
        }
        table.insert(key, function);
    }

    pub(super) fn remove(&mut self, name: &str) {
        if let Some((depth, _)) = self.resolve(name) {
            let key = self.scope(name).2.to_owned();
            self.table_mut(depth).remove(&key);
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
            .find_map(|depth| self.table(depth).get(name))
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

    pub(super) fn info(&self) -> Value {
        let script = Value::ScriptBlock(self.source.as_ref().into());
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
    match lower {
        "verbose" | "vb" | "debug" | "db" => Some(ArgumentBinding::Switch),
        "erroraction"
        | "ea"
        | "errorvariable"
        | "ev"
        | "warningaction"
        | "wa"
        | "warningvariable"
        | "wv"
        | "informationaction"
        | "infa"
        | "informationvariable"
        | "iv"
        | "progressaction"
        | "proga"
        | "outvariable"
        | "ov"
        | "outbuffer"
        | "ob"
        | "pipelinevariable"
        | "pv" => Some(ArgumentBinding::Named),
        _ => None,
    }
}

pub(super) fn session_info(name: &str, kind: &str) -> Option<Value> {
    if !(cfg!(unix) && kind == "Cmdlet" && name == "Switch-Process") {
        return None;
    }
    let mut props = command_properties(name, kind, CMDLET);
    props.extend([
        (
            "Source".into(),
            Value::String("Microsoft.PowerShell.Core".into()),
        ),
        (
            "ModuleName".into(),
            Value::String("Microsoft.PowerShell.Core".into()),
        ),
        ("Version".into(), super::version_value("7.7.0.500".into())),
        (
            "Definition".into(),
            Value::String(
                "\nSwitch-Process [[-WithCommand] <string[]>] [<CommonParameters>]\n".into(),
            ),
        ),
        ("Visibility".into(), Value::String("Public".into())),
        ("Options".into(), Value::String("ReadOnly".into())),
        (
            "RemotingCapability".into(),
            Value::String("PowerShell".into()),
        ),
    ]);
    Some(Value::object(props))
}
