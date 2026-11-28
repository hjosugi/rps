//! Dynamically scoped values and the lazily created, session-wide error list.
use super::{ps_variable, startup::Stage};
use rpsh_runtime::{ArrayList, Value};
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

/// A cmdlet can retain a reference to a variable and observe subsequent writes.
/// An injected PSVariable occupies the session dictionary; a compiled assignment
/// in the block can introduce a separate local with the same name.
#[derive(Clone, Debug)]
enum Binding {
    Plain(Value),
    Observed(Value),
    Injected(Box<InjectedVariable>),
}

#[derive(Clone, Debug)]
struct InjectedVariable {
    object: Value,
    local: Option<Value>,
}

impl Binding {
    fn value(&self) -> Value {
        match self {
            Self::Plain(v) => v.clone(),
            Self::Observed(obj) => ps_variable::value(obj).unwrap_or(Value::Null),
            Self::Injected(slot) => slot
                .local
                .clone()
                .unwrap_or_else(|| ps_variable::value(&slot.object).unwrap_or(Value::Null)),
        }
    }

    fn assign(&mut self, value: Value) -> Value {
        if let Self::Observed(object) = self {
            return ps_variable::replace(object, value).unwrap_or(Value::Null);
        }
        if let Self::Injected(slot) = self {
            return slot
                .local
                .replace(value)
                .unwrap_or_else(|| ps_variable::value(&slot.object).unwrap_or(Value::Null));
        }
        std::mem::replace(self, Self::Plain(value)).value()
    }
}

/// Invocation metadata is available immediately, but a cold script that
/// never consults session variables needs neither PS values nor a hash table.
#[derive(Clone, Debug, Default)]
struct Scope {
    table: OnceCell<HashMap<String, Binding>>,
    pending: RefCell<Option<Invocation>>,
}

#[derive(Clone, Debug)]
struct Invocation {
    args: Vec<String>,
    path: Option<PathBuf>,
}

fn bind_invocation(table: &mut HashMap<String, Binding>, args: &[String], path: Option<&Path>) {
    table.insert(
        "args".into(),
        Binding::Plain(Value::array(
            args.iter()
                .map(|arg| Value::String(arg.as_str().into()))
                .collect(),
        )),
    );
    if let Some(path) = path {
        bind_script_path(table, path);
    }
}

fn bind_script_path(table: &mut HashMap<String, Binding>, path: &Path) {
    table.insert(
        "PSCommandPath".into(),
        Binding::Plain(Value::String(path.to_string_lossy().as_ref().into())),
    );
    if let Some(parent) = path.parent() {
        table.insert(
            "PSScriptRoot".into(),
            Binding::Plain(Value::String(parent.to_string_lossy().as_ref().into())),
        );
    }
}

impl Scope {
    fn stored_key<'a>(&'a self, key: &str) -> Option<&'a String> {
        self.get_key_value(key).map(|(k, _)| k).or_else(|| {
            (!key.starts_with("env:"))
                .then(|| self.keys().find(|k| k.eq_ignore_ascii_case(key)))
                .flatten()
        })
    }

    fn binding(&self, key: &str) -> Option<&Binding> {
        self.get(key).or_else(|| {
            if key.starts_with("env:") {
                return None;
            }
            self.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v)
        })
    }

    fn put(&mut self, key: String, value: Value) -> Option<Value> {
        if let Some(slot) = self.get_mut(&key) {
            return Some(slot.assign(value));
        }
        if let Some(key) = self.stored_key(&key).cloned() {
            return self.get_mut(&key).map(|slot| slot.assign(value));
        }
        self.insert(key, Binding::Plain(value));
        None
    }

    fn discard(&mut self, key: &str) -> Option<Value> {
        let key = self.stored_key(key)?.clone();
        self.remove(&key).map(|s| s.value())
    }

    fn set_invocation(&mut self, args: &[String], path: Option<PathBuf>) {
        if let Some(table) = self.table.get_mut() {
            bind_invocation(table, args, path.as_deref());
        } else {
            let pending = self.pending.get_mut();
            let path = path.or_else(|| pending.take().and_then(|old| old.path));
            *pending = Some(Invocation {
                args: args.to_vec(),
                path,
            });
        }
    }

    fn initialize(pending: &RefCell<Option<Invocation>>) -> HashMap<String, Binding> {
        Stage::Variables.enter();
        let mut table = HashMap::new();
        if let Some(invocation) = pending.take() {
            bind_invocation(&mut table, &invocation.args, invocation.path.as_deref());
        }
        table
    }
}

impl Deref for Scope {
    type Target = HashMap<String, Binding>;
    fn deref(&self) -> &Self::Target {
        self.table.get_or_init(|| Self::initialize(&self.pending))
    }
}

impl DerefMut for Scope {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.table.get_or_init(|| Self::initialize(&self.pending));
        self.table.get_mut().expect("initialized scope")
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Variables {
    entries: Scope,
    errors: OnceCell<ArrayList>,
    parents: Vec<Scope>,
    script_scope: usize,
    pub(super) nested: bool,
}

impl Variables {
    pub(super) fn child(&self) -> Self {
        let mut child = self.clone();
        child.parents.push(std::mem::take(&mut child.entries));
        child.nested = true;
        child
    }

    pub(super) fn start_script_scope(&mut self) {
        self.script_scope = self.parents.len();
    }

    fn scope(&self, name: &str) -> Option<usize> {
        let (prefix, _) = name.split_once(':')?;
        if prefix.eq_ignore_ascii_case("global") || prefix.eq_ignore_ascii_case("env") {
            Some(0)
        } else if prefix.eq_ignore_ascii_case("script") {
            Some(self.script_scope)
        } else if prefix.eq_ignore_ascii_case("local") || prefix.eq_ignore_ascii_case("private") {
            Some(self.parents.len())
        } else {
            None
        }
    }

    fn visible_scope(&self, key: &str) -> Option<usize> {
        if self.entries.binding(key).is_some() {
            Some(self.parents.len())
        } else {
            self.parents.iter().rposition(|p| p.binding(key).is_some())
        }
    }

    fn frame(&self, i: usize) -> &Scope {
        self.parents.get(i).unwrap_or(&self.entries)
    }
    fn frame_mut(&mut self, i: usize) -> &mut Scope {
        self.parents.get_mut(i).unwrap_or(&mut self.entries)
    }

    pub(super) fn get(&self, key: &str) -> Option<Value> {
        self.entries
            .binding(key)
            .or_else(|| self.parents.iter().rev().find_map(|p| p.binding(key)))
            .map(Binding::value)
    }

    pub(super) fn contains_key(&self, key: &str) -> bool {
        self.visible_scope(key).is_some()
    }

    pub(super) fn get_scoped(&self, name: &str, key: &str) -> Option<Value> {
        match self.scope(name) {
            Some(i) => self.frame(i).binding(key).map(Binding::value),
            None => self.get(key),
        }
    }

    pub(super) fn expose(&mut self, name: &str, key: &str) -> Option<Value> {
        let i = self.scope(name).or_else(|| self.visible_scope(key))?;
        let local = i > 0;
        let scope = self.frame_mut(i);
        let key = scope.stored_key(key)?.clone();
        let slot = scope.get_mut(&key)?;
        if let Binding::Plain(value) = slot {
            *slot = Binding::Observed(ps_variable::make(&key, value.clone(), local));
        }
        match slot {
            Binding::Observed(obj) => Some(obj.clone()),
            Binding::Injected(slot) => Some(slot.object.clone()),
            _ => None,
        }
    }

    pub(super) fn inject(&mut self, name: String, object: Value, dynamic: bool) {
        let binding = if dynamic {
            Binding::Observed(object)
        } else {
            Binding::Injected(Box::new(InjectedVariable {
                object,
                local: None,
            }))
        };
        self.entries.insert(name, binding);
    }

    pub(super) fn cmdlet_object(&self, name: &str, key: &str) -> Option<Value> {
        let i = self.scope(name).unwrap_or(self.parents.len());
        match self.frame(i).binding(key)? {
            Binding::Observed(obj) => Some(obj.clone()),
            Binding::Injected(slot) => Some(slot.object.clone()),
            _ => None,
        }
    }

    pub(super) fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        self.entries.put(key, value)
    }

    pub(super) fn insert_scoped(&mut self, name: &str, key: String, value: Value) -> Option<Value> {
        let i = self.scope(name).unwrap_or(self.parents.len());
        self.frame_mut(i).put(key, value)
    }

    pub(super) fn remove(&mut self, key: &str) -> Option<Value> {
        self.entries.discard(key)
    }

    pub(super) fn restore_local(&mut self, key: &str, previous: Option<Value>) {
        if let Some(value) = previous {
            self.insert(key.to_owned(), value);
        } else {
            self.remove(key);
        }
    }

    pub(super) fn remove_scoped(&mut self, name: &str, key: &str) -> Option<Value> {
        let i = self.scope(name).unwrap_or(self.parents.len());
        self.frame_mut(i).discard(key)
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries
            .keys()
            .chain(
                self.parents
                    .iter()
                    .enumerate()
                    .rev()
                    .flat_map(move |(i, scope)| {
                        scope.keys().filter(move |key| {
                            self.entries.binding(key).is_none()
                                && self.parents[i + 1..]
                                    .iter()
                                    .all(|p| p.binding(key).is_none())
                        })
                    }),
            )
    }

    /// Environment assignments always belong to the global frame. The process
    /// launcher can borrow their native strings without allocating a copy.
    pub(super) fn environment(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.frame(0).iter().filter_map(|(k, slot)| match slot {
            Binding::Plain(v) if k.starts_with("env:") => Some((k, v)),
            _ => None,
        })
    }

    pub(super) fn environment_value(&self, key: &str) -> Option<&Value> {
        match self.frame(0).get(key) {
            Some(Binding::Plain(v)) => Some(v),
            _ => None,
        }
    }

    pub(super) fn set_invocation(&mut self, args: &[String], path: Option<PathBuf>) {
        self.entries.set_invocation(args, path);
    }

    pub(super) fn set_script_path(&mut self, path: &Path) {
        bind_script_path(&mut self.entries, path);
    }

    pub(super) fn restore_invocation(&mut self, mut caller: Variables) {
        for name in [
            "args",
            "input",
            "_",
            "PSBoundParameters",
            "PSScriptRoot",
            "PSCommandPath",
        ] {
            self.entries.discard(name);
            if let Some(key) = caller.entries.stored_key(name).cloned()
                && let Some(binding) = caller.entries.remove(&key)
            {
                self.entries.insert(key, binding);
            }
        }
    }

    pub(super) fn errors(&self) -> &ArrayList {
        self.errors.get_or_init(|| {
            Stage::ErrorHistory.enter();
            ArrayList::default()
        })
    }

    pub(super) fn restore_session(&mut self, child: &mut Self) {
        if self.errors.get().is_none()
            && let Some(errors) = child.errors.take()
        {
            let _ = self.errors.set(errors);
        }
        self.entries = child.parents.pop().expect("child variable scope");
        self.parents = std::mem::take(&mut child.parents);
    }

    pub(super) fn visible_names(&self) -> Vec<&str> {
        let mut names: Vec<_> = self
            .keys()
            .map(String::as_str)
            .filter(|key| *key != "Error")
            .collect();
        names.push("Error");
        for name in super::preferences::names().filter(|name| !self.contains_key(name)) {
            names.push(name);
        }
        names.sort_unstable();
        names
    }

    pub(super) fn error_value(&self, global: bool) -> Value {
        if !global && let Some(local) = self.get("Error") {
            return local.clone();
        }
        Value::ArrayList(self.errors().clone())
    }
}
