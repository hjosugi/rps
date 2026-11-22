//! The parser-facing tokenizer: a compatibility view over [`tokenize_full`].
//!
//! The walking-skeleton parser still understands only the original ten token
//! kinds. Instead of keeping a second lexer, this maps the full token stream
//! into that view and turns every construct the parser does not handle into an
//! explicit [`ParseError::NotImplemented`], preserving the feature labels the
//! harness reports. Parser issues replace the mapping as they land.

use crate::full_token_kind::FullTokenKind;
use crate::{ParseError, Span, Token, TokenKind, parse_number_literal, tokenize_full};

/// Splits `source` into the tokens the walking-skeleton parser understands.
/// The last token is always [`TokenKind::EndOfInput`].
pub fn tokenize(source: &str) -> Result<Vec<Token>, ParseError> {
    let full = tokenize_full(source)?;
    let mut tokens = Vec::with_capacity(full.len());
    for token in full {
        let span = token.span;
        let kind = match token.kind {
            FullTokenKind::Number => {
                let text = span.text(source);
                match parse_number_literal(text, span) {
                    Some(Ok(literal)) => literal
                        .is_int32()
                        .map_or(TokenKind::Numeric(literal), TokenKind::Int32),
                    Some(Err(error)) => return Err(error),
                    None => {
                        return Err(ParseError::not_implemented("unrecognized character", span));
                    }
                }
            }
            FullTokenKind::StringLiteral => {
                let text = span.text(source);
                let mut chars = text.char_indices();
                let Some((_, first)) = chars.next() else {
                    return Err(unterminated(span));
                };
                let Some((last_start, last)) = chars.next_back() else {
                    return Err(unterminated(span));
                };
                if matches!(first, '\u{2018}'..='\u{201b}')
                    || matches!(last, '\u{2018}'..='\u{201b}')
                {
                    return Err(ParseError::not_implemented(
                        "typographic quote characters",
                        span,
                    ));
                }
                if first != '\'' || last != '\'' || last_start < first.len_utf8() {
                    return Err(unterminated(span));
                }
                let inner = &text[first.len_utf8()..last_start];
                TokenKind::StringLiteral(inner.replace("''", "'"))
            }
            FullTokenKind::Plus => TokenKind::Plus,
            FullTokenKind::Minus => TokenKind::Minus,
            FullTokenKind::Multiply => TokenKind::Multiply,
            FullTokenKind::LParen => TokenKind::LParen,
            FullTokenKind::RParen => TokenKind::RParen,
            FullTokenKind::Semi => TokenKind::Semi,
            FullTokenKind::Dot => TokenKind::Dot,
            FullTokenKind::Identifier => TokenKind::Identifier(span.text(source).to_owned()),
            FullTokenKind::NewLine => TokenKind::NewLine,
            FullTokenKind::Exit => TokenKind::Exit,
            FullTokenKind::EndOfInput => TokenKind::EndOfInput,
            FullTokenKind::Comment => {
                let text = span.text(source);
                if text.starts_with("<#") {
                    return Err(ParseError::not_implemented("block comments", span));
                }
                if text
                    .get(..9)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#requires"))
                {
                    return Err(ParseError::not_implemented("#requires statements", span));
                }
                continue;
            }
            FullTokenKind::LineContinuation => continue,
            other => return Err(ParseError::not_implemented(feature_for(other), span)),
        };
        tokens.push(Token { kind, span });
    }
    Ok(tokens)
}

fn unterminated(span: Span) -> ParseError {
    ParseError::not_implemented("syntax error reporting (unterminated string)", span)
}

fn feature_for(kind: FullTokenKind) -> &'static str {
    use FullTokenKind as K;
    match kind {
        K::Variable => "variables",
        K::SplattedVariable => "array and hashtable literals, here-strings and splatting",
        K::Parameter => "dash-prefixed operators and parameters (-eq, -not, -Name)",
        K::Generic | K::Identifier | K::Label | K::Unknown => "command invocation",
        K::StringExpandable => "expandable (double-quoted) strings",
        K::HereStringLiteral | K::HereStringExpandable => "here-strings",
        K::LCurly | K::RCurly => "script blocks",
        K::LBracket | K::RBracket => "type literals, casts and indexing",
        K::AtParen | K::AtCurly => "array and hashtable literals, here-strings and splatting",
        K::DollarParen => "subexpressions",
        K::Pipe => "pipelines",
        K::Ampersand | K::AndAnd | K::OrOr => "call operator, background operator and && chains",
        K::Comma => "array construction (comma operator)",
        K::DotDot => "range operator",
        K::Dot | K::ColonColon => "member access, dot-sourcing and real literals",
        K::Exclaim => "logical not",
        K::Divide | K::Rem => "division and remainder operators",
        K::Redirection | K::RedirectInStd => "redirection",
        K::QuestionMark
        | K::QuestionQuestion
        | K::QuestionQuestionEquals
        | K::QuestionDot
        | K::QuestionLBracket => "ternary and null-conditional operators",
        K::Equals
        | K::PlusEquals
        | K::MinusEquals
        | K::MultiplyEquals
        | K::DivideEquals
        | K::RemainderEquals => "assignment",
        K::Ieq
        | K::Ine
        | K::Ige
        | K::Igt
        | K::Ilt
        | K::Ile
        | K::Ilike
        | K::Inotlike
        | K::Imatch
        | K::Inotmatch
        | K::Ireplace
        | K::Icontains
        | K::Inotcontains
        | K::Iin
        | K::Inotin
        | K::Isplit
        | K::Ceq
        | K::Cne
        | K::Cge
        | K::Cgt
        | K::Clt
        | K::Cle
        | K::Clike
        | K::Cnotlike
        | K::Cmatch
        | K::Cnotmatch
        | K::Creplace
        | K::Ccontains
        | K::Cnotcontains
        | K::Cin
        | K::Cnotin
        | K::Csplit
        | K::Is
        | K::IsNot
        | K::As
        | K::Not
        | K::Bnot
        | K::And
        | K::Or
        | K::Xor
        | K::Band
        | K::Bor
        | K::Bxor
        | K::Join
        | K::Format
        | K::Shl
        | K::Shr => "dash-prefixed operators and parameters (-eq, -not, -Name)",
        _ if kind.flags().0 != 0 && kind.name().chars().all(|ch| !ch.is_ascii_lowercase()) => {
            "statement keywords (if, foreach, function, ...)"
        }
        _ => "statement keywords (if, foreach, function, ...)",
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
        assert!(matches!(
            kinds("2147483648")[0],
            TokenKind::Numeric(crate::NumberLiteral::Int64(2147483648))
        ));
        assert_eq!(
            kinds("1kb"),
            vec![TokenKind::Int32(1024), TokenKind::EndOfInput]
        );
        assert!(matches!(
            kinds("1.5")[0],
            TokenKind::Numeric(crate::NumberLiteral::Double(_))
        ));
        assert_eq!(refused("<# c #>"), "block comments");
        assert_eq!(refused("#Requires -Version 7"), "#requires statements");
        assert_eq!(
            refused("'abc"),
            "syntax error reporting (unterminated string)"
        );
        assert_eq!(refused("'a\u{2019}"), "typographic quote characters");
        assert_eq!(
            kinds("\u{2013}1"),
            vec![TokenKind::Minus, TokenKind::Int32(1), TokenKind::EndOfInput]
        );
    }

    #[test]
    fn exit_is_a_keyword_only_as_a_whole_word() {
        assert_eq!(
            crate::parse("exits").unwrap_err().feature(),
            "command invocation"
        );
        assert_eq!(refused("exit.exe"), "command invocation");
    }
}
