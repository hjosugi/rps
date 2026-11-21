//! A JSON writer, enough for result files. The harness stays free of
//! dependencies so that it builds wherever the workspace does.

use std::collections::BTreeMap;
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

/// A parsed JSON value, for reading result files back: `--compare` accepts
/// both the current schema and baselines written before it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub(crate) fn parse(text: &str) -> Result<Value, String> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            offset: 0,
        };
        let value = parser.value(0)?;
        parser.skip_whitespace();
        if parser.offset != parser.bytes.len() {
            return Err(parser.error("trailing input"));
        }
        Ok(value)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(fields) => fields.get(key),
            _ => None,
        }
    }

    pub(crate) fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(number) => Some(*number),
            _ => None,
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 128 {
            return Err(self.error("nesting is too deep"));
        }
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::Text),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number().map(Value::Number),
            Some(_) => Err(self.error("expected a JSON value")),
            None => Err(self.error("expected a JSON value")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, String> {
        self.offset += 1;
        let mut fields = BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.offset += 1;
            return Ok(Value::Object(fields));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.value(depth + 1)?;
            fields.insert(key, value);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.offset += 1,
                Some(b'}') => {
                    self.offset += 1;
                    return Ok(Value::Object(fields));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, String> {
        self.offset += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.offset += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.offset += 1,
                Some(b']') => {
                    self.offset += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error("unterminated string"));
            };
            self.offset += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(escape) = self.peek() else {
                        return Err(self.error("unterminated escape"));
                    };
                    self.offset += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("unknown escape")),
                    }
                }
                byte if byte < 0x20 => return Err(self.error("control character in string")),
                byte if byte < 0x80 => out.push(byte as char),
                _ => {
                    self.offset -= 1;
                    let rest = std::str::from_utf8(&self.bytes[self.offset..])
                        .map_err(|_| self.error("invalid UTF-8"))?;
                    let Some(ch) = rest.chars().next() else {
                        return Err(self.error("unterminated string"));
                    };
                    self.offset += ch.len_utf8();
                    out.push(ch);
                }
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let high = self.hex4()?;
        let code = if (0xD800..=0xDBFF).contains(&high) {
            if self.bytes.get(self.offset..self.offset + 2) != Some(b"\\u") {
                return Err(self.error("expected a low surrogate"));
            }
            self.offset += 2;
            let low = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err(self.error("invalid low surrogate"));
            }
            0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
        } else {
            high
        };
        char::from_u32(code).ok_or_else(|| self.error("invalid unicode escape"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return Err(self.error("truncated unicode escape"));
            };
            self.offset += 1;
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(self.error("expected a hex digit")),
            };
            value = value * 16 + digit as u32;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.offset;
        if self.peek() == Some(b'-') {
            self.offset += 1;
        }
        match self.peek() {
            Some(b'0') => self.offset += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.offset += 1;
                }
            }
            _ => return Err(self.error("expected a digit")),
        }
        if self.peek() == Some(b'.') {
            self.offset += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("expected a digit"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("expected a digit"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.offset += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.offset])
            .map_err(|_| self.error("invalid number"))?;
        text.parse().map_err(|_| self.error("invalid number"))
    }

    fn literal(&mut self, text: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.offset..].starts_with(text.as_bytes()) {
            self.offset += text.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.offset += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", byte as char)))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.offset += 1;
        }
    }

    fn error(&self, expected: &str) -> String {
        format!("{expected} at JSON offset {}", self.offset)
    }
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
