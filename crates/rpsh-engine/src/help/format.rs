//! Console and Out-String views for synthesized help. Each indented PowerShell
//! formatting field wraps independently, including existing spaces and newlines.
use super::{EXTENDED, Value};

fn field<'a>(o: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Object(fields) = o else {
        return None;
    };
    fields
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn text(o: &Value, name: &str) -> String {
    field(o, name).map(ToString::to_string).unwrap_or_default()
}

pub(super) fn root(o: &Value) -> bool {
    field(o, "__HelpView").is_some()
}
fn parameter(o: &Value) -> bool {
    field(o, "__PSTypeNames")
        .and_then(Value::sequence)
        .is_some_and(|v| {
            v.first()
                .is_some_and(|s| s.to_string() == format!("{EXTENDED}#parameter"))
        })
}

pub(in crate::help) fn comments(o: &Value) -> Option<String> {
    root(o).then(|| format!("\n.SYNOPSIS\n\n{}\n", text(o, "Synopsis")))
}

pub(crate) fn supported(o: &Value) -> bool {
    field(o, "__Format").is_none() && (root(o) || parameter(o))
}

struct Output {
    buf: String,
    width: usize,
    words: bool,
}
impl Output {
    fn field(&mut self, indent: usize, value: &str) {
        rpsh_format::write_field(&mut self.buf, indent, self.width, self.words, value);
    }
    fn parameter(&mut self, p: &Value, indent: usize, details: bool) {
        let mut name = format!("-{}", text(p, "name"));
        if let Some(ty) = field(p, "parameterValue") {
            name.push_str(&format!(" <{ty}>"));
        }
        self.field(indent, &name);
        if details {
            let base = indent + 4;
            self.field(base, "");
            for (label, prop) in [
                ("ParameterRequired", "required"),
                ("ParameterPosition", "position"),
                ("AcceptsPipelineInput", "pipelineInput"),
                ("ParameterSetName", "parameterSetName"),
                ("ParameterAliases", "aliases"),
                ("ParameterIsDynamic", "isDynamic"),
                ("AcceptsWildCardCharacters", "globbing"),
            ] {
                let label = rpsh_messages::text(&format!("HelpDisplayStrings/{label}"));
                self.field(base, &format!("{label}{}", text(p, prop)));
            }
            self.field(base, "");
        } else {
            self.field(indent, "");
        }
    }
    fn root(&mut self, h: &Value) {
        let view = text(h, "__HelpView");
        let full = view == "Full";
        self.field(0, "NAME");
        self.field(4, &text(h, "Name"));
        if view != "Examples" {
            self.field(4, "");
            self.field(0, "SYNTAX");
            if let Some(items) = field(h, "Syntax")
                .and_then(|s| field(s, "syntaxItem"))
                .and_then(Value::sequence)
            {
                for item in items.iter() {
                    let mut syntax = text(item, "name");
                    if let Some(params) = field(item, "parameter").and_then(Value::sequence) {
                        for p in params.iter() {
                            syntax.push(' ');
                            syntax.push_str(&syntax_parameter(p));
                        }
                    }
                    syntax.push_str(" [<CommonParameters>]");
                    self.field(4, &syntax);
                    self.field(4, "");
                }
            }
        }
        if full || view == "Detailed" {
            self.field(4, "");
            self.field(0, "PARAMETERS");
            if let Some(params) = field(h, "parameters")
                .and_then(|s| field(s, "parameter"))
                .and_then(Value::sequence)
            {
                for p in params.iter() {
                    self.parameter(p, 4, full);
                }
            }
            self.field(4, "<CommonParameters>");
            self.field(
                4,
                rpsh_messages::text("HelpDisplayStrings/BaseCmdletInformation"),
            );
            self.field(4, "");
        }
        if full {
            self.field(4, "");
            self.field(0, "INPUTS");
            self.field(4, &type_text(h, "inputTypes", "inputType"));
            self.field(4, "");
            self.field(0, "OUTPUTS");
            self.field(4, &type_text(h, "returnValues", "returnValue"));
            self.field(4, "");
        } else {
            self.field(0, "");
        }
        self.field(0, "ALIASES");
        self.field(4, &text(h, "aliases"));
        self.field(0, "");
        self.field(0, "REMARKS");
        self.field(4, &text(h, "remarks"));
    }
}

fn type_text(h: &Value, collection: &str, item: &str) -> String {
    field(h, collection)
        .and_then(|x| field(x, item))
        .and_then(|x| field(x, "type"))
        .map(|x| text(x, "name"))
        .unwrap_or_default()
}

fn syntax_parameter(p: &Value) -> String {
    let optional = text(p, "required") == "false";
    let mut name = format!("-{}", text(p, "name"));
    if text(p, "position") != "Named" {
        name = format!("[{name}]");
    }
    if let Some(group) = field(p, "parameterValueGroup")
        .and_then(|g| field(g, "parameterValue"))
        .and_then(Value::sequence)
    {
        let group: Vec<_> = group.iter().map(ToString::to_string).collect();
        name.push_str(&format!(" {{{}}}", group.join(" | ")));
    } else if let Some(ty) = field(p, "parameterValue") {
        name.push_str(&format!(" <{ty}>"));
    }
    if optional { format!("[{name}]") } else { name }
}

pub(crate) fn group(items: &[Value], width: usize) -> Option<String> {
    if items.is_empty() || !items.iter().all(supported) {
        return None;
    }
    let mut out = Output {
        buf: "\n".into(),
        width,
        words: rpsh_format::wraps_words(&crate::globalization::current_name()),
    };
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.buf.push('\n');
        }
        if root(item) {
            out.root(item);
        } else {
            out.parameter(item, 0, true);
        }
    }
    out.buf.push_str("\n\n");
    Some(out.buf)
}
