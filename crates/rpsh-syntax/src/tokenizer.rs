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
                let single = matches!(first, '\'' | '\u{2018}'..='\u{201b}');
                let closes = matches!(last, '\'' | '\u{2018}'..='\u{201b}');
                if !single || !closes || last_start < first.len_utf8() {
                    return Err(unterminated(span));
                }
                let inner = &text[first.len_utf8()..last_start];
                TokenKind::StringLiteral(inner.replace("''", "'"))
            }
            FullTokenKind::Plus => TokenKind::Plus,
            FullTokenKind::Minus => TokenKind::Minus,
            FullTokenKind::Multiply => TokenKind::Multiply,
            FullTokenKind::Divide => TokenKind::Divide,
            FullTokenKind::Rem => TokenKind::Remainder,
            FullTokenKind::LParen => TokenKind::LParen,
            FullTokenKind::RParen => TokenKind::RParen,
            FullTokenKind::Semi => TokenKind::Semi,
            FullTokenKind::Dot => TokenKind::Dot,
            FullTokenKind::Equals => TokenKind::Equals,
            FullTokenKind::LBracket => TokenKind::LBracket,
            FullTokenKind::RBracket => TokenKind::RBracket,
            FullTokenKind::LCurly => TokenKind::LCurly,
            FullTokenKind::AtParen => TokenKind::AtParen,
            FullTokenKind::AtCurly => TokenKind::AtCurly,
            FullTokenKind::Comma => TokenKind::Comma,
            FullTokenKind::DotDot => TokenKind::DotDot,
            FullTokenKind::PlusEquals => TokenKind::PlusEquals,
            FullTokenKind::MinusEquals => TokenKind::MinusEquals,
            FullTokenKind::MultiplyEquals => TokenKind::MultiplyEquals,
            FullTokenKind::DivideEquals => TokenKind::DivideEquals,
            FullTokenKind::RemainderEquals => TokenKind::RemainderEquals,
            FullTokenKind::Join => TokenKind::Join,
            FullTokenKind::Imatch => TokenKind::Match,
            FullTokenKind::Inotmatch => TokenKind::NotMatch,
            FullTokenKind::Cmatch => TokenKind::CMatch,
            FullTokenKind::Cnotmatch => TokenKind::CNotMatch,
            FullTokenKind::Ireplace => TokenKind::Replace,
            FullTokenKind::Creplace => TokenKind::CReplace,
            FullTokenKind::Isplit => TokenKind::Split,
            FullTokenKind::Csplit => TokenKind::CSplit,
            FullTokenKind::Ieq => TokenKind::Eq,
            FullTokenKind::Ine => TokenKind::Ne,
            FullTokenKind::Ige => TokenKind::Ge,
            FullTokenKind::Igt => TokenKind::Gt,
            FullTokenKind::Ilt => TokenKind::Lt,
            FullTokenKind::Ile => TokenKind::Le,
            FullTokenKind::Ceq => TokenKind::CEq,
            FullTokenKind::Cne => TokenKind::CNe,
            FullTokenKind::Cge => TokenKind::CGe,
            FullTokenKind::Cgt => TokenKind::CGt,
            FullTokenKind::Clt => TokenKind::CLt,
            FullTokenKind::Cle => TokenKind::CLe,
            FullTokenKind::Ilike => TokenKind::Like,
            FullTokenKind::Inotlike => TokenKind::NotLike,
            FullTokenKind::Clike => TokenKind::CLike,
            FullTokenKind::Cnotlike => TokenKind::CNotLike,
            FullTokenKind::Icontains => TokenKind::Contains,
            FullTokenKind::Inotcontains => TokenKind::NotContains,
            FullTokenKind::Ccontains => TokenKind::CContains,
            FullTokenKind::Cnotcontains => TokenKind::CNotContains,
            FullTokenKind::Iin => TokenKind::In,
            FullTokenKind::Inotin => TokenKind::NotIn,
            FullTokenKind::Cin => TokenKind::CIn,
            FullTokenKind::Cnotin => TokenKind::CNotIn,
            FullTokenKind::Shl => TokenKind::Shl,
            FullTokenKind::Shr => TokenKind::Shr,
            FullTokenKind::Band => TokenKind::Band,
            FullTokenKind::Bor => TokenKind::Bor,
            FullTokenKind::Bxor => TokenKind::Bxor,
            FullTokenKind::And => TokenKind::And,
            FullTokenKind::Or => TokenKind::Or,
            FullTokenKind::Xor => TokenKind::Xor,
            FullTokenKind::Not => TokenKind::Not,
            FullTokenKind::Bnot => TokenKind::Bnot,
            FullTokenKind::Exclaim => TokenKind::Exclaim,
            FullTokenKind::Is => TokenKind::Is,
            FullTokenKind::IsNot => TokenKind::IsNot,
            FullTokenKind::As => TokenKind::As,
            FullTokenKind::PlusPlus | FullTokenKind::PostfixPlusPlus => TokenKind::PlusPlus,
            FullTokenKind::MinusMinus | FullTokenKind::PostfixMinusMinus => TokenKind::MinusMinus,
            FullTokenKind::QuestionMark => TokenKind::QuestionMark,
            FullTokenKind::Colon => TokenKind::Colon,
            FullTokenKind::QuestionQuestion => TokenKind::Coalesce,
            FullTokenKind::QuestionQuestionEquals => TokenKind::CoalesceEquals,
            FullTokenKind::Label => {
                let text = span.text(source);
                TokenKind::Label(text.trim_start_matches(':').to_owned())
            }
            FullTokenKind::RCurly => TokenKind::RCurly,
            FullTokenKind::DollarParen => TokenKind::DollarParen,
            FullTokenKind::Variable => {
                let text = span.text(source);
                let name = text
                    .strip_prefix("${")
                    .and_then(|rest| rest.strip_suffix('}'))
                    .or_else(|| text.strip_prefix('$'))
                    .unwrap_or(text);
                if name.ends_with(':') {
                    return Err(ParseError::not_implemented(
                        "variable reference is not valid ('$x:')",
                        span,
                    ));
                }
                TokenKind::Variable(name.to_owned())
            }
            FullTokenKind::StringExpandable => {
                TokenKind::ExpandableString(span.text(source).to_owned())
            }
            FullTokenKind::HereStringLiteral => TokenKind::HereString {
                text: span.text(source).to_owned(),
                expandable: false,
            },
            FullTokenKind::HereStringExpandable => TokenKind::HereString {
                text: span.text(source).to_owned(),
                expandable: true,
            },
            FullTokenKind::Identifier => TokenKind::Identifier(span.text(source).to_owned()),
            FullTokenKind::Generic => TokenKind::Bareword(span.text(source).to_owned()),
            FullTokenKind::Parameter => {
                TokenKind::Parameter(span.text(source).trim_start_matches('-').to_owned())
            }
            FullTokenKind::NewLine => TokenKind::NewLine,
            FullTokenKind::Exit => TokenKind::Exit,
            FullTokenKind::If => TokenKind::If,
            FullTokenKind::Function => TokenKind::Function,
            FullTokenKind::Filter => TokenKind::Filter,
            FullTokenKind::Param => TokenKind::Param,
            FullTokenKind::ElseIf => TokenKind::ElseIf,
            FullTokenKind::Else => TokenKind::Else,
            FullTokenKind::For => TokenKind::For,
            FullTokenKind::Foreach => TokenKind::Foreach,
            FullTokenKind::In => TokenKind::In,
            FullTokenKind::While => TokenKind::While,
            FullTokenKind::Do => TokenKind::Do,
            FullTokenKind::Until => TokenKind::Until,
            FullTokenKind::Try => TokenKind::Try,
            FullTokenKind::Catch => TokenKind::Catch,
            FullTokenKind::Finally => TokenKind::Finally,
            FullTokenKind::Throw => TokenKind::Throw,
            FullTokenKind::Return => TokenKind::Return,
            FullTokenKind::Break => TokenKind::Break,
            FullTokenKind::Continue => TokenKind::Continue,
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
        assert_eq!(
            kinds("$x"),
            vec![TokenKind::Variable("x".to_owned()), TokenKind::EndOfInput]
        );
        assert_eq!(
            kinds("\"a\""),
            vec![
                TokenKind::ExpandableString("\"a\"".to_owned()),
                TokenKind::EndOfInput
            ]
        );
        assert_eq!(refused("1 | x"), "pipelines");
        assert_eq!(
            kinds("Get-Date"),
            vec![
                TokenKind::Bareword("Get-Date".to_owned()),
                TokenKind::EndOfInput
            ]
        );
        assert_eq!(
            refused("switch (1) { 1 { 2 } }"),
            "statement keywords (if, foreach, function, ...)"
        );
        assert_eq!(
            kinds(
                "if\nwhile\nfor\nforeach\ndo\ntry\ncatch\nfinally\nthrow\nreturn\nbreak\ncontinue"
            ),
            vec![
                TokenKind::If,
                TokenKind::NewLine,
                TokenKind::While,
                TokenKind::NewLine,
                TokenKind::For,
                TokenKind::NewLine,
                TokenKind::Foreach,
                TokenKind::NewLine,
                TokenKind::Do,
                TokenKind::NewLine,
                TokenKind::Try,
                TokenKind::NewLine,
                TokenKind::Catch,
                TokenKind::NewLine,
                TokenKind::Finally,
                TokenKind::NewLine,
                TokenKind::Throw,
                TokenKind::NewLine,
                TokenKind::Return,
                TokenKind::NewLine,
                TokenKind::Break,
                TokenKind::NewLine,
                TokenKind::Continue,
                TokenKind::EndOfInput,
            ]
        );
        assert_eq!(
            kinds(":outer $i++"),
            vec![
                TokenKind::Label("outer".to_owned()),
                TokenKind::Variable("i".to_owned()),
                TokenKind::PlusPlus,
                TokenKind::EndOfInput,
            ]
        );
        assert_eq!(
            kinds("1 -eq 1"),
            vec![
                TokenKind::Int32(1),
                TokenKind::Eq,
                TokenKind::Int32(1),
                TokenKind::EndOfInput
            ]
        );
        assert_eq!(
            kinds("1..3"),
            vec![
                TokenKind::Int32(1),
                TokenKind::DotDot,
                TokenKind::Int32(3),
                TokenKind::EndOfInput
            ]
        );
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
        assert_eq!(
            kinds("'a\u{2019}"),
            vec![
                TokenKind::StringLiteral("a".to_owned()),
                TokenKind::EndOfInput
            ]
        );
        assert_eq!(
            kinds("\u{2013}1"),
            vec![TokenKind::Minus, TokenKind::Int32(1), TokenKind::EndOfInput]
        );
    }

    #[test]
    fn exit_is_a_keyword_only_as_a_whole_word() {
        assert_eq!(
            kinds("exits"),
            vec![
                TokenKind::Identifier("exits".to_owned()),
                TokenKind::EndOfInput
            ]
        );
        assert_eq!(
            kinds("exit.exe"),
            vec![
                TokenKind::Bareword("exit.exe".to_owned()),
                TokenKind::EndOfInput
            ]
        );
    }
}
