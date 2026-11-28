//! Dynamically scoped values and the lazily created, session-wide error list.
use rpsh_runtime::{ArrayList, Value};
use std::cell::OnceCell;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub(super) struct Variables {
    entries: HashMap<String, Value>,
    errors: OnceCell<ArrayList>,
    parents: Vec<HashMap<String, Value>>,
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

    pub(super) fn set_script_path(&mut self, path: &std::path::Path) {
        self.insert(
            "PSCommandPath".into(),
            Value::String(path.to_string_lossy().as_ref().into()),
        );
        if let Some(parent) = path.parent() {
            self.insert(
                "PSScriptRoot".into(),
                Value::String(parent.to_string_lossy().as_ref().into()),
            );
        }
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
        self.errors.get_or_init(ArrayList::default)
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
