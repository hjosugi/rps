//! A JSON writer, enough for result files. The harness stays free of
//! dependencies so that it builds wherever the workspace does.

use std::fmt::Write;

pub(crate) enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Json>),
    Object(Vec<(&'static str, Json)>),
}

impl Json {
    pub(crate) fn text(value: impl Into<String>) -> Json {
        Json::Text(value.into())
    }

    pub(crate) fn optional<T>(value: Option<T>, convert: impl FnOnce(T) -> Json) -> Json {
        value.map_or(Json::Null, convert)
    }

    /// Two-space indented JSON with a trailing newline.
    pub(crate) fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Number(value) if !value.is_finite() => out.push_str("null"),
            Json::Number(value) => {
                let _ = write!(out, "{value}");
            }
            Json::Text(value) => write_string(out, value),
            Json::Array(items) if items.is_empty() => out.push_str("[]"),
            Json::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    out.push_str(if index == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    item.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push(']');
            }
            Json::Object(fields) if fields.is_empty() => out.push_str("{}"),
            Json::Object(fields) => {
                out.push('{');
                for (index, (key, value)) in fields.iter().enumerate() {
                    out.push_str(if index == 0 { "\n" } else { ",\n" });
                    indent(out, depth + 1);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                }
                out.push('\n');
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_nested_values() {
        let value = Json::Object(vec![
            ("name", Json::text("a \"b\"\n\u{1}")),
            ("count", Json::Number(3.0)),
            ("ratio", Json::Number(1.25)),
            ("missing", Json::optional(None::<f64>, Json::Number)),
            (
                "items",
                Json::Array(vec![Json::Bool(true), Json::Array(vec![])]),
            ),
            ("empty", Json::Object(vec![])),
        ]);
        let expected = r#"{
  "name": "a \"b\"\n\u0001",
  "count": 3,
  "ratio": 1.25,
  "missing": null,
  "items": [
    true,
    []
  ],
  "empty": {}
}
"#;
        assert_eq!(value.pretty(), expected);
    }

    #[test]
    fn non_finite_numbers_become_null() {
        assert_eq!(Json::Number(f64::NAN).pretty(), "null\n");
    }
}
