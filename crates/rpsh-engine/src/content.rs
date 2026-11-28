//! Text and byte content for filesystem, variable and environment paths.
use super::{
    Engine, EngineError, RuntimeError, Value, automatic_variable, convert_int32, environment,
    expand_glob, expand_home, ops, strip_scope,
};
impl Engine {
    pub(super) fn read_plain_content(
        &self,
        paths: &[String],
        named: &[(String, Value)],
    ) -> Result<Vec<Value>, EngineError> {
        if paths.iter().any(|path| {
            path.split_once(':').is_some_and(|(provider, name)| {
                !name.is_empty() && matches!(provider.to_ascii_lowercase().as_str(), "alias")
            })
        }) {
            return Err(RuntimeError::NotImplemented {
                feature: "Get-Content for this provider",
            }
            .into());
        }
        if named
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("asbytestream") && ops::is_true(value))
        {
            let mut outputs = Vec::new();
            for path in paths {
                let path = expand_home(path);
                let files = if path.contains(['*', '?']) {
                    expand_glob(&path)
                } else {
                    vec![path]
                };
                for file in files {
                    let bytes = std::fs::read(&file).map_err(|_| RuntimeError::NotImplemented {
                        feature: "Get-Content of a missing file",
                    })?;
                    outputs.extend(bytes.into_iter().map(Value::Byte));
                }
            }
            return Ok(outputs);
        }
        let mut text = String::new();
        for path in paths {
            if let Some((provider, name)) = path.split_once(':') {
                let name = name.trim_start_matches("\\");
                if !name.is_empty() {
                    let value = if provider.eq_ignore_ascii_case("env") {
                        environment::get(&self.variables, name)
                    } else if provider.eq_ignore_ascii_case("variable") {
                        let key = strip_scope(name);
                        self.variables
                            .get(key)
                            .cloned()
                            .or_else(|| automatic_variable(key))
                    } else {
                        None
                    };
                    if let Some(value) = value {
                        if !text.is_empty() && !text.ends_with('\n') {
                            text.push('\n');
                        }
                        text.push_str(&value.to_string());
                        continue;
                    }
                }
            }
            let path = expand_home(path);
            if path.contains(['*', '?']) {
                for matched in expand_glob(&path) {
                    let Ok(content) = std::fs::read_to_string(&matched) else {
                        continue;
                    };
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str(&content);
                }
            } else {
                let content =
                    std::fs::read_to_string(&path).map_err(|_| RuntimeError::NotImplemented {
                        feature: "Get-Content of a missing file",
                    })?;
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&content);
            }
        }
        if named
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("raw") && ops::is_true(value))
        {
            return Ok(vec![Value::String(text.into())]);
        }
        if let Some((_, delimiter)) = named
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("delimiter"))
        {
            let delimiter = delimiter.to_string();
            let mut parts: Vec<Value> = text
                .split(&delimiter)
                .map(|part| Value::String(part.into()))
                .collect();
            if text.ends_with(&delimiter) {
                parts.pop();
            }
            return Ok(parts);
        }
        let mut lines: Vec<Value> = text
            .split('\n')
            .map(|line| Value::String(line.trim_end_matches('\r').into()))
            .collect();
        if text.ends_with('\n') {
            lines.pop();
        }
        if let Some((_, count)) = named.iter().find(|(name, _)| {
            name.eq_ignore_ascii_case("totalcount")
                || name.eq_ignore_ascii_case("first")
                || name.eq_ignore_ascii_case("head")
        }) {
            let count = convert_int32(count)?.max(0) as usize;
            lines.truncate(count);
        }
        if let Some((_, count)) = named.iter().find(|(name, _)| {
            name.eq_ignore_ascii_case("tail") || name.eq_ignore_ascii_case("last")
        }) {
            let count = convert_int32(count)?.max(0) as usize;
            if lines.len() > count {
                lines = lines.split_off(lines.len() - count);
            }
        }
        Ok(lines)
    }
    pub(super) fn write_plain_content(
        &mut self,
        paths: &[String],
        values: &[Value],
        named: &[(String, Value)],
    ) -> Result<(), EngineError> {
        let no_newline = named
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("nonewline") && ops::is_true(value));
        let mut text = String::new();
        let provider_paths = paths.iter().all(|path| {
            path.split_once(':').is_some_and(|(provider, name)| {
                !name.is_empty()
                    && (provider.eq_ignore_ascii_case("env")
                        || provider.eq_ignore_ascii_case("variable"))
            })
        });
        if provider_paths {
            let text = values
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            for path in paths {
                let Some((provider, name)) = path.split_once(':') else {
                    continue;
                };
                let name = name.trim_start_matches("\\");
                let value = Value::String(text.as_str().into());
                if provider.eq_ignore_ascii_case("env") {
                    environment::set(&mut self.variables, name, &value);
                } else {
                    self.variables.insert(strip_scope(name).to_owned(), value);
                }
            }
            return Ok(());
        }
        if named
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("asbytestream") && ops::is_true(value))
        {
            let bytes: Vec<u8> = values
                .iter()
                .map(|value| convert_int32(value).map(|value| value as u8))
                .collect::<Result<_, _>>()?;
            for path in paths {
                std::fs::write(path, &bytes).map_err(|_| RuntimeError::NotImplemented {
                    feature: "Set-Content of an unwritable file",
                })?;
            }
            return Ok(());
        }
        for item in values {
            text.push_str(&item.to_string());
            if !no_newline {
                text.push('\n');
            }
        }
        for path in paths {
            let files = if path.contains(['*', '?']) {
                expand_glob(path)
            } else {
                vec![path.to_owned()]
            };
            for path in files {
                std::fs::write(&path, &text).map_err(|_| RuntimeError::NotImplemented {
                    feature: "Set-Content of an unwritable file",
                })?;
            }
        }
        Ok(())
    }
}
