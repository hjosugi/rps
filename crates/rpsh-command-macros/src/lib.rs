//! Compile-time parameter declarations for rpsh cmdlets. Only the emitted
//! static data is linked into the shell.
#![forbid(unsafe_code)]
use proc_macro::{Delimiter, Group, TokenStream, TokenTree};
use std::collections::BTreeSet;

#[proc_macro_derive(CmdletParameters, attributes(parameter))]
pub fn derive(input: TokenStream) -> TokenStream {
    expand(input).unwrap_or_else(|message| {
        format!("compile_error!({message:?});")
            .parse()
            .expect("quoted diagnostic")
    })
}

fn expand(input: TokenStream) -> Result<TokenStream, String> {
    let mut tokens = input.into_iter();
    let mut parameters = Vec::new();
    let mut names = BTreeSet::new();
    let name = loop {
        match tokens.next() {
            Some(TokenTree::Punct(hash)) if hash.as_char() == '#' => {
                if let Some(TokenTree::Group(attr)) = tokens.next()
                    && attr.delimiter() == Delimiter::Bracket
                {
                    let mut attr = attr.stream().into_iter();
                    if matches!(attr.next(), Some(TokenTree::Ident(key)) if key.to_string() == "parameter")
                    {
                        let group = arguments(attr.next(), "parameter")?;
                        if attr.next().is_some() {
                            return Err("unexpected tokens after #[parameter(...)]".into());
                        }
                        let (name, declaration) = parameter(group)?;
                        if !names.insert(name.to_ascii_lowercase()) {
                            return Err(format!("duplicate cmdlet parameter {name}"));
                        }
                        parameters.push(declaration);
                    }
                }
            }
            Some(TokenTree::Ident(keyword)) if keyword.to_string() == "struct" => {
                let name = ident(tokens.next(), "cmdlet struct name")?;
                if matches!(tokens.next(), Some(TokenTree::Punct(p)) if p.as_char() == '<') {
                    return Err("derive CmdletParameters on a concrete cmdlet struct".into());
                }
                break name;
            }
            Some(_) => (),
            None => return Err("CmdletParameters requires a struct".into()),
        }
    };
    let body = parameters.join(",");
    format!(
        "impl ::rpsh_commands::signature::CmdletParameters for {name} {{
            const PARAMETERS: &'static [::rpsh_commands::signature::Parameter] = &[{body}];
        }}"
    )
    .parse()
    .map_err(|_| "could not expand cmdlet parameter declarations".into())
}

fn ident(token: Option<TokenTree>, expected: &str) -> Result<String, String> {
    match token {
        Some(TokenTree::Ident(id)) => Ok(id.to_string()),
        _ => Err(format!("expected {expected} in cmdlet declaration")),
    }
}

fn arguments(token: Option<TokenTree>, name: &str) -> Result<Group, String> {
    match token {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => Ok(group),
        _ => Err(format!("expected parenthesized arguments for {name}")),
    }
}

fn parameter(group: Group) -> Result<(String, String), String> {
    let mut tokens = group.stream().into_iter().peekable();
    let name = ident(tokens.next(), "parameter name")?;
    if !matches!(tokens.next(), Some(TokenTree::Punct(p)) if p.as_char() == ':') {
        return Err(format!("expected a colon and parameter type after {name}"));
    }
    let ty = ident(tokens.next(), "ParameterType variant")?;
    let mut declaration = format!(
        "::rpsh_commands::signature::Parameter::new({name:?}, ::rpsh_commands::signature::ParameterType::{ty})"
    );
    let mut seen = BTreeSet::new();
    while let Some(separator) = tokens.next() {
        if !matches!(separator, TokenTree::Punct(p) if p.as_char() == ',') {
            return Err(format!(
                "expected a comma after {name}'s parameter type or option"
            ));
        }
        if tokens.peek().is_none() {
            break;
        }
        let option = ident(tokens.next(), "parameter option")?;
        if !seen.insert(option.clone()) {
            return Err(format!("duplicate option {option} on {name}"));
        }
        match option.as_str() {
            "mandatory" | "pipeline" | "remaining" | "allow_null" | "allow_empty_collection" => {
                declaration.push_str(&format!(".{option}()"))
            }
            "position" => {
                let pos = arguments(tokens.next(), "position")?.stream().to_string();
                let pos = pos
                    .parse::<u16>()
                    .map_err(|_| format!("{name}: position must be an unsigned 16-bit integer"))?;
                declaration.push_str(&format!(".positional({pos})"));
            }
            "sets" | "aliases" => {
                let words = arguments(tokens.next(), &option)?.stream();
                let method = if option == "sets" {
                    "in_sets"
                } else {
                    "aliases"
                };
                declaration.push_str(&format!(".{method}(&[{words}])"));
            }
            _ => return Err(format!("unknown option {option} on {name}")),
        }
    }
    Ok((name, declaration))
}
