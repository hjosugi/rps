//! Installed .help.txt topics. Search user and module directories on demand,
//! with the same language fallback and module-version preference as PowerShell.
use super::{TYPE_NAMES, View, string};
use crate::{Variables, encoding, environment, globalization};
use rpsh_runtime::{ArrayList, PsString, Value, culture, ops};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

const DEFAULT: &str = include_str!("../../assets/en-US/default.help.txt");
const EXTENSION: &str = ".help.txt";

#[derive(Debug)]
struct Source {
    path: PathBuf,
    name: String,
    version: Option<(String, [i32; 4])>,
    bundled: bool,
}

#[derive(Debug)]
struct Document {
    name: String,
    content: PsString,
    synopsis: String,
    // A PSObject materializes its mutable TypeNames only on first access.
    // A short-help wrapper shares that collection if the base already has one.
    type_names: ArrayList,
}

const BASE_NAMES: &str = "__HelpFileTypes";

pub(crate) fn base_names(slot: &ArrayList) -> Value {
    let mut stored = slot.items_mut();
    if stored.is_empty() {
        stored.push(Value::collection(
            &TYPE_NAMES,
            vec![string("System.String"), string("System.Object")],
        ));
    }
    stored[0].clone()
}

pub(crate) fn names_slot(fields: &[(String, Value)]) -> Option<&ArrayList> {
    fields.iter().find_map(|(key, value)| match value {
        Value::ArrayList(slot) if key == BASE_NAMES => Some(slot),
        _ => None,
    })
}

impl Document {
    fn value(&self, multiple: bool, view: View) -> Value {
        let mut fields = vec![
            ("Name".into(), string(&self.name)),
            ("Category".into(), string("HelpFile")),
            ("Synopsis".into(), string(&self.synopsis)),
            ("Component".into(), Value::Null),
            ("Role".into(), Value::Null),
            ("Functionality".into(), Value::Null),
            ("Length".into(), Value::Int32(self.content.len() as i32)),
            ("__TypeName".into(), string("System.String")),
            ("__Text".into(), Value::String(self.content.clone())),
            (BASE_NAMES.into(), Value::ArrayList(self.type_names.clone())),
        ];
        let explicit = if multiple {
            let short = vec![string("HelpInfoShort")];
            if let Some(Value::Collection(existing)) = self.type_names.items().first() {
                *existing.items.items_mut() = short;
                Some(Value::Collection(existing.clone()))
            } else {
                Some(Value::collection(&TYPE_NAMES, short))
            }
        } else if view != View::Default {
            let base = base_names(&self.type_names);
            let original = base.sequence().expect("file help type names");
            let mut names: Vec<_> = original
                .iter()
                .map(ToString::to_string)
                .filter(|s| {
                    !s.eq_ignore_ascii_case("System.String")
                        && !s.eq_ignore_ascii_case("System.Object")
                })
                .map(|s| string(format!("{s}#{}View", view.label())))
                .collect();
            if original.is_empty() {
                names.push(string(format!("HelpInfo#{}View", view.label())));
            }
            names.extend(original.iter().cloned());
            Some(Value::collection(&TYPE_NAMES, names))
        } else {
            None
        };
        if let Some(names) = explicit {
            fields.push(("__PSTypeNames".into(), names));
        }
        Value::object(fields)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Cache {
    culture: String,
    documents: HashMap<PathBuf, Document>,
}

impl Cache {
    fn load(&mut self, source: &Source) -> Option<&mut Document> {
        use std::collections::hash_map::Entry;
        match self.documents.entry(source.path.clone()) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                let content = if source.bundled {
                    DEFAULT.to_owned()
                } else {
                    let bytes = std::fs::read(&source.path).ok()?;
                    encoding::decode_with_bom(&bytes).into_owned()
                };
                let synopsis = content.lines().nth(4).unwrap_or_default().trim().to_owned();
                Some(entry.insert(Document {
                    name: source.name.clone(),
                    content: content.into(),
                    synopsis,
                    type_names: ArrayList::default(),
                }))
            }
        }
    }

    pub(crate) fn search(&mut self, target: &str, vars: &Variables, view: View) -> Vec<Value> {
        let locale = globalization::current_name();
        if self.culture != locale {
            self.documents.clear();
            self.culture = locale.clone();
        }
        let mut folders = Folders::new(&locale);
        let path = Path::new(target);
        let explicit = path.parent().is_some_and(|p| !p.as_os_str().is_empty());
        let target = if explicit {
            folders.add(path.parent().unwrap(), None);
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
        } else {
            folders.defaults(vars);
            target
        };
        let wildcard = target.contains(['*', '?', '[']);
        let exact: Vec<_> = folders
            .files
            .iter()
            .filter(|s| s.name.eq_ignore_ascii_case(target))
            .collect();
        let matches = if !exact.is_empty() {
            exact
        } else {
            let pattern = if wildcard {
                target.to_owned()
            } else {
                format!("*{target}*")
            };
            let names: Vec<_> = folders
                .files
                .iter()
                .filter(|s| ops::wildcard_match(&s.name, &pattern, false))
                .collect();
            if !names.is_empty() {
                names
            } else {
                folders
                    .files
                    .iter()
                    .filter(|s| {
                        self.load(s).is_some_and(|d| {
                            ops::wildcard_match(&d.content.to_string_lossy(), &pattern, false)
                        })
                    })
                    .collect()
            }
        };
        let multiple = matches.len() > 1;
        matches
            .into_iter()
            .filter_map(|s| self.load(s).map(|d| d.value(multiple, view)))
            .collect()
    }
}

struct Folders {
    languages: Vec<String>,
    files: Vec<Source>,
    seen: HashSet<PathBuf>,
}

impl Folders {
    fn new(locale: &str) -> Self {
        let mut languages = Vec::new();
        let mut next = locale;
        while !next.is_empty() {
            languages.push(next.to_owned());
            next = next.rsplit_once('-').map_or("", |(parent, _)| parent);
        }
        for fallback in ["", "en-US", "en"] {
            if !languages.iter().any(|l| l == fallback) {
                languages.push(fallback.to_owned());
            }
        }
        Self {
            languages,
            files: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn defaults(&mut self, vars: &Variables) {
        let home = environment::ps_home();
        self.add(home, None);
        if !self
            .files
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case("default"))
        {
            self.files.push(Source {
                path: home.join("en-US/default.help.txt"),
                name: "default".into(),
                version: None,
                bundled: true,
            });
        }
        let env_path = |name| {
            environment::get(vars, name)
                .map(|v| v.to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        };
        let data =
            env_path("XDG_DATA_HOME").or_else(|| env_path("HOME").map(|p| p.join(".local/share")));
        if let Some(path) = data {
            self.add(&path.join("powershell/Help"), None);
        }
        if let Some(paths) = environment::get(vars, "PSModulePath") {
            for root in
                std::env::split_paths(&paths.to_string()).filter(|p| !p.as_os_str().is_empty())
            {
                self.module_tree(&root);
            }
        }
        self.deduplicate();
    }

    fn add(&mut self, directory: &Path, module_root: Option<&Path>) {
        let Ok(canonical) = directory.canonicalize() else {
            return;
        };
        if !self.seen.insert(canonical) {
            return;
        }
        let version = module_root.and_then(|root| module_version(root, directory));
        let mut selected = HashSet::new();
        for language in &self.languages {
            let Ok(entries) = std::fs::read_dir(directory.join(language)) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(filename) = name.to_str() else {
                    continue;
                };
                let Some(at) = filename.len().checked_sub(EXTENSION.len()) else {
                    continue;
                };
                if !filename
                    .get(at..)
                    .is_some_and(|s| s.eq_ignore_ascii_case(EXTENSION))
                    || !entry.path().is_file()
                {
                    continue;
                }
                if !selected.insert(filename.to_owned()) {
                    continue;
                }
                self.files.push(Source {
                    path: entry.path(),
                    name: filename[..at].to_owned(),
                    version: version.clone(),
                    bundled: false,
                });
            }
        }
    }

    fn module_tree(&mut self, root: &Path) {
        let mut pending = VecDeque::from([root.to_owned()]);
        let mut visited = HashSet::new();
        while let Some(parent) = pending.pop_front() {
            let Ok(id) = parent.canonicalize() else {
                continue;
            };
            if !visited.insert(id) {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&parent) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name();
                if !resource_directory(&name.to_string_lossy())
                    && std::fs::read_dir(&path).is_ok_and(|mut entries| {
                        entries.any(|entry| entry.is_ok_and(|file| file.path().is_file()))
                    })
                {
                    self.add(&path, Some(root));
                }
                pending.push_back(path);
            }
        }
    }

    fn deduplicate(&mut self) {
        let mut latest = HashMap::new();
        for file in &self.files {
            if let Some((module, version)) = &file.version {
                let best = latest
                    .entry((module.clone(), file.name.clone()))
                    .or_insert(*version);
                *best = (*best).max(*version);
            }
        }
        let mut seen = HashSet::new();
        self.files.retain(|file| {
            // Upstream checks the first filename across scopes even if its
            // module version was separately excluded by the version filter.
            seen.insert(file.name.clone())
                && file
                    .version
                    .as_ref()
                    .is_none_or(|(module, v)| latest[&(module.clone(), file.name.clone())] == *v)
        });
    }
}

fn resource_directory(name: &str) -> bool {
    let letters = |s: &str| s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic());
    (letters(name)
        || name
            .split_once('-')
            .is_some_and(|(a, b)| letters(a) && letters(b)))
        && culture::locale_details(name).is_some_and(|(_, id)| id != 4096)
}

fn module_version(root: &Path, directory: &Path) -> Option<(String, [i32; 4])> {
    let mut parts = directory.strip_prefix(root).ok()?.components();
    let module = parts.next()?.as_os_str().to_str()?;
    let raw = parts.next()?.as_os_str().to_str()?;
    let numbers = raw
        .split('.')
        .map(str::parse::<i32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if !(2..=4).contains(&numbers.len()) || numbers.iter().any(|&n| n < 0) {
        return None;
    }
    let mut version = [-1; 4];
    version[..numbers.len()].copy_from_slice(&numbers);
    Some((module.to_owned(), version))
}

pub(crate) fn is_topic(o: &Value) -> bool {
    super::format::field(o, BASE_NAMES).is_some()
}
pub(crate) fn short(o: &Value) -> bool {
    let Value::Object(fields) = o else {
        return false;
    };
    let Some(slot) = names_slot(fields) else {
        return false;
    };
    let base = slot.items();
    super::format::field(o, "__PSTypeNames")
        .or_else(|| base.first())
        .and_then(Value::sequence)
        .is_some_and(|names| {
            names
                .iter()
                .any(|v| v.to_string().eq_ignore_ascii_case("HelpInfoShort"))
        })
}
pub(crate) fn format(items: &[Value], width: usize) -> Option<String> {
    let first = items.first()?;
    if !is_topic(first) || super::format::field(first, "__Format").is_some() {
        return None;
    }
    if !short(first) {
        if items
            .iter()
            .any(|v| !is_topic(v) || short(v) || super::format::field(v, "__Format").is_some())
        {
            return None;
        }
        let mut out = String::new();
        for item in items {
            out.push_str(&item.to_string());
            out.push('\n');
        }
        return Some(out);
    }
    let rows: Vec<_> = items
        .iter()
        .filter(|v| !v.is_null_like())
        .map(|v| {
            if matches!(v, Value::Object(_)) {
                rpsh_format::CommandTableItem::Command(
                    ["Name", "Category", "ModuleName", "Synopsis"]
                        .map(|p| super::format::text(v, p)),
                )
            } else {
                rpsh_format::CommandTableItem::Text(v.to_string())
            }
        })
        .collect();
    Some(rpsh_format::help_table(&rows, width))
}
