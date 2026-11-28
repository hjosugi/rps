//! NativeCommandParameterBinder supplies either an argv vector or a legacy
//! argument string. In the latter case .NET parses the complete string once,
//! so an unmatched quote can span PowerShell argument boundaries.
use super::{Engine, EngineError, glob};
use crate::preferences::{self, ArgumentStyle};
use crate::{environment, variables::Variables};
use rpsh_runtime::Value;
use rpsh_syntax::{ArgumentSeparator, CommandArgument, Expr, ExprKind};
use std::path::Path;

enum Arguments {
    List(Vec<String>),
    Legacy(String),
}

struct Parameter {
    text: String,
    separator: ArgumentSeparator,
}

impl Arguments {
    fn new(style: ArgumentStyle, path: &Path) -> Self {
        let special = || {
            let named = |word: Option<&std::ffi::OsStr>, names: &[&str]| {
                word.is_some_and(|word| names.iter().any(|name| word.eq_ignore_ascii_case(name)))
            };
            named(path.extension(), &["js", "wsf", "cmd", "bat", "vbs"])
                || named(
                    path.file_stem(),
                    &["cmd", "cscript", "find", "sqlcmd", "wscript"],
                )
        };
        if style == ArgumentStyle::Legacy || style == ArgumentStyle::Windows && special() {
            Self::Legacy(String::new())
        } else {
            Self::List(Vec::new())
        }
    }

    fn begin(&mut self, parameter: Option<&Parameter>) {
        match self {
            Self::List(out) => {
                if let Some(p) = parameter
                    && p.separator == ArgumentSeparator::Space
                {
                    out.push(p.text.clone());
                }
            }
            Self::Legacy(line) => {
                if !line.is_empty() {
                    line.push(' ');
                }
                if let Some(p) = parameter {
                    line.push_str(&p.text);
                    if p.separator != ArgumentSeparator::Colon {
                        line.push(' ');
                    }
                }
            }
        }
    }

    fn word(
        &mut self,
        word: String,
        quoted: bool,
        p: Option<&Parameter>,
        separator: &str,
        verbatim: Option<&Variables>,
    ) {
        if let Some(variables) = verbatim
            && !word.is_empty()
        {
            let word = expand_environment(&word, variables);
            match self {
                Self::List(argv) => {
                    argv.extend(word.split(' ').filter(|s| !s.is_empty()).map(str::to_owned))
                }
                Self::Legacy(line) => {
                    line.push_str(separator);
                    line.push_str(&word);
                }
            }
            return;
        }
        match self {
            Self::List(out) => glob::expand(word, quoted, |word, _| {
                if let Some(p) = p
                    && p.separator != ArgumentSeparator::Space
                {
                    if word == p.text {
                        return;
                    }
                    if p.separator == ArgumentSeparator::Colon {
                        out.push(format!("{}{word}", p.text));
                        return;
                    }
                    out.push(p.text.clone());
                }
                out.push(word);
            }),
            Self::Legacy(line) if !word.is_empty() => {
                line.push_str(separator);
                let wrap = glob::needs_quotes(&word);
                let mut first = true;
                glob::expand(word, quoted, |word, globbed| {
                    if !first {
                        line.push(' ');
                    }
                    first = false;
                    let quotes = wrap || globbed && glob::needs_quotes(&word);
                    if quotes {
                        line.push('"');
                    }
                    line.push_str(&word);
                    if wrap {
                        let backslashes = word
                            .as_bytes()
                            .iter()
                            .rev()
                            .take_while(|b| **b == b'\\')
                            .count();
                        line.extend(std::iter::repeat_n('\\', backslashes));
                    }
                    if quotes {
                        line.push('"');
                    }
                });
            }
            Self::Legacy(_) => {}
        }
    }

    fn finish(self) -> Vec<String> {
        match self {
            Self::List(list) => list,
            Self::Legacy(line) => split_legacy(&line),
        }
    }
}

impl Engine {
    pub(super) fn native_arguments(
        &mut self,
        args: &[CommandArgument],
        path: &Path,
    ) -> Result<Vec<String>, EngineError> {
        let mut output = Arguments::new(preferences::argument_style(&self.variables), path);
        let mut verbatim = false;
        for arg in args {
            let (p, value) = match arg {
                CommandArgument::Bareword(word) => {
                    output.begin(None);
                    if word == "--%" {
                        verbatim = true;
                    } else {
                        output.word(
                            word.clone(),
                            false,
                            None,
                            "",
                            verbatim.then_some(&self.variables),
                        );
                    }
                    continue;
                }
                CommandArgument::Value(expr) => (None, Some(expr)),
                CommandArgument::Parameter {
                    name,
                    value,
                    separator,
                } => {
                    let colon = if *separator == ArgumentSeparator::Space {
                        ""
                    } else {
                        ":"
                    };
                    (
                        Some(Parameter {
                            text: format!("-{name}{colon}"),
                            separator: *separator,
                        }),
                        value.as_ref(),
                    )
                }
            };
            output.begin(p.as_ref());
            if let Some(expr) = value {
                self.native_argument_value(expr, p.as_ref(), &mut output, &mut verbatim)?;
            }
        }
        Ok(output.finish())
    }

    fn native_argument_value(
        &mut self,
        expr: &Expr,
        p: Option<&Parameter>,
        out: &mut Arguments,
        verbatim: &mut bool,
    ) -> Result<(), EngineError> {
        let quoted = matches!(
            expr.kind,
            ExprKind::String(_) | ExprKind::ExpandableString(_)
        );
        let value = self.eval_expression(expr)?;
        if matches!(&value, Value::String(s) if s.as_ascii() == Some("--%")) {
            *verbatim = true;
            return Ok(());
        }
        let sequence = value.sequence();
        let items = sequence
            .as_deref()
            .unwrap_or_else(|| std::slice::from_ref(&value));
        for (index, item) in items.iter().enumerate() {
            if !item.is_null_like() {
                out.word(
                    item.to_string(),
                    quoted,
                    p,
                    if index == 0 { "" } else { " " },
                    verbatim.then_some(&self.variables),
                );
            }
        }
        Ok(())
    }
}

// On an unknown name the closing percent can open the next reference.
// Values substituted from the environment are not expanded a second time.
fn expand_environment(mut text: &str, variables: &Variables) -> String {
    let mut result = String::with_capacity(text.len());
    while let Some((prefix, tail)) = text.split_once('%') {
        result.push_str(prefix);
        if let Some((name, rest)) = tail.split_once('%')
            && !name.is_empty()
            && !name.contains(['=', '\0'])
            && let Some(value) = environment::get(variables, name)
        {
            result.push_str(&value.to_string());
            text = rest;
        } else {
            result.push('%');
            text = tail;
        }
    }
    result.push_str(text);
    result
}

// ProcessUtils.ParseArgumentsIntoList on Unix splits on ASCII space and tab.
// Backslashes only change meaning before a double quote.
fn split_legacy(line: &str) -> Vec<String> {
    let mut chars = line.chars().peekable();
    let mut args = Vec::new();
    loop {
        while chars.next_if(|ch| matches!(ch, ' ' | '\t')).is_some() {}
        if chars.peek().is_none() {
            break;
        }
        let mut word = String::new();
        let mut quoted = false;
        while let Some(ch) = chars.next() {
            match ch {
                ' ' | '\t' if !quoted => break,
                '"' if quoted && chars.next_if_eq(&'"').is_some() => word.push('"'),
                '"' => quoted = !quoted,
                '\\' => {
                    let mut slashes = 1usize;
                    while chars.next_if_eq(&'\\').is_some() {
                        slashes += 1;
                    }
                    let quote = chars.peek() == Some(&'"');
                    word.extend(std::iter::repeat_n(
                        '\\',
                        if quote { slashes / 2 } else { slashes },
                    ));
                    if quote && !slashes.is_multiple_of(2) {
                        chars.next();
                        word.push('"');
                    }
                }
                ch => word.push(ch),
            }
        }
        args.push(word);
    }
    args
}
