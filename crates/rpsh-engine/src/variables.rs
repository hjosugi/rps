//! Dynamically scoped values and the lazily created, session-wide error list.
use super::startup::Stage;
use rpsh_runtime::{ArrayList, Value};
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

/// Invocation metadata is available immediately, but a cold script that
/// never consults session variables needs neither PS values nor a hash table.
#[derive(Clone, Debug, Default)]
struct Scope {
    table: OnceCell<HashMap<String, Value>>,
    pending: RefCell<Option<Invocation>>,
}

#[derive(Clone, Debug)]
struct Invocation {
    args: Vec<String>,
    path: Option<PathBuf>,
}

fn bind_invocation(table: &mut HashMap<String, Value>, args: &[String], path: Option<&Path>) {
    table.insert(
        "args".into(),
        Value::array(
            args.iter()
                .map(|arg| Value::String(arg.as_str().into()))
                .collect(),
        ),
    );
    if let Some(path) = path {
        bind_script_path(table, path);
    }
}

fn bind_script_path(table: &mut HashMap<String, Value>, path: &Path) {
    table.insert(
        "PSCommandPath".into(),
        Value::String(path.to_string_lossy().as_ref().into()),
    );
    if let Some(parent) = path.parent() {
        table.insert(
            "PSScriptRoot".into(),
            Value::String(parent.to_string_lossy().as_ref().into()),
        );
    }
}

impl Scope {
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

    fn initialize(pending: &RefCell<Option<Invocation>>) -> HashMap<String, Value> {
        Stage::Variables.enter();
        let mut table = HashMap::new();
        if let Some(invocation) = pending.take() {
            bind_invocation(&mut table, &invocation.args, invocation.path.as_deref());
        }
        table
    }
}

impl Deref for Scope {
    type Target = HashMap<String, Value>;
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

    pub(super) fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .get(key)
            .or_else(|| self.parents.iter().rev().find_map(|p| p.get(key)))
    }

    pub(super) fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub(super) fn get_scoped(&self, name: &str, key: &str) -> Option<&Value> {
        match self.scope(name) {
            Some(i) => self.parents.get(i).unwrap_or(&self.entries).get(key),
            None => self.get(key),
        }
    }

    pub(super) fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        self.entries.insert(key, value)
    }

    pub(super) fn insert_scoped(&mut self, name: &str, key: String, value: Value) -> Option<Value> {
        let i = self.scope(name).unwrap_or(self.parents.len());
        self.parents
            .get_mut(i)
            .unwrap_or(&mut self.entries)
            .insert(key, value)
    }

    pub(super) fn remove(&mut self, key: &str) -> Option<Value> {
        self.entries.remove(key)
    }

    pub(super) fn remove_scoped(&mut self, name: &str, key: &str) -> Option<Value> {
        let i = self.scope(name).unwrap_or(self.parents.len());
        self.parents
            .get_mut(i)
            .unwrap_or(&mut self.entries)
            .remove(key)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.entries
            .iter()
            .chain(
                self.parents
                    .iter()
                    .enumerate()
                    .rev()
                    .flat_map(move |(i, scope)| {
                        scope.iter().filter(move |(key, _)| {
                            !self.entries.contains_key(*key)
                                && self.parents[i + 1..].iter().all(|p| !p.contains_key(*key))
                        })
                    }),
            )
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &String> {
        self.iter().map(|(name, _)| name)
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
            match caller.remove(name) {
                Some(value) => {
                    self.insert(name.to_owned(), value);
                }
                None => {
                    self.remove(name);
                }
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
