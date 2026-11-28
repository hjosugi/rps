//! Unix argument expansion in NativeCommandParameterBinder.PossiblyGlobArg.

use std::path::{Component, Path, PathBuf};

use rpsh_runtime::{PsString, culture, wildcard};

pub(super) fn expand(word: String, quoted: bool, mut emit: impl FnMut(String, bool)) {
    if !cfg!(unix) || quoted || needs_quotes(&word) {
        emit(word, false);
    } else if wildcard::contains_wildcard_characters(word.encode_utf16()) {
        if let Some(matches) = expand_paths(&word).filter(|paths| !paths.is_empty()) {
            for path in matches {
                emit(path, true);
            }
        } else {
            emit(word, false);
        }
    } else {
        emit(crate::expand_home(&word), false);
    }
}

// The native binder quotes a value containing whitespace outside embedded
// double quotes before it reaches wildcard or tilde expansion.
pub(super) fn needs_quotes(word: &str) -> bool {
    let mut in_quotes = false;
    let mut after_backslash = false;
    for ch in word.chars() {
        if ch == '"' && !after_backslash {
            in_quotes = !in_quotes;
        } else if ch.is_whitespace() && !in_quotes {
            return true;
        }
        after_backslash = ch == '\\';
    }
    false
}

fn absolute_pattern(cwd: &Path, word: &str) -> PathBuf {
    let mut normalized = PathBuf::new();
    for part in cwd
        .join(crate::expand_home(&word.replace('\\', "/")))
        .components()
    {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            part => normalized.push(part.as_os_str()),
        }
    }
    normalized
}

// NormalizeRelativePath expresses the directory relative to cwd, then
// appends the file's own name. This preserves paths to an ancestor, too.
fn relative_name(path: &Path, cwd: &Path) -> PathBuf {
    let Some(parent) = path.parent() else {
        return path.to_owned();
    };
    let shared = parent
        .components()
        .zip(cwd.components())
        .take_while(|(a, b)| a == b)
        .count();
    let mut relative = PathBuf::new();
    for _ in cwd.components().skip(shared) {
        relative.push("..");
    }
    relative.extend(path.components().skip(shared));
    relative
}

// LocationGlobber converts PowerShell escapes before constructing the leaf
// matcher. Two backticks supply one backtick to that second parser.
fn provider_pattern(component: &str) -> PsString {
    let mut result = String::with_capacity(component.len());
    let mut chars = component.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            result.push(if chars.next_if_eq(&'`').is_some() {
                '`'
            } else {
                '\\'
            });
        } else {
            result.push(ch);
        }
    }
    result.into()
}

fn expand_paths(word: &str) -> Option<Vec<String>> {
    let cwd = std::env::current_dir().ok()?;
    let pattern = absolute_pattern(&cwd, word);
    let mut parts = pattern.components().peekable();
    let mut fixed = PathBuf::new();
    while let Some(part) = parts.next_if(|part| {
        !part
            .as_os_str()
            .to_str()
            .is_some_and(|name| wildcard::contains_wildcard_characters(name.encode_utf16()))
    }) {
        fixed.push(part.as_os_str());
    }
    let mut paths = vec![fixed];
    let locale = culture::current_locale();
    for part in parts {
        let name = part.as_os_str().to_str()?;
        let matcher = wildcard::Matcher::new(&provider_pattern(name), 2).ok()?;
        let mut children = Vec::new();
        for parent in &paths {
            let Ok(entries) = std::fs::read_dir(parent) else {
                continue;
            };
            let mut group = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str().filter(|name| !name.starts_with('.')) else {
                    continue;
                };
                let name = PsString::from(name);
                if matcher.is_match(Some(&name)).ok()? {
                    let path = entry.path();
                    let key = culture::sort_key(&name, &locale, false).ok()?;
                    group.push((!path.is_dir(), key, path));
                }
            }
            group.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            children.extend(group.into_iter().map(|(_, _, path)| path));
        }
        paths = children;
    }
    let absolute = word.starts_with(['~', '/']);
    Some(
        paths
            .into_iter()
            .filter(|path| std::fs::symlink_metadata(path).is_ok())
            .map(|path| {
                let path = if absolute {
                    path
                } else {
                    relative_name(&path, &cwd)
                };
                path.to_string_lossy().into_owned()
            })
            .collect(),
    )
}
