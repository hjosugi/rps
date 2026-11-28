//! CultureInfo and normalization arguments for System.String.
use super::{hidden_type, overload_error};
use culture::{NormalizationError, NormalizationForm, StringComparison};
use rpsh_runtime::{PsString, RuntimeError, Value, culture};

pub(super) const CULTURE_TYPE: &str = "System.Globalization.CultureInfo";
pub(super) const FORM_TYPE: &str = "System.Text.NormalizationForm";
pub(super) const COMPARISON_TYPE: &str = "System.StringComparison";
type EnumEntry<T> = (&'static str, i32, T);
struct EnumTable<T: 'static> {
    ty: &'static str,
    entries: &'static [EnumEntry<T>],
}
const FORMS: EnumTable<NormalizationForm> = EnumTable {
    ty: FORM_TYPE,
    entries: &[
        ("FormC", 1, NormalizationForm::C),
        ("FormD", 2, NormalizationForm::D),
        ("FormKC", 5, NormalizationForm::Kc),
        ("FormKD", 6, NormalizationForm::Kd),
    ],
};
const COMPARISONS: EnumTable<StringComparison> = EnumTable {
    ty: COMPARISON_TYPE,
    entries: StringComparison::ENTRIES,
};

pub(super) fn invocation(
    id: &'static str,
    method: &str,
    count: usize,
    detail: &str,
) -> RuntimeError {
    RuntimeError::Script {
        id,
        category: "NotSpecified: (:) [], MethodInvocationException".into(),
        exception_type: "System.Management.Automation.MethodInvocationException",
        message: format!(
            "Exception calling \"{method}\" with \"{count}\" argument(s): \"{detail}\""
        ),
    }
}

pub(super) fn argument_error(
    method: &str,
    parameter: &str,
    ty: &str,
    text: &str,
    detail: &str,
) -> RuntimeError {
    RuntimeError::Script {
        id: "MethodArgumentConversionInvalidCastArgument",
        category: "NotSpecified: (:) [], MethodException".into(),
        exception_type: "System.Management.Automation.MethodException",
        message: format!(
            "Cannot convert argument \"{parameter}\", with value: \"{text}\", for \"{method}\" to type \"{ty}\": \"{detail}\""
        ),
    }
}

fn cast_error(id: &'static str, detail: String) -> RuntimeError {
    RuntimeError::Script {
        id,
        category: "InvalidArgument: (:) [], RuntimeException".into(),
        exception_type: "System.Management.Automation.RuntimeException",
        message: detail,
    }
}

pub(super) fn current_name() -> String {
    let locale = culture::current_locale();
    culture::locale_details(&locale).map_or_else(|| locale.replace('_', "-"), |(name, _)| name)
}

fn details(value: &Value) -> Result<(String, u32), String> {
    if let Value::Object(entries) = value
        && hidden_type(value).as_deref() == Some(CULTURE_TYPE)
        && let Some((_, Value::Int32(lcid))) = entries.iter().find(|(k, _)| k == "LCID")
    {
        return Ok((value.to_string(), *lcid as u32));
    }
    let text = value.to_string();
    if matches!(
        value,
        Value::Int32(_) | Value::Int64(_) | Value::UInt32(_) | Value::Int16(_) | Value::Byte(_)
    ) {
        let id = text.parse::<i32>().unwrap_or(-1);
        if let Ok(id) = u32::try_from(id)
            && let Some(name) = culture::locale_for_lcid(id)
            && let Some((name, _)) = culture::locale_details(&name)
        {
            return Ok((name, id));
        }
        return Err(format!(
            "Culture is not supported. (Parameter 'culture')\n{id} (0x{id:04x}) is an invalid culture identifier."
        ));
    }
    if let Some((name, lcid)) = culture::locale_details(&text) {
        return Ok((name, lcid));
    }
    Err(format!(
        "Culture is not supported. (Parameter 'name')\n{text} is an invalid culture identifier."
    ))
}

fn culture_object(name: String, lcid: u32) -> Value {
    let language = if name.is_empty() {
        "iv"
    } else {
        name.split('-').next().unwrap_or(&name)
    };
    let text = PsString::from(name.as_str());
    Value::object(vec![
        ("__TypeName".into(), Value::String(CULTURE_TYPE.into())),
        ("__Text".into(), Value::String(text.clone())),
        ("Name".into(), Value::String(text)),
        ("LCID".into(), Value::Int32(lcid as i32)),
        (
            "TwoLetterISOLanguageName".into(),
            Value::String(language.into()),
        ),
    ])
}

pub(super) fn current_culture() -> Value {
    let locale = culture::current_locale();
    let (name, id) = culture::locale_details(&locale).unwrap_or((locale, 4096));
    culture_object(name, id)
}

pub(super) fn invariant_culture() -> Value {
    culture_object(String::new(), 127)
}

pub(super) fn culture_property(value: &Value, name: &str) -> Option<Result<Value, RuntimeError>> {
    if !name.eq_ignore_ascii_case("DateTimeFormat")
        || hidden_type(value).as_deref() != Some(CULTURE_TYPE)
    {
        return None;
    }
    let locale = value.to_string();
    let pattern = |short| {
        culture::time_pattern(&locale, short)
            .map(|v| Value::String(v.into()))
            .ok_or(RuntimeError::NotImplemented {
                feature: "ICU time patterns",
            })
    };
    Some((|| {
        Ok(Value::object(vec![
            ("LongTimePattern".into(), pattern(false)?),
            ("ShortTimePattern".into(), pattern(true)?),
        ]))
    })())
}

pub(super) fn cast_culture(value: &Value) -> Result<Value, RuntimeError> {
    if value.is_null_like() {
        return Ok(Value::Null);
    }
    details(value)
        .map(|(name, id)| culture_object(name, id))
        .map_err(|why| {
            cast_error(
                "InvalidCastConstructorException",
                format!(
                    "Cannot convert value \"{value}\" to type \"{CULTURE_TYPE}\". Error: \"{why}\""
                ),
            )
        })
}

pub(super) fn culture_method(name: &str, args: &[Value]) -> Result<Value, RuntimeError> {
    let method = if name == "new" {
        ".ctor"
    } else {
        "GetCultureInfo"
    };
    let value = match args {
        [value] => value,
        [value, Value::Bool(_)] if name == "new" => value,
        _ => return Err(overload_error(method, args.len())),
    };
    details(value)
        .map(|(name, id)| culture_object(name, id))
        .map_err(|why| invocation("CultureNotFoundException", method, args.len(), &why))
}

pub(super) fn method_locale(method: &str, value: &Value) -> Result<String, RuntimeError> {
    if value.is_null_like() {
        return Ok(culture::current_locale());
    }
    if hidden_type(value).as_deref() == Some("System.Management.Automation.Language.NullString") {
        let detail = format!(
            "Cannot convert the \"\" value of type \"System.Management.Automation.Language.NullString\" to type \"{CULTURE_TYPE}\"."
        );
        return Err(argument_error(method, "culture", CULTURE_TYPE, "", &detail));
    }
    details(value)
        .map_err(|why| {
            let detail = format!(
                "Cannot convert value \"{value}\" to type \"{CULTURE_TYPE}\". Error: \"{why}\""
            );
            argument_error(method, "culture", CULTURE_TYPE, &value.to_string(), &detail)
        })
        .map(|(name, _)| name)
}

pub(super) fn case_method(
    text: &PsString,
    method: &str,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let upper = method.starts_with("ToUpper");
    let locale = match (method.ends_with("Invariant"), args) {
        (true, []) => String::new(),
        (false, []) => culture::current_locale(),
        (false, [value]) if value.is_null_like() => culture::current_locale(),
        (false, [value]) => method_locale(method, value)?,
        _ => return Err(overload_error(method, args.len())),
    };
    culture::change_case(text, &locale, upper).map(Value::String)
}

impl<T> EnumTable<T> {
    fn entry(&self, text: &str) -> Option<&'static EnumEntry<T>> {
        self.entries
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case(text))
    }
    fn value(&self, entry: &EnumEntry<T>) -> Value {
        let (name, number, _) = entry;
        Value::object(vec![
            ("__TypeName".into(), Value::String(self.ty.into())),
            ("__Text".into(), Value::String((*name).into())),
            ("__EnumValue".into(), Value::Int32(*number)),
            ("value__".into(), Value::Int32(*number)),
        ])
    }
    fn parse(&self, value: &Value) -> Result<&'static EnumEntry<T>, (&'static str, String)> {
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

    fn cast(&self, value: &Value) -> Result<Value, RuntimeError> {
        self.parse(value)
            .map(|entry| self.value(entry))
            .map_err(|(id, message)| cast_error(id, message))
    }
}

pub(super) fn form_member(name: &str) -> Option<Value> {
    FORMS.entry(name).map(|e| FORMS.value(e))
}
pub(super) fn cast_form(value: &Value) -> Result<Value, RuntimeError> {
    FORMS.cast(value)
}
pub(super) fn comparison_member(name: &str) -> Option<Value> {
    COMPARISONS.entry(name).map(|e| COMPARISONS.value(e))
}
pub(super) fn cast_comparison(value: &Value) -> Result<Value, RuntimeError> {
    COMPARISONS.cast(value)
}
pub(super) fn comparison(method: &str, value: &Value) -> Result<StringComparison, RuntimeError> {
    COMPARISONS
        .parse(value)
        .map(|entry| entry.2)
        .map_err(|(_, detail)| {
            argument_error(
                method,
                "comparisonType",
                COMPARISON_TYPE,
                &value.to_string(),
                &detail,
            )
        })
}

pub(super) fn normalization(
    text: &PsString,
    method: &str,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let form = match args {
        [] => NormalizationForm::C,
        [value] if !value.is_null_like() => {
            FORMS
                .parse(value)
                .map(|entry| entry.2)
                .map_err(|(_, detail)| {
                    argument_error(
                        method,
                        "normalizationForm",
                        FORM_TYPE,
                        &value.to_string(),
                        &detail,
                    )
                })?
        }
        _ => return Err(overload_error(method, args.len())),
    };
    let is_normalized = method == "IsNormalized";
    let result = if is_normalized {
        form.is_normalized(text).map(Value::Bool)
    } else {
        form.normalize(text).map(Value::String)
    };
    result.map_err(|error| match error {
        NormalizationError::Unavailable => RuntimeError::NotImplemented {
            feature: "system ICU normalization",
        },
        NormalizationError::InvalidUnicode => {
            let parameter = if is_normalized { "source" } else { "strInput" };
            invocation(
                "ArgumentException",
                method,
                args.len(),
                &format!("String contains invalid Unicode code points. (Parameter '{parameter}')"),
            )
        }
    })
}
