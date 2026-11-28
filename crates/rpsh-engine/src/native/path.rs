//! PowerShell caches the ordered PATH components. Each invocation looks for
//! the file again, so creating or removing an earlier match takes effect.
use rpsh_runtime::culture::ordinal_compare;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Default)]
pub(crate) struct LookupPaths {
    key: String,
    entries: Vec<PathBuf>,
    // PowerShell's provider location can change independently of the process
    // directory. Plain relative PATH entries use the original process directory.
    process_directory: Option<PathBuf>,
}

fn same_name(a: &str, b: &str) -> bool {
    if a.is_ascii() && b.is_ascii() {
        a.eq_ignore_ascii_case(b)
    } else {
        ordinal_compare(
            &a.encode_utf16().collect::<Vec<_>>(),
            &b.encode_utf16().collect::<Vec<_>>(),
            false,
        )
        .is_eq()
    }
}

fn absolute(path: &Path) -> Option<PathBuf> {
    let full = std::path::absolute(path).ok()?;
    let mut result = PathBuf::new();
    for part in full.components() {
        match part {
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    Some(result)
}

fn file(path: &Path) -> bool {
    // Command discovery includes named symlinks. The actual spawn tells the
    // caller when a selected file or its script interpreter cannot be executed.
    std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir())
}

impl LookupPaths {
    pub(crate) fn remember_process_directory(&mut self) {
        if self.process_directory.is_none() {
            self.process_directory = std::env::current_dir().ok();
        }
    }

    pub(super) fn update(&mut self, path: &str, home: impl Fn() -> Option<String>) {
        if same_name(&self.key, path) {
            return;
        }
        crate::startup::Stage::NativeSearch.enter();
        self.key.clear();
        self.key.push_str(path);
        self.entries.clear();
        for directory in path.split(':').map(str::trim).filter(|dir| !dir.is_empty()) {
            let dir = Path::new(directory);
            let dir = if let Ok(rest) = dir.strip_prefix("~")
                && let Some(home) = home()
            {
                Path::new(&home).join(rest)
            } else {
                dir.to_owned()
            };
            if !self
                .entries
                .iter()
                .any(|other| same_name(&other.to_string_lossy(), &dir.to_string_lossy()))
            {
                self.entries.push(dir);
            }
        }
    }

    pub(super) fn find(&self, name: &str, all: bool) -> Vec<PathBuf> {
        if name.is_empty() {
            return Vec::new();
        }
        if name.contains('/') {
            return absolute(Path::new(name))
                .filter(|path| file(path))
                .into_iter()
                .collect();
        }
        let mut paths = Vec::new();
        for directory in &self.entries {
            let path = if directory.is_relative()
                && !directory.as_os_str().as_encoded_bytes().starts_with(b".")
                && let Some(initial) = &self.process_directory
            {
                initial.join(directory).join(name)
            } else {
                directory.join(name)
            };
            if file(&path)
                && let Some(full) = absolute(&path)
                && !paths.contains(&full)
            {
                paths.push(full);
                if !all {
                    break;
                }
            }
        }
        paths
    }
}
