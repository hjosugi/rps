use crate::{PsString, Value};
use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

/// A reference to a mutable System.Collections.ArrayList. Cloning this handle
/// preserves the list's identity when a script assigns it to another variable.
#[derive(Clone, Default)]
pub struct ArrayList(Rc<RefCell<Vec<Value>>>);

impl ArrayList {
    pub const TYPE: &str = "System.Collections.ArrayList";

    pub fn from_values(values: Vec<Value>) -> Self {
        Self(Rc::new(RefCell::new(values)))
    }

    pub fn items(&self) -> Ref<'_, Vec<Value>> {
        self.0.borrow()
    }

    pub fn items_mut(&self) -> RefMut<'_, Vec<Value>> {
        self.0.borrow_mut()
    }

    /// ArrayList doubles its existing capacity; an empty backing store starts
    /// at four slots. Rust's Vec has different growth rules for a capacity of one.
    pub fn reserve(&self, additional: usize) -> Result<(), std::collections::TryReserveError> {
        let mut items = self.items_mut();
        let required = items.len().saturating_add(additional);
        if required > items.capacity() {
            let grown = if items.capacity() == 0 {
                4
            } else {
                items.capacity().saturating_mul(2)
            };
            let extra = required.max(grown) - items.len();
            items.try_reserve_exact(extra)?;
        }
        Ok(())
    }
}

impl fmt::Debug for ArrayList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrayList")
            .field("count", &self.items().len())
            .finish()
    }
}

impl PartialEq for ArrayList {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Debug)]
pub enum Sequence<'a> {
    Array(&'a [Value]),
    List(Ref<'a, Vec<Value>>),
}

impl Deref for Sequence<'_> {
    type Target = [Value];

    fn deref(&self) -> &[Value] {
        match self {
            Self::Array(items) => items,
            Self::List(items) => items,
        }
    }
}

/// PowerShell enumerates one level when converting a collection to text.
/// Nested collections use their .NET type name, even when a list contains itself.
pub(crate) fn sequence_text(items: &[Value], separator: &PsString) -> PsString {
    let mut units = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if index != 0 {
            units.extend(separator.units());
        }
        match item {
            Value::Array(_) | Value::ArrayList(_) => units.extend(item.type_name().encode_utf16()),
            other => units.extend(other.to_ps_string().units()),
        }
    }
    PsString::from_units(units)
}
