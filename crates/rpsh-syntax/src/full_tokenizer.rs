//! A lexical tokenizer for the full reference token surface.
//!
//! It produces [`FullTokenKind`] tokens with byte spans and line/column, and
//! follows the reference tokenizer's mode switch closely enough to compare
//! token streams: a statement starts in command mode, a literal or operator
//! switches to expression mode, and `|` switches back to command mode.
//!
//! The parser still uses the smaller `tokenize` for now; this is the surface
//! #22 replaces it with. Unsupported characters become `Unknown` tokens so the
//! differential harness can report them without losing the rest.

use crate::command_text::{self, Delimiter};
use crate::full_token_kind::{FullTokenKind, KEYWORDS, OPERATORS, TokenFlags};
use crate::{ParseError, Span, parse_number_literal};

/// One token of the full tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullToken {
    pub kind: FullTokenKind,
    pub span: Span,
    /// UTF-16 code-unit offsets, as the reference parser reports them.
    pub start_char: u32,
    pub end_char: u32,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub flags: TokenFlags,
    /// The same token kind can represent an operator or command argument.
    pub(crate) in_command_arguments: bool,
}

/// Tokenizes `source` into the full token surface.
pub fn tokenize_full(source: &str) -> Result<Vec<FullToken>, ParseError> {
    let mut tokenizer = Tokenizer {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
        mode: Mode::Command,
        command_seen: false,
        statement_start: true,
        ternary_depth: 0,
        hashtable_depth: 0,
        brace_depth: 0,
        command_after_paren: Vec::new(),
        command_member: false,
    };
    tokenizer.run()?;
    let lines = Lines::new(source);
    for token in &mut tokenizer.tokens {
        token.start_char = lines.char_offset(token.span.start);
        token.end_char = lines.char_offset(token.span.end);
        if token.kind == FullTokenKind::EndOfInput {
            let end = lines.char_offset(source.len() as u32) + 1;
            token.start_char = end;
            token.end_char = end;
        }
        (token.start_line, token.start_column) = lines.position(token.span.start);
        (token.end_line, token.end_column) = lines.position(token.span.end);
    }
    Ok(tokenizer.tokens)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Command,
    Expression,
}

struct Lines {
    starts: Vec<usize>,
    // (byte offset, UTF-16 code units before it)
    offsets: Vec<(usize, u32)>,
}

impl Lines {
    fn new(source: &str) -> Self {
        let mut starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        let mut offsets = Vec::new();
        let mut utf16 = 0u32;
        for (byte, ch) in source.char_indices() {
            offsets.push((byte, utf16));
            utf16 += ch.len_utf16() as u32;
        }
        offsets.push((source.len(), utf16));
        Self { starts, offsets }
    }

    fn char_offset(&self, byte: u32) -> u32 {
        let byte = byte as usize;
        match self
            .offsets
            .binary_search_by_key(&byte, |(offset, _)| *offset)
        {
            Ok(index) => self.offsets[index].1,
            Err(index) => self.offsets[index.saturating_sub(1)].1,
        }
    }

    /// Zero-based line and UTF-16-like column; the column counts characters.
    fn position(&self, offset: u32) -> (u32, u32) {
        let offset = offset as usize;
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let line_start = self.starts[line] as u32;
        let column = self.char_offset(offset as u32) - self.char_offset(line_start);
        (line as u32, column)
    }
}

struct Tokenizer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<FullToken>,
    mode: Mode,
    command_seen: bool,
    statement_start: bool,
    ternary_depth: usize,
    hashtable_depth: usize,
    brace_depth: usize,
    command_after_paren: Vec<bool>,
    command_member: bool,
}

impl Tokenizer<'_> {
    fn run(&mut self) -> Result<(), ParseError> {
        while self.pos < self.bytes.len() {
            self.skip_space_and_continuation();
            if self.pos >= self.bytes.len() {
                break;
            }
            let start = self.pos;
            let byte = self.bytes[self.pos];
            if byte == b'\r' || byte == b'\n' {
                self.newline();
                continue;
            }
            if byte == b'`' && self.line_continuation() {
                continue;
            }
            if byte == b'#' {
                self.line_comment();
                continue;
            }
            if byte == b'?' && self.mode == Mode::Command && self.peek(1) == Some(b'[') {
                let start = self.pos;
                let mut end = self.pos + 2;
                while self.bytes.get(end) != Some(&b']') && end < self.bytes.len() {
                    end += 1;
                }
                if self.bytes.get(end) == Some(&b']') {
                    end += 1;
                }
                self.pos = end;
                self.push(FullTokenKind::Generic, start, end);
                self.statement_start = false;
                continue;
            }
            if byte == b':' && self.statement_start {
                let start = self.pos;
                let mut end = start + 1;
                while let Some(ch) = self.source[end..].chars().next() {
                    if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                        end += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                if end > start + 1 {
                    self.pos = end;
                    self.push(FullTokenKind::Label, start, end);
                    self.statement_start = true;
                    self.command_seen = false;
                    continue;
                }
            }
            if byte == b'<' && self.peek(1) == Some(b'#') {
                self.block_comment();
                continue;
            }
            if self.redirection() {
                continue;
            }
            if byte == b':'
                && self.bytes.get(self.pos + 1) != Some(&b':')
                && !(self.statement_start && self.mode == Mode::Command)
            {
                let start = self.pos;
                self.pos += 1;
                let kind = if self.ternary_depth > 0 {
                    self.ternary_depth -= 1;
                    FullTokenKind::Colon
                } else {
                    FullTokenKind::Generic
                };
                self.push(kind, start, self.pos);
                self.statement_start = false;
                continue;
            }
            if self.here_string()? {
                continue;
            }
            if self.verbatim_argument() {
                continue;
            }
            if self.parameter() || self.generic_argument() {
                continue;
            }
            if self.structural()? {
                continue;
            }
            if byte == b'$' {
                self.variable();
                continue;
            }
            if byte == b'@' && self.splat_or_delimiter()? {
                continue;
            }
            if self.number()? {
                continue;
            }
            if self.string()? {
                continue;
            }
            if self.dash_token() {
                continue;
            }
            if self.keyword() {
                continue;
            }
            if self.operator() {
                continue;
            }
            if self.bareword() {
                continue;
            }
            self.push(FullTokenKind::Unknown, start, start + 1);
            self.pos = start + 1;
            self.mode = Mode::Expression;
            self.statement_start = false;
        }
        let end = self.bytes.len();
        self.push_kind(FullTokenKind::EndOfInput, end, end);
        Ok(())
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.pos + ahead).copied()
    }

    fn char(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn peek_char(&self, ahead: usize) -> Option<char> {
        self.source[self.pos..].chars().nth(ahead)
    }

    fn skip_space_and_continuation(&mut self) {
        loop {
            let Some(ch) = self.char() else {
                return;
            };
            if ch == '\r' || ch == '\n' {
                return;
            }
            if ch.is_whitespace() || ch == '\u{feff}' || ch == '\u{200b}' {
                self.pos += ch.len_utf8();
                continue;
            }
            return;
        }
    }

    fn line_continuation(&mut self) -> bool {
        let start = self.pos;
        let mut at = self.pos + 1;
        if self.bytes.get(at) == Some(&b'\r') {
            at += 1;
        }
        if self.bytes.get(at) == Some(&b'\n') {
            self.pos = at + 1;
            self.push(FullTokenKind::LineContinuation, start, self.pos);
            return true;
        }
        false
    }

    fn newline(&mut self) {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'\r') {
            self.pos += 1;
            if self.bytes.get(self.pos) == Some(&b'\n') {
                self.pos += 1;
            }
        } else {
            self.pos += 1;
        }
        self.push(FullTokenKind::NewLine, start, self.pos);
        self.mode = Mode::Command;
        self.command_seen = false;
        self.statement_start = true;
    }

    fn redirection(&mut self) -> bool {
        let start = self.pos;
        let first = self.bytes[start];
        let prefixed = self.peek(1) == Some(b'>')
            && (first == b'*' || (!self.statement_start && matches!(first, b'1'..=b'6')));
        if !prefixed && first != b'>' {
            return false;
        }
        self.pos += if prefixed { 2 } else { 1 };
        if self.bytes.get(self.pos) == Some(&b'>') {
            self.pos += 1;
        } else if prefixed
            && self.peek(0) == Some(b'&')
            && (self.peek(1) == Some(b'1') || (first != b'*' && self.peek(1) == Some(b'2')))
        {
            self.pos += 2;
        }
        self.push(FullTokenKind::Redirection, start, self.pos);
        true
    }

    fn line_comment(&mut self) {
        let start = self.pos;
        while self.pos < self.bytes.len() && !matches!(self.bytes[self.pos], b'\r' | b'\n') {
            self.pos += 1;
        }
        self.push(FullTokenKind::Comment, start, self.pos);
    }

    fn block_comment(&mut self) {
        let start = self.pos;
        let mut at = self.pos + 2;
        while at + 1 < self.bytes.len() {
            if self.bytes[at] == b'#' && self.bytes[at + 1] == b'>' {
                at += 2;
                self.pos = at;
                self.push(FullTokenKind::Comment, start, at);
                return;
            }
            at += 1;
        }
        self.pos = self.bytes.len();
        self.push(FullTokenKind::Comment, start, self.pos);
    }

    fn structural(&mut self) -> Result<bool, ParseError> {
        let start = self.pos;
        let pair = self.bytes.get(self.pos..self.pos + 2);
        let (kind, len) = match pair {
            Some(b"@(") => (FullTokenKind::AtParen, 2),
            Some(b"@{") => (FullTokenKind::AtCurly, 2),
            Some(b"$(") => (FullTokenKind::DollarParen, 2),
            Some(b"?[") if self.mode == Mode::Expression => (FullTokenKind::QuestionLBracket, 2),
            _ => match self.bytes[self.pos] {
                b'(' => (FullTokenKind::LParen, 1),
                b')' => (FullTokenKind::RParen, 1),
                b'{' => (FullTokenKind::LCurly, 1),
                b'}' => (FullTokenKind::RCurly, 1),
                b'[' => (FullTokenKind::LBracket, 1),
                b']' => (FullTokenKind::RBracket, 1),
                b';' => (FullTokenKind::Semi, 1),
                b'<' => {
                    self.statement_start = false;
                    (FullTokenKind::RedirectInStd, 1)
                }
                _ => return Ok(false),
            },
        };
        if matches!(
            kind,
            FullTokenKind::LParen | FullTokenKind::AtParen | FullTokenKind::DollarParen
        ) {
            let argument = self.mode == Mode::Command
                && self.command_seen
                && !self.statement_start
                && self.tokens.last().is_some_and(|token| {
                    token.span.end < start as u32
                        || token.kind == FullTokenKind::Comma
                        || self.after_member()
                        || (token.kind == FullTokenKind::Parameter
                            && token.span.text(self.source).ends_with(':'))
                });
            self.command_after_paren.push(argument);
        }
        self.pos += len;
        self.push(kind, start, self.pos);
        match kind {
            FullTokenKind::AtCurly => self.hashtable_depth += 1,
            FullTokenKind::LCurly => self.brace_depth += 1,
            FullTokenKind::RCurly => {
                if self.hashtable_depth > 0 {
                    self.hashtable_depth -= 1;
                }
                if self.brace_depth > 0 {
                    self.brace_depth -= 1;
                }
            }
            _ => {}
        }
        match kind {
            FullTokenKind::LCurly | FullTokenKind::RCurly | FullTokenKind::Semi => {
                self.mode = Mode::Command;
                self.command_seen = false;
                self.statement_start = true;
            }
            FullTokenKind::RParen => {
                // A postfix member or index still belongs to the expression.
                self.command_seen = self.command_after_paren.pop().unwrap_or(false)
                    && !matches!(self.bytes.get(self.pos), Some(b'.' | b'[' | b'?'));
                self.mode = if self.command_seen {
                    Mode::Command
                } else {
                    Mode::Expression
                };
                self.statement_start = false;
            }
            FullTokenKind::LParen => {
                let call = self.tokens.len() >= 2
                    && matches!(
                        self.tokens[self.tokens.len() - 2].kind,
                        FullTokenKind::Identifier | FullTokenKind::Generic
                    );
                if call {
                    self.mode = Mode::Expression;
                    self.statement_start = false;
                } else {
                    self.mode = Mode::Command;
                    self.command_seen = false;
                    self.statement_start = true;
                }
            }
            FullTokenKind::LBracket
            | FullTokenKind::DollarParen
            | FullTokenKind::AtParen
            | FullTokenKind::AtCurly => {
                self.mode = Mode::Expression;
                self.statement_start = false;
            }
            _ => {}
        }
        Ok(true)
    }

    fn here_string(&mut self) -> Result<bool, ParseError> {
        if self.bytes.get(self.pos) != Some(&b'@') {
            return Ok(false);
        }
        let Some(quote) = self.peek(1) else {
            return Ok(false);
        };
        if let Some(close) = here_string_kind(quote) {
            if !matches!(self.peek(2), Some(b'\r' | b'\n')) {
                return Ok(false);
            }
            let start = self.pos;
            let mut at = self.pos + 2;
            if self.bytes.get(at) == Some(&b'\r') {
                at += 1;
            }
            if self.bytes.get(at) == Some(&b'\n') {
                at += 1;
            }
            while at < self.bytes.len() {
                if self.bytes[at] == b'\n' {
                    let line_start = at + 1;
                    if self.bytes.get(line_start) == Some(&close)
                        && self.bytes.get(line_start + 1) == Some(&b'@')
                    {
                        at = line_start + 2;
                        self.pos = at;
                        self.push(here_string_kind_for(quote), start, at);
                        self.mode = Mode::Expression;
                        self.statement_start = false;
                        return Ok(true);
                    }
                }
                at += 1;
            }
            self.pos = self.bytes.len();
            self.push(here_string_kind_for(quote), start, self.pos);
            self.mode = Mode::Expression;
            self.statement_start = false;
            return Ok(true);
        }
        Ok(false)
    }

    fn variable(&mut self) {
        let start = self.pos;
        let command_argument = self.mode == Mode::Command && self.command_seen;
        self.pos += 1;
        if self.bytes.get(self.pos) == Some(&b'{') {
            while self.pos < self.bytes.len() && self.bytes[self.pos] != b'}' {
                self.pos += 1;
            }
            if self.pos < self.bytes.len() {
                self.pos += 1;
            }
        } else {
            let name = self.pos;
            while let Some(ch) = self.source[self.pos..].chars().next() {
                if ch.is_alphanumeric() || matches!(ch, '_' | ':' | '?' | '$' | '^') {
                    self.pos += ch.len_utf8();
                } else {
                    break;
                }
            }
            if self.pos == name && self.pos < self.bytes.len() {
                self.pos += self.char().map_or(1, char::len_utf8);
            }
        }
        self.push(FullTokenKind::Variable, start, self.pos);
        self.command_member = command_argument && self.bytes.get(self.pos) == Some(&b'.');
        self.mode =
            if command_argument && !matches!(self.bytes.get(self.pos), Some(b'.' | b'[' | b'?')) {
                Mode::Command
            } else {
                Mode::Expression
            };
        self.statement_start = false;
    }

    fn splat_or_delimiter(&mut self) -> Result<bool, ParseError> {
        let start = self.pos;
        self.pos += 1;
        if self.bytes.get(self.pos) == Some(&b'(') || self.bytes.get(self.pos) == Some(&b'{') {
            self.pos = start;
            return Ok(false);
        }
        let mut saw = false;
        while let Some(ch) = self.source[self.pos..].chars().next() {
            if ch.is_alphanumeric() || matches!(ch, '_' | ':' | '?') {
                saw = true;
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        if saw {
            self.push(FullTokenKind::SplattedVariable, start, self.pos);
            if matches!(self.bytes.get(self.pos), Some(b'.' | b'[')) {
                self.mode = Mode::Expression;
            }
            self.statement_start = false;
            Ok(true)
        } else {
            self.pos = start;
            Ok(false)
        }
    }

    fn number(&mut self) -> Result<bool, ParseError> {
        let start = self.pos;
        let Some(mut ch) = self.char() else {
            return Ok(false);
        };
        if ch == '-'
            && self.peek_char(1).is_some_and(|next| next.is_ascii_digit())
            && (self.mode == Mode::Expression || !self.command_seen)
        {
            self.pos += 1;
            let Some(next) = self.char() else {
                self.pos = start;
                return Ok(false);
            };
            ch = next;
        }
        let radix = if self.source[self.pos..].starts_with("0x")
            || self.source[self.pos..].starts_with("0X")
        {
            16
        } else if self.source[self.pos..].starts_with("0b")
            || self.source[self.pos..].starts_with("0B")
        {
            2
        } else {
            10
        };
        if ch == '.' && self.peek_char(1).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        } else if !ch.is_ascii_digit() {
            self.pos = start;
            return Ok(false);
        }
        if radix != 10 {
            self.pos += 2;
            while self.char().is_some_and(|c| match radix {
                16 => c.is_ascii_hexdigit() || c == '_',
                _ => matches!(c, '0' | '1' | '_'),
            }) {
                self.pos += 1;
            }
        } else {
            while self.char().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                self.pos += 1;
            }
            if self.char() == Some('.') && self.peek_char(1) != Some('.') {
                self.pos += 1;
                while self.char().is_some_and(|c| c.is_ascii_digit() || c == '_') {
                    self.pos += 1;
                }
            }
            if matches!(self.char(), Some('e' | 'E')) {
                let save = self.pos;
                self.pos += 1;
                if matches!(self.char(), Some('+' | '-')) {
                    self.pos += 1;
                }
                if self.char().is_some_and(|c| c.is_ascii_digit()) {
                    while self.char().is_some_and(|c| c.is_ascii_digit()) {
                        self.pos += 1;
                    }
                } else {
                    self.pos = save;
                }
            }
        }
        while self.char().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        if self.mode == Mode::Command && self.command_seen && self.char() == Some('>') {
            self.pos = start;
            return Ok(false);
        }
        let text = &self.source[start..self.pos];
        match parse_number_literal(text, Span::new(start, self.pos)) {
            Some(Ok(_)) => {
                self.push(FullTokenKind::Number, start, self.pos);
                if !self.command_seen {
                    self.mode = Mode::Expression;
                }
                self.statement_start = false;
                Ok(true)
            }
            Some(Err(error)) => {
                self.pos = start;
                Err(error)
            }
            None => {
                self.pos = start;
                Ok(false)
            }
        }
    }

    fn string(&mut self) -> Result<bool, ParseError> {
        let Some(quote) = self.char() else {
            return Ok(false);
        };
        let Some(delimiter) = command_text::quote(quote) else {
            return Ok(false);
        };
        let kind = if delimiter == Delimiter::Single {
            FullTokenKind::StringLiteral
        } else {
            FullTokenKind::StringExpandable
        };
        let command_argument = self.mode == Mode::Command && self.command_seen;
        let start = self.pos;
        self.pos = command_text::closing_end(self.source, start + quote.len_utf8(), delimiter)
            .unwrap_or(self.bytes.len());
        self.push(kind, start, self.pos);
        self.mode = if command_argument {
            Mode::Command
        } else {
            Mode::Expression
        };
        self.statement_start = false;
        Ok(true)
    }

    fn dash_token(&mut self) -> bool {
        let Some(ch) = self.char() else {
            return false;
        };
        if !is_dash(ch) {
            return false;
        }
        let start = self.pos;
        let mut at = start + ch.len_utf8();
        if let Some(next) = self.source[at..].chars().next()
            && (next.is_alphabetic() || next == '_' || next == '?')
        {
            if self.mode == Mode::Command && !self.after_member() {
                while let Some(ch) = self.source[at..].chars().next() {
                    if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '?' {
                        at += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                if self.bytes.get(at) == Some(&b':') {
                    at += 1;
                }
                self.pos = at;
                self.push(FullTokenKind::Parameter, start, at);
                return true;
            }
            let word_start = at;
            while let Some(ch) = self.source[at..].chars().next() {
                if ch.is_alphanumeric() {
                    at += ch.len_utf8();
                } else {
                    break;
                }
            }
            let normalized = format!("-{}", &self.source[word_start..at]);
            let explicit_insensitive = normalized
                .strip_prefix("-i")
                .or_else(|| normalized.strip_prefix("-I"))
                .map(|rest| format!("-{rest}"));
            let found = OPERATORS
                .iter()
                .find(|(_, text)| text.eq_ignore_ascii_case(&normalized))
                .or_else(|| {
                    let candidate = explicit_insensitive.as_ref()?;
                    OPERATORS
                        .iter()
                        .find(|(_, text)| text.eq_ignore_ascii_case(candidate))
                });
            if let Some((kind, _)) = found {
                self.pos = at;
                self.push(*kind, start, at);
                self.mode = Mode::Expression;
                self.statement_start = false;
                return true;
            }
        }
        self.pos = start + ch.len_utf8();
        self.push(FullTokenKind::Minus, start, self.pos);
        self.mode = Mode::Expression;
        self.statement_start = false;
        true
    }

    fn parameter(&mut self) -> bool {
        if self.mode != Mode::Command || self.bytes.get(self.pos) != Some(&b'-') {
            return false;
        }
        let start = self.pos;
        let mut at = start + 1;
        if !self.source[at..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_alphabetic() || matches!(ch, '_' | '?'))
        {
            return false;
        }
        while let Some(ch) = self.source[at..].chars().next() {
            if ch.is_whitespace()
                || matches!(
                    ch,
                    '{' | '}' | '(' | ')' | ';' | ',' | '|' | '&' | '.' | '['
                )
            {
                break;
            }
            if command_text::quote(ch).is_some() {
                // A quote converts the entire parameter candidate to a generic word.
                return false;
            }
            at += ch.len_utf8();
            if ch == ':' {
                break;
            }
        }
        if !self.command_seen && is_unary_dash_operator(&self.source[start..at]) {
            return false;
        }
        self.pos = at;
        self.push(FullTokenKind::Parameter, start, at);
        true
    }

    fn keyword(&mut self) -> bool {
        if (self.mode == Mode::Command && !self.statement_start) || self.follows_member_operator() {
            return false;
        }
        let Some((kind, len, end)) = self.word().and_then(|(word, end)| {
            KEYWORDS
                .iter()
                .find(|(_, keyword)| word.eq_ignore_ascii_case(keyword))
                .map(|(kind, _)| (*kind, word.len(), end))
        }) else {
            return false;
        };
        if kind == FullTokenKind::InlineScript && self.mode == Mode::Command && self.statement_start
        {
            return false;
        }
        if let Some(next) = self.source[end..].chars().next()
            && !next.is_whitespace()
            && !matches!(
                next,
                '(' | ')' | '{' | '}' | '[' | ']' | ';' | ',' | '|' | '&' | '<' | '>'
            )
        {
            return false;
        }
        let pipeline_foreach = kind == FullTokenKind::Foreach
            && self
                .tokens
                .iter()
                .rev()
                .find(|token| {
                    !matches!(
                        token.kind,
                        FullTokenKind::NewLine
                            | FullTokenKind::Comment
                            | FullTokenKind::LineContinuation
                    )
                })
                .is_some_and(|token| token.kind == FullTokenKind::Pipe);
        let start = end - len;
        self.pos = end;
        self.push(kind, start, end);
        self.statement_start = false;
        self.command_seen = true;
        self.mode = match kind {
            FullTokenKind::Foreach if pipeline_foreach => Mode::Command,
            FullTokenKind::Return
            | FullTokenKind::Throw
            | FullTokenKind::If
            | FullTokenKind::In
            | FullTokenKind::While
            | FullTokenKind::Until
            | FullTokenKind::For
            | FullTokenKind::Foreach
            | FullTokenKind::Switch
            | FullTokenKind::Catch
            | FullTokenKind::Trap
            | FullTokenKind::Using
            | FullTokenKind::Data
            | FullTokenKind::Exit => Mode::Expression,
            _ => Mode::Command,
        };
        true
    }

    /// True when the cursor follows `.$name`, the target of `$o.name = value`.
    fn after_identifier(&self) -> bool {
        self.tokens.last().is_some_and(|token| {
            matches!(
                token.kind,
                FullTokenKind::Identifier | FullTokenKind::Generic
            )
        })
    }

    fn after_member(&self) -> bool {
        let tokens = &self.tokens;
        tokens.len() >= 2
            && tokens[tokens.len() - 1].kind == FullTokenKind::Identifier
            && tokens[tokens.len() - 2].kind == FullTokenKind::Dot
    }

    /// `-ieq`, `-ine`, ... are explicit case-insensitive spellings of `-eq`,
    /// `-ne`, ...; the operator table keeps only the short form.
    fn explicit_insensitive_operator(&mut self) -> bool {
        if self.bytes.get(self.pos) != Some(&b'-') {
            return false;
        }
        let rest = &self.source[self.pos + 1..];
        let Some(second) = rest.chars().next() else {
            return false;
        };
        if second != 'i' && second != 'I' {
            return false;
        }
        let mut at = second.len_utf8();
        while let Some(ch) = rest[at..].chars().next() {
            if ch.is_alphabetic() {
                at += ch.len_utf8();
            } else {
                break;
            }
        }
        if at == second.len_utf8() {
            return false;
        }
        let candidate = format!("-{}", &rest[second.len_utf8()..at]);
        let Some((kind, _)) = OPERATORS
            .iter()
            .find(|(_, text)| text.eq_ignore_ascii_case(&candidate))
        else {
            return false;
        };
        let start = self.pos;
        self.pos += 1 + at;
        self.push(*kind, start, self.pos);
        self.mode = Mode::Expression;
        self.statement_start = false;
        true
    }

    fn operator(&mut self) -> bool {
        // In command position, ? and % begin barewords. A standalone %
        // still has token kind Rem, but its arguments are in command mode.
        if self.mode == Mode::Command
            && self.statement_start
            && matches!(self.bytes[self.pos], b'?' | b'%')
            && self.peek(1) != Some(b'=')
        {
            return false;
        }
        if self.mode == Mode::Command
            && self.bytes.get(self.pos) == Some(&b'-')
            && let Some(ch) = self.peek_char(1)
            && (ch.is_alphabetic() || ch == '?')
            && !self.after_member()
            && !is_unary_dash_operator(&self.source[self.pos..])
        {
            return false;
        }
        if self.explicit_insensitive_operator() {
            return true;
        }
        for (kind, text) in OPERATORS {
            let end = self.pos + text.len();
            if end <= self.bytes.len()
                && self
                    .source
                    .get(self.pos..end)
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(text))
            {
                // A dash operator must be a whole word: `-in` is not a prefix
                // of `-InputObject`.
                if text.starts_with('-')
                    && self
                        .source
                        .get(end..)
                        .and_then(|rest| rest.chars().next())
                        .is_some_and(|ch| {
                            ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '?'
                        })
                {
                    continue;
                }
                if self.mode == Mode::Command
                    && self.command_seen
                    && !matches!(
                        *kind,
                        FullTokenKind::Pipe
                            | FullTokenKind::AndAnd
                            | FullTokenKind::OrOr
                            | FullTokenKind::Ampersand
                            | FullTokenKind::Comma
                            | FullTokenKind::ColonColon
                    )
                    && !(self.hashtable_depth > 0 && *kind == FullTokenKind::Equals)
                    && !(self.brace_depth > 0
                        && *kind == FullTokenKind::Equals
                        && self.after_identifier())
                    && !self.after_member()
                {
                    continue;
                }
                if self.mode == Mode::Command
                    && (*text == "." || *text == "/")
                    && !self.after_member()
                    && self
                        .bytes
                        .get(end)
                        .is_some_and(|byte| !byte.is_ascii_whitespace())
                {
                    return false;
                }
                let start = self.pos;
                self.pos = end;
                let kind = *kind;
                if kind == FullTokenKind::QuestionMark {
                    self.ternary_depth += 1;
                }
                self.push(kind, start, end);
                let assignment_command = kind == FullTokenKind::Equals
                    && self.tokens.len() >= 2
                    && matches!(
                        self.tokens[self.tokens.len() - 2].kind,
                        FullTokenKind::Variable
                            | FullTokenKind::Identifier
                            | FullTokenKind::Generic
                    )
                    && (self.tokens.len() < 3
                        || matches!(
                            self.tokens[self.tokens.len() - 3].kind,
                            FullTokenKind::NewLine | FullTokenKind::Semi | FullTokenKind::LCurly
                        ));
                if assignment_command {
                    self.mode = Mode::Command;
                    self.command_seen = false;
                    self.statement_start = true;
                    return true;
                }
                match kind {
                    FullTokenKind::Pipe
                    | FullTokenKind::AndAnd
                    | FullTokenKind::OrOr
                    | FullTokenKind::Ampersand => {
                        self.mode = Mode::Command;
                        self.command_seen = false;
                        self.statement_start = true;
                    }
                    FullTokenKind::Dot => {
                        self.mode = Mode::Command;
                        self.command_seen = false;
                        self.statement_start = true;
                    }
                    FullTokenKind::Comma if self.mode == Mode::Command && self.command_seen => {
                        self.statement_start = false;
                    }
                    _ => {
                        self.mode = Mode::Expression;
                        self.statement_start = false;
                    }
                }
                return true;
            }
        }
        false
    }

    /// PowerShell emits a bare marker and a single literal token for the
    /// remainder of the line, up to an unquoted pipe or pipeline chain.
    fn verbatim_argument(&mut self) -> bool {
        let start = self.pos;
        if self.mode != Mode::Command
            || !self.command_seen
            || self.statement_start
            || !self.source[start..].starts_with("--%")
        {
            return false;
        }
        let end = start + 3;
        if self.source[end..].chars().next().is_some_and(|c| {
            !c.is_whitespace() && !matches!(c, ';' | ',' | '|' | '&' | ')' | '}' | '<' | '>')
        }) {
            return false;
        }
        let parameter_value = self.tokens.last().is_some_and(|token| {
            token.kind == FullTokenKind::Parameter && token.span.text(self.source).ends_with(':')
        });
        self.pos = end;
        self.push(FullTokenKind::Generic, start, end);
        if !parameter_value {
            self.skip_space_and_continuation();
            let raw = self.pos;
            let mut quoted = false;
            while let Some(ch) = self.char() {
                if matches!(ch, '\r' | '\n')
                    || !quoted && (ch == '|' || ch == '&' && self.peek(1) == Some(b'&'))
                {
                    break;
                }
                if matches!(ch, '"' | '\u{201c}'..='\u{201e}') {
                    quoted = !quoted;
                }
                self.pos += ch.len_utf8();
            }
            self.push(FullTokenKind::Generic, raw, self.pos);
        }
        true
    }

    /// A generic word can embed quoted segments, escaped characters and
    /// expansions. A standalone quoted expression or variable is its own token.
    fn generic_argument(&mut self) -> bool {
        let start = self.pos;
        if self.mode != Mode::Command || !self.command_seen || self.statement_start {
            return false;
        }
        if self.tokens.last().is_some_and(|prev| {
            prev.kind == FullTokenKind::Catch
                || prev.span.end as usize == start
                    && !matches!(
                        prev.kind,
                        FullTokenKind::Parameter
                            | FullTokenKind::RParen
                            | FullTokenKind::StringLiteral
                            | FullTokenKind::StringExpandable
                            | FullTokenKind::Comma
                    )
        }) {
            return false;
        }
        let first = self.char().unwrap();
        if command_text::quote(first).is_some()
            || matches!(
                first,
                '#' | '(' | ')' | '{' | '}' | '<' | '>' | '|' | '&' | ';' | ','
            )
        {
            return false;
        }
        let mut generic = first == '[';
        if first == '@' {
            let rest = &self.source[start + 1..];
            let tail = rest.trim_start_matches(|ch: char| {
                ch.is_alphanumeric() || matches!(ch, '_' | ':' | '?')
            });
            let Some(next) = tail.chars().next() else {
                return false;
            };
            if tail.len() == rest.len()
                || next.is_whitespace()
                || matches!(
                    next,
                    '.' | '[' | '=' | ',' | ';' | ')' | '}' | '|' | '&' | '<' | '>'
                )
            {
                return false;
            }
            generic = true;
        }
        let mut at = start;
        let mut leading_variable_end = None;
        while let Some(ch) = self.source[at..].chars().next() {
            if ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')' | '{' | '}' | '|' | '&') {
                break;
            }
            let next = at + ch.len_utf8();
            if let Some(kind) = command_text::quote(ch) {
                generic = true;
                at =
                    command_text::closing_end(self.source, next, kind).unwrap_or(self.source.len());
            } else if ch == '`' {
                generic = true;
                at = next + self.source[next..].chars().next().map_or(0, char::len_utf8);
            } else if ch == '$' {
                let begin = at;
                at = next;
                match self.bytes.get(at) {
                    Some(b'(') if begin == start => return false,
                    Some(b'(') => {
                        at = command_text::closing_end(self.source, at + 1, Delimiter::Parentheses)
                            .unwrap_or(self.source.len())
                    }
                    Some(b'{') => {
                        let Some(close) = self.source[at + 1..].find('}') else {
                            return false;
                        };
                        at += close + 2;
                    }
                    _ => {
                        while let Some(var) = self.source[at..].chars().next() {
                            if !(var.is_alphanumeric() || matches!(var, '_' | ':' | '?')) {
                                break;
                            }
                            at += var.len_utf8();
                        }
                    }
                }
                if begin == start {
                    leading_variable_end = Some(at);
                    if matches!(self.bytes.get(at), Some(b'.' | b'[')) {
                        return false;
                    }
                }
                generic = true;
            } else {
                if matches!(ch, '#' | '<' | '>') && at > start
                    || ch == '=' && self.hashtable_depth == 0 && at > start
                {
                    generic = true;
                }
                at = next;
            }
        }
        if at == start || leading_variable_end == Some(at) {
            return false;
        }
        let word = &self.source[start..at];
        if first == '-'
            && word != "--"
            && !word[1..].starts_with(|ch: char| ch.is_alphabetic() || matches!(ch, '_' | '?'))
        {
            generic = true;
        }
        if !generic {
            return false;
        }
        self.pos = at;
        self.push(FullTokenKind::Generic, start, at);
        true
    }

    fn follows_member_operator(&self) -> bool {
        self.tokens.len() >= 2
            && matches!(
                self.tokens[self.tokens.len() - 1].kind,
                FullTokenKind::Dot | FullTokenKind::ColonColon
            )
            && !matches!(
                self.tokens[self.tokens.len() - 2].kind,
                FullTokenKind::NewLine | FullTokenKind::Semi
            )
    }

    fn bareword(&mut self) -> bool {
        let start = self.pos;
        let mut at = self.pos;
        let command_mode = self.mode == Mode::Command;
        let after_dot = self.follows_member_operator();
        while let Some(ch) = self.source[at..].chars().next() {
            let accepted = if command_mode && !after_dot {
                ch.is_alphanumeric()
                    || matches!(
                        ch,
                        '_' | '-' | '.' | '/' | '\\' | ':' | '+' | '~' | '@' | '%' | '?' | '*'
                    )
                    || (self.command_seen && (ch == '[' || ch == ']'))
                    || (at > start && ch == '>')
            } else if after_dot {
                ch.is_alphanumeric() || ch == '_'
            } else {
                ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == '/'
            };
            if accepted {
                at += ch.len_utf8();
            } else {
                break;
            }
        }
        if at == start {
            return false;
        }
        self.pos = at;
        let word = &self.source[start..at];
        let has_command_punctuation = word.chars().any(|ch| !(ch.is_alphanumeric() || ch == '_'));
        let contextual_keyword = command_mode
            && !self.statement_start
            && KEYWORDS
                .iter()
                .any(|(_, keyword)| word.eq_ignore_ascii_case(keyword));
        let kind = if command_mode && self.command_seen && word == "--" {
            FullTokenKind::MinusMinus
        } else if command_mode && self.statement_start && word == "%" {
            FullTokenKind::Rem
        } else if has_command_punctuation
            || contextual_keyword
            || command_mode && self.command_seen && word.as_bytes()[0].is_ascii_digit()
        {
            FullTokenKind::Generic
        } else {
            FullTokenKind::Identifier
        };
        if command_mode {
            self.command_seen = true;
        }
        self.push(kind, start, at);
        if after_dot {
            self.mode = if self.command_member {
                Mode::Command
            } else {
                Mode::Expression
            };
            if self.bytes.get(at) != Some(&b'.') {
                self.command_member = false;
            }
        }
        self.statement_start = false;
        true
    }

    fn word(&self) -> Option<(&str, usize)> {
        let start = self.pos;
        let mut at = self.pos;
        while let Some(ch) = self.source[at..].chars().next() {
            if ch.is_alphanumeric() || ch == '_' {
                at += ch.len_utf8();
            } else {
                break;
            }
        }
        (at > start).then_some((&self.source[start..at], at))
    }

    fn push(&mut self, kind: FullTokenKind, start: usize, end: usize) {
        self.push_kind(kind, start, end);
    }

    fn push_kind(&mut self, kind: FullTokenKind, start: usize, end: usize) {
        self.tokens.push(FullToken {
            kind,
            span: Span::new(start, end),
            start_char: 0,
            end_char: 0,
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
            flags: kind.flags(),
            in_command_arguments: self.mode == Mode::Command && self.command_seen,
        });
    }
}

fn is_unary_dash_operator(text: &str) -> bool {
    ["-not", "-bnot", "-join", "-split"].iter().any(|operator| {
        text.get(..operator.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(operator))
    })
}

fn is_dash(ch: char) -> bool {
    matches!(
        ch,
        '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{fe58}' | '\u{fe63}' | '\u{ff0d}'
    )
}

fn here_string_kind(quote: u8) -> Option<u8> {
    match quote {
        b'\'' => Some(b'\''),
        b'"' => Some(b'"'),
        _ => None,
    }
}

fn here_string_kind_for(quote: u8) -> FullTokenKind {
    if quote == b'\'' {
        FullTokenKind::HereStringLiteral
    } else {
        FullTokenKind::HereStringExpandable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<FullTokenKind> {
        tokenize_full(source)
            .unwrap()
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn tokenizes_a_command_line() {
        assert_eq!(
            kinds("Get-Item -Name x"),
            vec![
                FullTokenKind::Generic,
                FullTokenKind::Parameter,
                FullTokenKind::Identifier,
                FullTokenKind::EndOfInput,
            ]
        );
    }

    #[test]
    fn switches_mode_after_a_literal() {
        assert_eq!(
            kinds("1 -eq 1"),
            vec![
                FullTokenKind::Number,
                FullTokenKind::Ieq,
                FullTokenKind::Number,
                FullTokenKind::EndOfInput,
            ]
        );
        assert_eq!(
            kinds("-eq 1"),
            vec![
                FullTokenKind::Parameter,
                FullTokenKind::Number,
                FullTokenKind::EndOfInput,
            ]
        );
    }

    #[test]
    fn keeps_command_words_together() {
        assert_eq!(
            kinds("a-b"),
            vec![FullTokenKind::Generic, FullTokenKind::EndOfInput]
        );
    }

    #[test]
    fn tokenizes_variables_and_strings() {
        assert_eq!(
            kinds("$x @y ${z} 'a' \"b\""),
            vec![
                FullTokenKind::Variable,
                FullTokenKind::SplattedVariable,
                FullTokenKind::Variable,
                FullTokenKind::StringLiteral,
                FullTokenKind::StringExpandable,
                FullTokenKind::EndOfInput,
            ]
        );
    }

    #[test]
    fn tokenizes_comments_and_line_continuations() {
        assert_eq!(
            kinds("# one\n<# two #>\n1"),
            vec![
                FullTokenKind::Comment,
                FullTokenKind::NewLine,
                FullTokenKind::Comment,
                FullTokenKind::NewLine,
                FullTokenKind::Number,
                FullTokenKind::EndOfInput,
            ]
        );
    }

    #[test]
    fn records_lines_and_columns() {
        let token = tokenize_full("1\n + 2").unwrap()[2];
        assert_eq!(token.span.text("1\n + 2"), "+");
        assert_eq!((token.start_line, token.start_column), (1, 1));
    }
}
