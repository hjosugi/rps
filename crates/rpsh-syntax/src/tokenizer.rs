use crate::{ParseError, Span, Token, TokenKind};

const NUMERIC_FORMS: &str = "numeric literals other than decimal Int32";

/// Words that start a statement the skeleton does not parse. Reported apart
/// from command invocation so the two show up separately in harness output.
const STATEMENT_KEYWORDS: &[&str] = &[
    "begin",
    "break",
    "catch",
    "class",
    "clean",
    "continue",
    "data",
    "do",
    "dynamicparam",
    "else",
    "elseif",
    "end",
    "enum",
    "filter",
    "finally",
    "for",
    "foreach",
    "function",
    "if",
    "in",
    "param",
    "process",
    "return",
    "switch",
    "throw",
    "trap",
    "try",
    "until",
    "using",
    "while",
];

/// Splits `source` into tokens. The last token is always
/// [`TokenKind::EndOfInput`].
pub fn tokenize(source: &str) -> Result<Vec<Token>, ParseError> {
    if source.len() > u32::MAX as usize {
        return Err(ParseError::not_implemented(
            "scripts larger than 4 GiB",
            Span::default(),
        ));
    }
    Tokenizer {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
    }
    .run()
}

struct Tokenizer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
}

impl Tokenizer<'_> {
    fn run(mut self) -> Result<Vec<Token>, ParseError> {
        while let Some(&byte) = self.bytes.get(self.pos) {
            match byte {
                b' ' | b'\t' => self.pos += 1,
                b'\r' => {
                    let len = if self.peek(1) == Some(b'\n') { 2 } else { 1 };
                    self.push(TokenKind::NewLine, len);
                }
                b'\n' => self.push(TokenKind::NewLine, 1),
                b';' => self.push(TokenKind::Semi, 1),
                b'(' => self.push(TokenKind::LParen, 1),
                b')' => self.push(TokenKind::RParen, 1),
                b'+' | b'-' | b'*' => self.operator(byte)?,
                b'#' => self.comment()?,
                b'\'' => self.single_quoted_string()?,
                b'0'..=b'9' => self.number()?,
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.bareword()?,
                _ => return Err(self.unsupported_char(self.pos)),
            }
        }
        let end = self.bytes.len();
        self.tokens.push(Token {
            kind: TokenKind::EndOfInput,
            span: Span::new(end, end),
        });
        Ok(self.tokens)
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.pos + ahead).copied()
    }

    fn push(&mut self, kind: TokenKind, len: usize) {
        let span = Span::new(self.pos, self.pos + len);
        self.tokens.push(Token { kind, span });
        self.pos += len;
    }

    fn error(&self, feature: &'static str, start: usize, len: usize) -> ParseError {
        ParseError::not_implemented(feature, Span::new(start, start + len))
    }

    fn operator(&mut self, byte: u8) -> Result<(), ParseError> {
        let next = self.peek(1);
        if next == Some(b'=') {
            return Err(self.error("compound assignment operators", self.pos, 2));
        }
        let kind = match byte {
            b'+' if next == Some(b'+') => {
                return Err(self.error("increment and decrement operators", self.pos, 2));
            }
            b'-' if next == Some(b'-') => {
                return Err(self.error("increment and decrement operators", self.pos, 2));
            }
            b'-' if next.is_some_and(|b| b.is_ascii_alphabetic()) => {
                return Err(self.error(
                    "dash-prefixed operators and parameters (-eq, -not, -Name)",
                    self.pos,
                    2,
                ));
            }
            b'+' => TokenKind::Plus,
            b'-' => TokenKind::Minus,
            _ => TokenKind::Multiply,
        };
        self.push(kind, 1);
        Ok(())
    }

    /// A `#` comment running to the end of the line. The line ending itself is
    /// left for the main loop.
    fn comment(&mut self) -> Result<(), ParseError> {
        let start = self.pos;
        let at_token_start = match start.checked_sub(1).map(|i| self.bytes[i]) {
            None => true,
            Some(prev) => matches!(prev, b' ' | b'\t' | b'\r' | b'\n' | b';' | b'('),
        };
        if !at_token_start {
            return Err(self.error("'#' directly after a token", start, 1));
        }
        let end = self.bytes[start..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
            .map_or(self.bytes.len(), |offset| start + offset);
        let is_requires = self.bytes[start..end]
            .get(..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"#requires"));
        if is_requires {
            return Err(self.error("#requires statements", start, 9));
        }
        self.pos = end;
        Ok(())
    }

    fn single_quoted_string(&mut self) -> Result<(), ParseError> {
        let start = self.pos;
        let mut value = String::new();
        let mut at = start + 1;
        loop {
            let Some(ch) = self.source[at..].chars().next() else {
                return Err(self.error(
                    "syntax error reporting (unterminated string)",
                    start,
                    self.bytes.len() - start,
                ));
            };
            let ch_start = at;
            at += ch.len_utf8();
            match ch {
                '\'' if self.bytes.get(at) == Some(&b'\'') => {
                    value.push('\'');
                    at += 1;
                }
                '\'' => break,
                // Upstream treats these as quote characters too, so they
                // would end the string here.
                '\u{2018}'..='\u{201B}' => {
                    return Err(self.error(
                        "typographic quote characters",
                        ch_start,
                        ch.len_utf8(),
                    ));
                }
                '\r' | '\n' => {
                    return Err(self.error("multi-line string literals", ch_start, 1));
                }
                _ => value.push(ch),
            }
        }
        self.tokens.push(Token {
            kind: TokenKind::StringLiteral(value),
            span: Span::new(start, at),
        });
        self.pos = at;
        self.expect_literal_end()
    }

    fn number(&mut self) -> Result<(), ParseError> {
        let start = self.pos;
        let len = self.bytes[start..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let end = start + len;
        match self.bytes.get(end) {
            Some(b'.') if self.bytes.get(end + 1) == Some(&b'.') => {
                return Err(self.error("range operator", end, 2));
            }
            Some(b) if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_') => {
                return Err(self.error(NUMERIC_FORMS, start, len + 1));
            }
            _ => {}
        }
        let Ok(value) = self.source[start..end].parse::<i32>() else {
            return Err(self.error(NUMERIC_FORMS, start, len));
        };
        self.push(TokenKind::Int32(value), len);
        self.expect_literal_end()
    }

    /// A literal has to be followed by whitespace, a separator or one of the
    /// implemented operators. `'a'.Length`, `'a'[0]` and the like are refused
    /// here, with the character that follows naming the feature.
    fn expect_literal_end(&self) -> Result<(), ParseError> {
        match self.bytes.get(self.pos) {
            None | Some(b' ' | b'\t' | b'\r' | b'\n' | b';' | b')' | b'+' | b'-' | b'*' | b'#') => {
                Ok(())
            }
            Some(b) if b.is_ascii_alphanumeric() || *b == b'_' => {
                Err(self.error("literals directly followed by another token", self.pos, 1))
            }
            Some(_) => Err(self.unsupported_char(self.pos)),
        }
    }

    fn bareword(&mut self) -> Result<(), ParseError> {
        let start = self.pos;
        let len = self.bytes[start..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
            .count();
        let word = &self.source[start..start + len];
        let ends_token = matches!(
            self.bytes.get(start + len),
            None | Some(b' ' | b'\t' | b'\r' | b'\n' | b';' | b'(' | b')')
        );
        if ends_token && word.eq_ignore_ascii_case("exit") {
            self.push(TokenKind::Exit, len);
            return Ok(());
        }
        let is_keyword = ends_token
            && STATEMENT_KEYWORDS
                .iter()
                .any(|keyword| word.eq_ignore_ascii_case(keyword));
        let feature = if is_keyword {
            "statement keywords (if, foreach, function, ...)"
        } else {
            "command invocation"
        };
        Err(self.error(feature, start, len))
    }

    fn unsupported_char(&self, at: usize) -> ParseError {
        let ch = self.source[at..].chars().next().unwrap_or('\0');
        let feature = match ch {
            '$' => "variables",
            '"' => "expandable (double-quoted) strings",
            '@' => "array and hashtable literals, here-strings and splatting",
            '{' | '}' => "script blocks",
            '[' | ']' => "type literals, casts and indexing",
            '|' => "pipelines",
            '&' => "call operator, background operator and && chains",
            '=' => "assignment",
            '<' if self.bytes.get(at + 1) == Some(&b'#') => "block comments",
            '<' | '>' => "redirection",
            '/' | '%' => "division and remainder operators",
            ',' => "array construction (comma operator)",
            '.' => "member access, dot-sourcing and real literals",
            '!' => "logical not",
            '`' => "backtick escapes and line continuation",
            '?' | ':' => "ternary and null-conditional operators",
            _ if !ch.is_ascii() => "non-ASCII characters outside string literals",
            _ => "unrecognized character",
        };
        self.error(feature, at, ch.len_utf8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source)
            .unwrap()
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    fn refused(source: &str) -> &'static str {
        tokenize(source).unwrap_err().feature()
    }

    #[test]
    fn tokenizes_the_supported_subset() {
        assert_eq!(
            kinds("1 + (2*3) - 4; 'it''s'\r\nEXIT"),
            vec![
                TokenKind::Int32(1),
                TokenKind::Plus,
                TokenKind::LParen,
                TokenKind::Int32(2),
                TokenKind::Multiply,
                TokenKind::Int32(3),
                TokenKind::RParen,
                TokenKind::Minus,
                TokenKind::Int32(4),
                TokenKind::Semi,
                TokenKind::StringLiteral("it's".to_owned()),
                TokenKind::NewLine,
                TokenKind::Exit,
                TokenKind::EndOfInput,
            ]
        );
    }

    #[test]
    fn skips_comments_but_keeps_the_line_ending() {
        assert_eq!(
            kinds("# one\n1 # two"),
            vec![
                TokenKind::NewLine,
                TokenKind::Int32(1),
                TokenKind::EndOfInput
            ]
        );
    }

    #[test]
    fn spans_cover_the_token_text() {
        let source = "12 + 'ab'";
        let tokens = tokenize(source).unwrap();
        let texts: Vec<&str> = tokens.iter().map(|t| t.span.text(source)).collect();
        assert_eq!(texts, ["12", "+", "'ab'", ""]);
    }

    #[test]
    fn names_the_missing_feature() {
        assert_eq!(refused("$x"), "variables");
        assert_eq!(refused("\"a\""), "expandable (double-quoted) strings");
        assert_eq!(refused("1 | x"), "pipelines");
        assert_eq!(refused("Get-Date"), "command invocation");
        assert_eq!(
            refused("if (1) { 2 }"),
            "statement keywords (if, foreach, function, ...)"
        );
        assert_eq!(
            refused("1 -eq 1"),
            "dash-prefixed operators and parameters (-eq, -not, -Name)"
        );
        assert_eq!(refused("1..3"), "range operator");
        assert_eq!(refused("2147483648"), NUMERIC_FORMS);
        assert_eq!(refused("1kb"), NUMERIC_FORMS);
        assert_eq!(refused("1.5"), NUMERIC_FORMS);
        assert_eq!(
            refused("'a'.Length"),
            "member access, dot-sourcing and real literals"
        );
        assert_eq!(refused("<# c #>"), "block comments");
        assert_eq!(refused("#Requires -Version 7"), "#requires statements");
        assert_eq!(
            refused("'abc"),
            "syntax error reporting (unterminated string)"
        );
        assert_eq!(refused("'a\u{2019}"), "typographic quote characters");
        assert_eq!(
            refused("\u{2013}1"),
            "non-ASCII characters outside string literals"
        );
    }

    #[test]
    fn exit_is_a_keyword_only_as_a_whole_word() {
        assert_eq!(refused("exits"), "command invocation");
        assert_eq!(refused("exit.exe"), "command invocation");
    }
}
