//! Dynamically scoped values and the lazily created, session-wide error list.
use rpsh_runtime::{ArrayList, Value};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

#[derive(Clone, Debug, Default)]
pub(super) struct Variables {
    entries: HashMap<String, Value>,
    errors: OnceCell<ArrayList>,
    pub(super) nested: bool,
}

impl Variables {
    pub(super) fn child(&self) -> Self {
        Self {
            nested: true,
            ..self.clone()
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
        self.extend(child.drain().filter(|(key, _)| key.starts_with("env:")));
    }

    pub(super) fn visible_names(&self) -> Vec<&str> {
        let mut names: Vec<_> = self
            .keys()
            .map(String::as_str)
            .filter(|key| *key != "Error")
            .collect();
        names.push("Error");
        for name in super::preferences::names().filter(|name| !self.contains_key(*name)) {
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

impl Deref for Variables {
    type Target = HashMap<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl DerefMut for Variables {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}
