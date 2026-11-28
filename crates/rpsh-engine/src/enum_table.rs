//! PowerShell enum parsing and boxed values for .NET adapters.
use super::method_error::cast_error;
use rpsh_runtime::{RuntimeError, Value};

pub(super) type EnumEntry<T> = (&'static str, i32, T);
pub(super) struct EnumTable<T: 'static> {
    pub(super) ty: &'static str,
    pub(super) entries: &'static [EnumEntry<T>],
}
impl<T> EnumTable<T> {
    pub(super) fn entry(&self, text: &str) -> Option<&'static EnumEntry<T>> {
        self.entries
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case(text))
    }
    pub(super) fn value(&self, entry: &EnumEntry<T>) -> Value {
        let (name, number, _) = entry;
        Value::object(vec![
            ("__TypeName".into(), Value::String(self.ty.into())),
            ("__Text".into(), Value::String((*name).into())),
            ("__EnumValue".into(), Value::Int32(*number)),
            ("value__".into(), Value::Int32(*number)),
        ])
    }
    pub(super) fn parse(
        &self,
        value: &Value,
    ) -> Result<&'static EnumEntry<T>, (&'static str, String)> {
        let ty = self.ty;
        let names = |separator: &str| {
            self.entries
                .iter()
                .map(|(name, _, _)| *name)
                .collect::<Vec<_>>()
                .join(separator)
        };
        let text = value.to_string();
        let named_error = |reason: String| {
            (
                "SubstringDisambiguationEnumParseThrewAnException",
                format!("Cannot convert value \"{text}\" to type \"{ty}\". Error: \"{reason}\""),
            )
        };
        let no_match = || {
            named_error(format!(
                "Unable to match the identifier name {text} to a valid enumerator name. Specify one of the following enumerator names and try again:\n{}",
                names(", ")
            ))
        };
        let invalid = |id, source: String| {
            (
                id,
                format!(
                    "Cannot convert {source} to type \"{ty}\" due to enumeration values that are not valid. Specify one of the following enumeration values and try again. The possible enumeration values are \"{}\".",
                    names(",")
                ),
            )
        };
        if value.is_null_like() {
            return Err(invalid("nullToEnumInvalidCast", "null".into()));
        }
        if matches!(value, Value::Bool(_)) {
            return Err((
                "InvalidCastIConvertible",
                format!(
                    "Cannot convert value \"{text}\" to type \"{ty}\". Error: \"Invalid cast from 'System.Boolean' to '{ty}'.\""
                ),
            ));
        }
        let trimmed = text.trim();
        if let Some(entry) = self.entry(trimmed) {
            return Ok(entry);
        }
        let number = if let Ok(num) = trimmed.parse::<i32>() {
            num
        } else if trimmed.contains(',') {
            let mut result = 0;
            for word in trimmed.split(',') {
                result |= self.entry(word.trim()).ok_or_else(no_match)?.1;
            }
            result
        } else {
            let prefix = trimmed.to_ascii_lowercase();
            let matches: Vec<_> = self
                .entries
                .iter()
                .filter(|(name, _, _)| name.to_ascii_lowercase().starts_with(&prefix))
                .collect();
            match matches.as_slice() {
                [entry] => return Ok(entry),
                [] => return Err(no_match()),
                _ => {
                    let names = matches
                        .iter()
                        .map(|(name, _, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(named_error(format!(
                        "The identifier name {text} cannot be processed because it is either too similar or identical to the following enumerator names: {names}. Use a more specific identifier name."
                    )));
                }
            }
        };
        self.entries
            .iter()
            .find(|(_, n, _)| *n == number)
            .ok_or_else(|| {
                invalid(
                    if matches!(value, Value::String(_)) {
                        "EnumParseUndefined"
                    } else {
                        "UndefinedIntegerToEnum"
                    },
                    format!("value \"{text}\""),
                )
            })
    }

    pub(super) fn cast(&self, value: &Value) -> Result<Value, RuntimeError> {
        self.parse(value)
            .map(|entry| self.value(entry))
            .map_err(|(id, message)| cast_error(id, message))
    }
}
