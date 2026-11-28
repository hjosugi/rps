//! Delimiters inside command words and expandable strings. Walk the source
//! by byte offset so both the lexer and expression parser see the same boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delimiter {
    Single,
    Double,
    Parentheses,
}

pub(crate) fn quote(ch: char) -> Option<Delimiter> {
    match ch {
        '\'' | '\u{2018}'..='\u{201b}' => Some(Delimiter::Single),
        '"' | '\u{201c}'..='\u{201e}' => Some(Delimiter::Double),
        _ => None,
    }
}

/// Return the byte after the closing delimiter, starting just after its opener.
pub(crate) fn closing_end(text: &str, mut at: usize, initial: Delimiter) -> Option<usize> {
    let mut depth = usize::from(initial == Delimiter::Parentheses);
    while let Some(ch) = text[at..].chars().next() {
        at += ch.len_utf8();
        if depth > 0 {
            // ScanSubExpression counts raw parentheses before parsing the
            // inner text, including parentheses between its quote characters.
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && initial == Delimiter::Parentheses {
                        return Some(at);
                    }
                }
                _ => {}
            }
        } else if quote(ch) == Some(initial) {
            if let Some(next) = text[at..].chars().next()
                && quote(next) == Some(initial)
            {
                at += next.len_utf8();
            } else {
                return Some(at);
            }
        } else if initial == Delimiter::Double {
            if ch == '`' {
                at += text[at..].chars().next().map_or(0, char::len_utf8);
            } else if ch == '$' && text[at..].starts_with('(') {
                at += 1;
                depth = 1;
            }
        }
    }
    None
}

pub(crate) fn single_text(body: &str) -> String {
    let mut content = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if quote(ch) == Some(Delimiter::Single)
            && let Some(escaped) = chars.next()
        {
            content.push(escaped);
        } else {
            content.push(ch);
        }
    }
    content
}

/// ScanSubExpression strips the first half of a doubled or backtick-escaped
/// double quote before the resulting nested token is parsed as source.
pub(crate) fn nested_source(text: &str) -> std::borrow::Cow<'_, str> {
    let mut chars = text.char_indices().peekable();
    let mut previous = 0;
    let mut output = String::new();
    while let Some((at, ch)) = chars.next() {
        if (ch == '`' || quote(ch) == Some(Delimiter::Double))
            && chars
                .peek()
                .is_some_and(|(_, next)| quote(*next) == Some(Delimiter::Double))
        {
            output.push_str(&text[previous..at]);
            previous = at + ch.len_utf8();
            chars.next();
        }
    }
    if previous == 0 {
        text.into()
    } else {
        output.push_str(&text[previous..]);
        output.into()
    }
}
