use crate::{
    BinaryOp, Expr, ExprKind, ParseError, ScriptBlockAst, Span, Statement, StringPart, Token,
    TokenKind, UnaryOp, tokenize,
};

const SYNTAX_ERRORS: &str = "syntax error reporting";

/// The evaluator recurses over the tree, so one statement may hold at most
/// this many expression nodes.
const MAX_EXPRESSION_NODES: usize = 1000;

/// Parses a complete script.
///
/// Parsing never has side effects and always covers the whole input, so a
/// script is either accepted in full or refused before it starts running.
pub fn parse(source: &str) -> Result<ScriptBlockAst, ParseError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        nodes: 0,
    };
    let statements = parser.statements()?;
    Ok(ScriptBlockAst {
        statements,
        span: Span::new(0, source.len()),
    })
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// Expression nodes created for the statement being parsed.
    nodes: usize,
}

impl Parser {
    /// The token at the cursor. The cursor never moves past `EndOfInput`.
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn at_statement_end(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::NewLine | TokenKind::Semi | TokenKind::EndOfInput
        )
    }

    fn refuse<T>(&self, feature: &'static str) -> Result<T, ParseError> {
        Err(ParseError::not_implemented(feature, self.peek().span))
    }

    fn statements(&mut self) -> Result<Vec<Statement>, ParseError> {
        let mut statements = Vec::new();
        loop {
            while matches!(self.peek().kind, TokenKind::NewLine | TokenKind::Semi) {
                self.pos += 1;
            }
            if self.peek().kind == TokenKind::EndOfInput {
                return Ok(statements);
            }
            self.nodes = 0;
            statements.push(self.statement()?);
            if !self.at_statement_end() {
                return self.refuse(SYNTAX_ERRORS);
            }
        }
    }

    fn statement(&mut self) -> Result<Statement, ParseError> {
        if let TokenKind::Variable(name) = &self.peek().kind
            && let Some(next) = self.tokens.get(self.pos + 1)
            && matches!(
                next.kind,
                TokenKind::Equals | TokenKind::PlusEquals | TokenKind::CoalesceEquals
            )
        {
            let append = next.kind == TokenKind::PlusEquals;
            let coalesce = next.kind == TokenKind::CoalesceEquals;
            let name = name.clone();
            let start = self.peek().span;
            self.pos += 2;
            let value = self.expression()?;
            let span = start.to(value.span);
            if !self.at_statement_end() {
                return self.refuse(SYNTAX_ERRORS);
            }
            return Ok(if append {
                Statement::AppendAssign { name, value, span }
            } else if coalesce {
                Statement::CoalesceAssign { name, value, span }
            } else {
                Statement::Assign { name, value, span }
            });
        }
        if self.peek().kind != TokenKind::Exit {
            return Ok(Statement::Expression(self.expression()?));
        }
        let exit = self.peek().span;
        self.pos += 1;
        if self.at_statement_end() {
            return Ok(Statement::Exit {
                code: None,
                span: exit,
            });
        }
        let code = self.expression()?;
        let span = exit.to(code.span);
        Ok(Statement::Exit {
            code: Some(code),
            span,
        })
    }

    fn expression(&mut self) -> Result<Expr, ParseError> {
        self.ternary()
    }

    fn ternary(&mut self) -> Result<Expr, ParseError> {
        let condition = self.coalesce()?;
        if self.peek().kind != TokenKind::QuestionMark {
            return Ok(condition);
        }
        let span = condition.span;
        self.pos += 1;
        let when_true = self.ternary()?;
        if self.peek().kind != TokenKind::Colon {
            return self.refuse("ternary ':'");
        }
        self.pos += 1;
        let when_false = self.ternary()?;
        let span = span.to(when_false.span);
        self.expr(
            ExprKind::Ternary {
                condition: Box::new(condition),
                when_true: Box::new(when_true),
                when_false: Box::new(when_false),
            },
            span,
        )
    }

    fn coalesce(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.logical()?;
        while self.peek().kind == TokenKind::Coalesce {
            let span = left.span;
            self.pos += 1;
            let right = self.logical()?;
            let span = span.to(right.span);
            left = self.expr(
                ExprKind::Coalesce {
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            )?;
        }
        Ok(left)
    }

    fn logical(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.bitwise()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::And => BinaryOp::And,
                TokenKind::Or => BinaryOp::Or,
                TokenKind::Xor => BinaryOp::Xor,
                _ => return Ok(left),
            };
            self.pos += 1;
            self.skip_newlines();
            let right = self.bitwise()?;
            left = self.binary(op, left, right)?;
        }
    }

    fn bitwise(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.comparison()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Band => BinaryOp::Band,
                TokenKind::Bor => BinaryOp::Bor,
                TokenKind::Bxor => BinaryOp::Bxor,
                _ => return Ok(left),
            };
            self.pos += 1;
            self.skip_newlines();
            let right = self.comparison()?;
            left = self.binary(op, left, right)?;
        }
    }

    fn comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.range()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Join => BinaryOp::Join,
                TokenKind::Eq => BinaryOp::Eq,
                TokenKind::Ne => BinaryOp::Ne,
                TokenKind::Ge => BinaryOp::Ge,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Le => BinaryOp::Le,
                TokenKind::CEq => BinaryOp::CEq,
                TokenKind::CNe => BinaryOp::CNe,
                TokenKind::CGe => BinaryOp::CGe,
                TokenKind::CGt => BinaryOp::CGt,
                TokenKind::CLt => BinaryOp::CLt,
                TokenKind::CLe => BinaryOp::CLe,
                TokenKind::Like => BinaryOp::Like,
                TokenKind::NotLike => BinaryOp::NotLike,
                TokenKind::CLike => BinaryOp::CLike,
                TokenKind::CNotLike => BinaryOp::CNotLike,
                TokenKind::Contains => BinaryOp::Contains,
                TokenKind::NotContains => BinaryOp::NotContains,
                TokenKind::CContains => BinaryOp::CContains,
                TokenKind::CNotContains => BinaryOp::CNotContains,
                TokenKind::In => BinaryOp::In,
                TokenKind::NotIn => BinaryOp::NotIn,
                TokenKind::CIn => BinaryOp::CIn,
                TokenKind::CNotIn => BinaryOp::CNotIn,
                TokenKind::Shl => BinaryOp::Shl,
                TokenKind::Shr => BinaryOp::Shr,
                TokenKind::Is => BinaryOp::Is,
                TokenKind::IsNot => BinaryOp::IsNot,
                TokenKind::As => BinaryOp::As,
                TokenKind::Match => BinaryOp::Match,
                TokenKind::NotMatch => BinaryOp::NotMatch,
                TokenKind::CMatch => BinaryOp::CMatch,
                TokenKind::CNotMatch => BinaryOp::CNotMatch,
                TokenKind::Replace => BinaryOp::Replace,
                TokenKind::CReplace => BinaryOp::CReplace,
                TokenKind::Split => BinaryOp::Split,
                TokenKind::CSplit => BinaryOp::CSplit,
                _ => return Ok(left),
            };
            self.pos += 1;
            self.skip_newlines();
            let right = self.range()?;
            left = self.binary(op, left, right)?;
        }
    }

    fn range(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.additive()?;
        while self.peek().kind == TokenKind::DotDot {
            self.pos += 1;
            self.skip_newlines();
            let right = self.additive()?;
            left = self.binary(BinaryOp::Range, left, right)?;
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Subtract,
                _ => return Ok(left),
            };
            self.pos += 1;
            self.skip_newlines();
            let right = self.multiplicative()?;
            left = self.binary(op, left, right)?;
        }
    }

    fn multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.comma()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Multiply => BinaryOp::Multiply,
                TokenKind::Divide => BinaryOp::Divide,
                TokenKind::Remainder => BinaryOp::Remainder,
                _ => return Ok(left),
            };
            self.pos += 1;
            self.skip_newlines();
            let right = self.comma()?;
            left = self.binary(op, left, right)?;
        }
    }

    fn comma(&mut self) -> Result<Expr, ParseError> {
        let first = self.unary()?;
        if self.peek().kind != TokenKind::Comma {
            return Ok(first);
        }
        let span = first.span;
        let mut items = vec![first];
        while self.peek().kind == TokenKind::Comma {
            self.pos += 1;
            self.skip_newlines();
            items.push(self.unary()?);
        }
        let span = span.to(items.last().map_or(span, |expr| expr.span));
        self.expr(ExprKind::Array(items), span)
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().kind {
            TokenKind::Comma => {
                let span = self.peek().span;
                self.pos += 1;
                let item = self.unary()?;
                let span = span.to(item.span);
                self.expr(ExprKind::Array(vec![item]), span)
            }
            TokenKind::Join => {
                let span = self.peek().span;
                self.pos += 1;
                let operand = self.unary()?;
                let span = span.to(operand.span);
                self.expr(
                    ExprKind::Unary {
                        op: UnaryOp::Join,
                        operand: Box::new(operand),
                    },
                    span,
                )
            }
            TokenKind::Not | TokenKind::Exclaim => {
                let span = self.peek().span;
                self.pos += 1;
                let operand = self.unary()?;
                let span = span.to(operand.span);
                self.expr(
                    ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                )
            }
            TokenKind::Bnot => {
                let span = self.peek().span;
                self.pos += 1;
                let operand = self.unary()?;
                let span = span.to(operand.span);
                self.expr(
                    ExprKind::Unary {
                        op: UnaryOp::Bnot,
                        operand: Box::new(operand),
                    },
                    span,
                )
            }
            TokenKind::Minus => {
                let minus = self.peek().span;
                self.pos += 1;
                let operand = self.unary()?;
                let span = minus.to(operand.span);
                self.expr(
                    ExprKind::Unary {
                        op: UnaryOp::Negate,
                        operand: Box::new(operand),
                    },
                    span,
                )
            }
            TokenKind::Plus => self.refuse("unary plus"),
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        let mut expression = self.base_expression()?;
        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.pos += 1;
                    let Token { kind, span } = self.peek().clone();
                    let TokenKind::Identifier(name) = kind else {
                        return self.refuse("member access, dot-sourcing and real literals");
                    };
                    self.pos += 1;
                    let mut span = expression.span.to(span);
                    let kind = if self.peek().kind == TokenKind::LParen {
                        self.pos += 1;
                        if self.peek().kind != TokenKind::RParen {
                            return self.refuse("method arguments");
                        }
                        span = span.to(self.peek().span);
                        self.pos += 1;
                        ExprKind::InvokeMember {
                            target: Box::new(expression),
                            name,
                        }
                    } else {
                        ExprKind::Member {
                            target: Box::new(expression),
                            name,
                        }
                    };
                    expression = self.expr(kind, span)?;
                }
                TokenKind::LBracket => {
                    self.pos += 1;
                    let index = self.expression()?;
                    let close = match self.peek().kind {
                        TokenKind::RBracket => self.peek().span,
                        _ => return self.refuse(SYNTAX_ERRORS),
                    };
                    self.pos += 1;
                    let span = expression.span.to(close);
                    expression = self.expr(
                        ExprKind::Index {
                            target: Box::new(expression),
                            index: Box::new(index),
                        },
                        span,
                    )?;
                }
                _ => return Ok(expression),
            }
        }
    }

    fn base_expression(&mut self) -> Result<Expr, ParseError> {
        let Token { kind, span } = self.peek().clone();
        match kind {
            TokenKind::Int32(value) => {
                self.pos += 1;
                self.expr(ExprKind::Int32(value), span)
            }
            TokenKind::Numeric(value) => {
                self.pos += 1;
                self.expr(ExprKind::Number(value), span)
            }
            TokenKind::Variable(name) => {
                self.pos += 1;
                self.expr(ExprKind::Variable(name), span)
            }
            TokenKind::ExpandableString(text) => {
                self.pos += 1;
                let parts = parse_expandable(&text, span)?;
                self.expr(ExprKind::ExpandableString(parts), span)
            }
            TokenKind::HereString { text, expandable } => {
                self.pos += 1;
                let content = here_string_content(&text);
                let parts = if expandable {
                    parse_expandable_body(content, span)?
                } else {
                    vec![StringPart::Literal(content.to_owned())]
                };
                self.expr(ExprKind::ExpandableString(parts), span)
            }
            TokenKind::LCurly => {
                self.pos += 1;
                let mut statements = Vec::new();
                loop {
                    while matches!(self.peek().kind, TokenKind::NewLine | TokenKind::Semi) {
                        self.pos += 1;
                    }
                    if self.peek().kind == TokenKind::RCurly {
                        let close = self.peek().span;
                        self.pos += 1;
                        return self.expr(ExprKind::ScriptBlock(statements), span.to(close));
                    }
                    statements.push(self.statement()?);
                }
            }
            TokenKind::AtParen => {
                self.pos += 1;
                if self.peek().kind == TokenKind::RParen {
                    let close = self.peek().span;
                    self.pos += 1;
                    self.expr(ExprKind::Array(Vec::new()), span.to(close))
                } else {
                    let inner = self.expression()?;
                    let close = match self.peek().kind {
                        TokenKind::RParen => self.peek().span,
                        _ => return self.refuse(SYNTAX_ERRORS),
                    };
                    self.pos += 1;
                    let items = match inner.kind {
                        ExprKind::Array(items) => items,
                        _ => vec![inner],
                    };
                    self.expr(ExprKind::Array(items), span.to(close))
                }
            }
            TokenKind::DollarParen => {
                self.pos += 1;
                let inner = self.expression()?;
                let close = match self.peek().kind {
                    TokenKind::RParen => self.peek().span,
                    _ => return self.refuse(SYNTAX_ERRORS),
                };
                self.pos += 1;
                self.expr(ExprKind::Subexpression(Box::new(inner)), span.to(close))
            }
            TokenKind::LBracket => {
                self.pos += 1;
                let Token {
                    kind,
                    span: name_span,
                } = self.peek().clone();
                let TokenKind::Identifier(type_name) = kind else {
                    return self.refuse("type literals, casts and indexing");
                };
                self.pos += 1;
                if self.peek().kind != TokenKind::RBracket {
                    return self.refuse("type literals, casts and indexing");
                }
                let close = self.peek().span;
                self.pos += 1;
                let _ = name_span;
                if matches!(
                    self.peek().kind,
                    TokenKind::Int32(_)
                        | TokenKind::Numeric(_)
                        | TokenKind::StringLiteral(_)
                        | TokenKind::ExpandableString(_)
                        | TokenKind::HereString { .. }
                        | TokenKind::Variable(_)
                        | TokenKind::LParen
                        | TokenKind::AtParen
                        | TokenKind::AtCurly
                        | TokenKind::LBracket
                        | TokenKind::DollarParen
                ) {
                    let value = self.primary()?;
                    let span = span.to(value.span);
                    self.expr(
                        ExprKind::Cast {
                            type_name,
                            value: Box::new(value),
                        },
                        span,
                    )
                } else {
                    self.expr(ExprKind::TypeLiteral(type_name), span.to(close))
                }
            }
            TokenKind::AtCurly => {
                self.pos += 1;
                let mut entries = Vec::new();
                loop {
                    while matches!(self.peek().kind, TokenKind::NewLine | TokenKind::Semi) {
                        self.pos += 1;
                    }
                    if self.peek().kind == TokenKind::RCurly {
                        self.pos += 1;
                        break;
                    }
                    let Token {
                        kind,
                        span: key_span,
                    } = self.peek().clone();
                    let key = match kind {
                        TokenKind::Identifier(name) => name,
                        TokenKind::StringLiteral(value) => value,
                        _ => return self.refuse("hashtable keys"),
                    };
                    self.pos += 1;
                    if self.peek().kind != TokenKind::Equals {
                        return self.refuse(SYNTAX_ERRORS);
                    }
                    self.pos += 1;
                    let value = self.expression()?;
                    let _ = key_span;
                    entries.push((key, value));
                    while matches!(self.peek().kind, TokenKind::NewLine) {
                        self.pos += 1;
                    }
                    match self.peek().kind {
                        TokenKind::Semi | TokenKind::RCurly => {}
                        _ => return self.refuse(SYNTAX_ERRORS),
                    }
                }
                self.expr(ExprKind::Object(entries), span)
            }
            TokenKind::StringLiteral(value) => {
                self.pos += 1;
                self.expr(ExprKind::String(value), span)
            }
            TokenKind::LParen => {
                self.pos += 1;
                if self.peek().kind == TokenKind::NewLine {
                    return self.refuse("newlines inside parentheses");
                }
                let inner = self.expression()?;
                match self.peek().kind {
                    TokenKind::RParen => {
                        let close = self.peek().span;
                        self.pos += 1;
                        self.expr(ExprKind::Paren(Box::new(inner)), span.to(close))
                    }
                    TokenKind::NewLine => self.refuse("newlines inside parentheses"),
                    _ => self.refuse(SYNTAX_ERRORS),
                }
            }
            TokenKind::Identifier(_) => self.refuse("command invocation"),
            TokenKind::Dot => self.refuse("member access, dot-sourcing and real literals"),
            _ => self.refuse(SYNTAX_ERRORS),
        }
    }

    /// A binary operator may be followed by line breaks before its right
    /// operand.
    fn skip_newlines(&mut self) {
        while self.peek().kind == TokenKind::NewLine {
            self.pos += 1;
        }
    }

    fn binary(&mut self, op: BinaryOp, left: Expr, right: Expr) -> Result<Expr, ParseError> {
        let span = left.span.to(right.span);
        self.expr(
            ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            span,
        )
    }

    fn expr(&mut self, kind: ExprKind, span: Span) -> Result<Expr, ParseError> {
        self.nodes += 1;
        if self.nodes > MAX_EXPRESSION_NODES {
            return Err(ParseError::not_implemented(
                "expressions with more than 1000 nodes",
                span,
            ));
        }
        Ok(Expr { kind, span })
    }
}

fn here_string_content(text: &str) -> &str {
    let mut content = &text[2..text.len().saturating_sub(2)];
    content = content.strip_prefix("\r\n").unwrap_or(content);
    content = content
        .strip_prefix('\n')
        .or_else(|| content.strip_prefix('\r'))
        .unwrap_or(content);
    content = content.strip_suffix("\r\n").unwrap_or(content);
    content = content
        .strip_suffix('\n')
        .or_else(|| content.strip_suffix('\r'))
        .unwrap_or(content);
    content
}

fn parse_expandable(text: &str, span: Span) -> Result<Vec<StringPart>, ParseError> {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return Ok(Vec::new());
    };
    let mut body = &text[first.len_utf8()..];
    let close = match first {
        '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' => '\'',
        '"' => '"',
        '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' => '\u{201d}',
        '@' => '"',
        _ => return Ok(vec![StringPart::Literal(text.to_owned())]),
    };
    body = body.strip_suffix(close).unwrap_or(body);
    parse_expandable_body(body, span)
}

fn parse_expandable_body(body: &str, span: Span) -> Result<Vec<StringPart>, ParseError> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut at = 0;
    while at < body.len() {
        let ch = body[at..].chars().next().unwrap_or('\0');
        match ch {
            '`' => {
                at += 1;
                let Some(escaped) = body[at..].chars().next() else {
                    break;
                };
                at += escaped.len_utf8();
                match escaped {
                    'n' => literal.push('\n'),
                    't' => literal.push('\t'),
                    '0' => literal.push('\0'),
                    'a' => literal.push('\u{7}'),
                    'b' => literal.push('\u{8}'),
                    'e' => literal.push('\u{1b}'),
                    'f' => literal.push('\u{c}'),
                    'r' => literal.push('\r'),
                    'v' => literal.push('\u{b}'),
                    'u' if body[at..].starts_with('{') => {
                        let end = body[at..].find('}').map(|offset| at + offset);
                        if let Some(end) = end {
                            let hex = &body[at + 1..end];
                            if let Ok(code) = u32::from_str_radix(hex, 16)
                                && let Some(ch) = char::from_u32(code)
                            {
                                literal.push(ch);
                                at = end + 1;
                                continue;
                            }
                        }
                        literal.push('u');
                    }
                    other => literal.push(other),
                }
                continue;
            }
            '"' => {
                if body[at + 1..].starts_with('"') {
                    literal.push('"');
                    at += 2;
                    continue;
                }
                literal.push('"');
                at += 1;
                continue;
            }
            '$' => {
                if body[at + 1..].starts_with('(') {
                    if !literal.is_empty() {
                        parts.push(StringPart::Literal(std::mem::take(&mut literal)));
                    }
                    let start = at + 2;
                    let mut depth = 1usize;
                    let mut end = start;
                    for (offset, ch) in body[start..].char_indices() {
                        match ch {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = start + offset;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    let inner = &body[start..end];
                    let ast = parse(inner)?;
                    let Some(Statement::Expression(expr)) = ast.statements.into_iter().next()
                    else {
                        return Err(ParseError::not_implemented(
                            "subexpressions with statements",
                            span,
                        ));
                    };
                    parts.push(StringPart::Subexpression(Box::new(expr)));
                    at = end + 1;
                    continue;
                }
                if body[at + 1..].starts_with('{')
                    && let Some(offset) = body[at + 2..].find('}')
                {
                    let name = &body[at + 2..at + 2 + offset];
                    if !literal.is_empty() {
                        parts.push(StringPart::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(StringPart::Variable(name.to_owned()));
                    at = at + 2 + offset + 1;
                    continue;
                }
                let rest = &body[at + 1..];
                let len = rest
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ':' | '?'))
                    .map(char::len_utf8)
                    .sum::<usize>();
                if len > 0 {
                    let name = &rest[..len];
                    if name.ends_with(':') {
                        return Err(ParseError::not_implemented(
                            "variable reference is not valid ('$x:')",
                            span,
                        ));
                    }
                    if !literal.is_empty() {
                        parts.push(StringPart::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(StringPart::Variable(name.to_owned()));
                    at += 1 + len;
                    continue;
                }
                literal.push('$');
                at += 1;
            }
            other => {
                literal.push(other);
                at += other.len_utf8();
            }
        }
    }
    if !literal.is_empty() {
        parts.push(StringPart::Literal(literal));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders an expression fully parenthesized, to make precedence visible.
    fn show(expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Int32(value) => value.to_string(),
            ExprKind::Number(value) => value.to_string(),
            ExprKind::String(value) => format!("'{value}'"),
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Negate => format!("(-{})", show(operand)),
                UnaryOp::Join => format!("(-join {})", show(operand)),
                UnaryOp::Not => format!("(-not {})", show(operand)),
                UnaryOp::Bnot => format!("(-bnot {})", show(operand)),
            },
            ExprKind::Binary { op, left, right } => {
                let symbol = match op {
                    BinaryOp::Add => "+",
                    BinaryOp::Subtract => "-",
                    BinaryOp::Multiply => "*",
                    BinaryOp::Divide => "/",
                    BinaryOp::Remainder => "%",
                    BinaryOp::Range => "..",
                    BinaryOp::Join => "-join",
                    BinaryOp::Eq => "-eq",
                    BinaryOp::Ne => "-ne",
                    BinaryOp::Ge => "-ge",
                    BinaryOp::Gt => "-gt",
                    BinaryOp::Lt => "-lt",
                    BinaryOp::Le => "-le",
                    BinaryOp::CEq => "-ceq",
                    BinaryOp::CNe => "-cne",
                    BinaryOp::CGe => "-cge",
                    BinaryOp::CGt => "-cgt",
                    BinaryOp::CLt => "-clt",
                    BinaryOp::CLe => "-cle",
                    BinaryOp::Like => "-like",
                    BinaryOp::NotLike => "-notlike",
                    BinaryOp::CLike => "-clike",
                    BinaryOp::CNotLike => "-cnotlike",
                    BinaryOp::Contains => "-contains",
                    BinaryOp::NotContains => "-notcontains",
                    BinaryOp::CContains => "-ccontains",
                    BinaryOp::CNotContains => "-cnotcontains",
                    BinaryOp::In => "-in",
                    BinaryOp::NotIn => "-notin",
                    BinaryOp::CIn => "-cin",
                    BinaryOp::CNotIn => "-cnotin",
                    BinaryOp::Shl => "-shl",
                    BinaryOp::Shr => "-shr",
                    BinaryOp::Band => "-band",
                    BinaryOp::Bor => "-bor",
                    BinaryOp::Bxor => "-bxor",
                    BinaryOp::And => "-and",
                    BinaryOp::Or => "-or",
                    BinaryOp::Xor => "-xor",
                    BinaryOp::Is => "-is",
                    BinaryOp::IsNot => "-isnot",
                    BinaryOp::As => "-as",
                    BinaryOp::Match => "-match",
                    BinaryOp::NotMatch => "-notmatch",
                    BinaryOp::CMatch => "-cmatch",
                    BinaryOp::CNotMatch => "-cnotmatch",
                    BinaryOp::Replace => "-replace",
                    BinaryOp::CReplace => "-creplace",
                    BinaryOp::Split => "-split",
                    BinaryOp::CSplit => "-csplit",
                };
                format!("({} {symbol} {})", show(left), show(right))
            }
            ExprKind::Paren(inner) => show(inner),
            ExprKind::Variable(name) => format!("${name}"),
            ExprKind::ExpandableString(parts) => format!(
                "\"{}\"",
                parts
                    .iter()
                    .map(|part| match part {
                        StringPart::Literal(text) => text.clone(),
                        StringPart::Variable(name) => format!("${name}"),
                        StringPart::Subexpression(expr) => format!("$({})", show(expr)),
                    })
                    .collect::<String>()
            ),
            ExprKind::Subexpression(expr) => format!("$({})", show(expr)),
            ExprKind::TypeLiteral(name) => format!("[{name}]"),
            ExprKind::Array(items) => {
                format!("[{}]", items.iter().map(show).collect::<Vec<_>>().join(","))
            }
            ExprKind::Index { target, index } => format!("{}[{}]", show(target), show(index)),
            ExprKind::Ternary {
                condition,
                when_true,
                when_false,
            } => format!(
                "({} ? {} : {})",
                show(condition),
                show(when_true),
                show(when_false)
            ),
            ExprKind::Coalesce { left, right } => format!("({} ?? {})", show(left), show(right)),
            ExprKind::Object(entries) => format!(
                "@{{{}}}",
                entries
                    .iter()
                    .map(|(name, value)| format!("{name}={}", show(value)))
                    .collect::<Vec<_>>()
                    .join(";")
            ),
            ExprKind::Cast { type_name, value } => format!("[{type_name}]{}", show(value)),
            ExprKind::Member { target, name } => format!("{}.{}", show(target), name),
            ExprKind::InvokeMember { target, name } => format!("{}.{}()", show(target), name),
            ExprKind::ScriptBlock(_) => "{...}".to_owned(),
        }
    }

    fn shown(source: &str) -> Vec<String> {
        parse(source)
            .unwrap()
            .statements
            .iter()
            .map(|statement| match statement {
                Statement::Expression(expr) => show(expr),
                Statement::Assign { name, value, .. } => format!("${name} = {}", show(value)),
                Statement::AppendAssign { name, value, .. } => {
                    format!("${name} += {}", show(value))
                }
                Statement::CoalesceAssign { name, value, .. } => {
                    format!("${name} ??= {}", show(value))
                }
                Statement::Exit { code: None, .. } => "exit".to_owned(),
                Statement::Exit {
                    code: Some(code), ..
                } => format!("exit {}", show(code)),
            })
            .collect()
    }

    fn refused(source: &str) -> &'static str {
        parse(source).unwrap_err().feature()
    }

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        assert_eq!(shown("1 + 2 * 3 - 4"), ["((1 + (2 * 3)) - 4)"]);
        assert_eq!(shown("(1 + 2) * 3"), ["((1 + 2) * 3)"]);
    }

    #[test]
    fn unary_minus_binds_tighter_than_binary_operators() {
        assert_eq!(shown("-2 * 3"), ["(-2 * 3)"]);
        assert_eq!(shown("1 - -1"), ["(1 - -1)"]);
    }

    #[test]
    fn statements_are_separated_by_semicolons_and_line_breaks() {
        assert_eq!(shown(";1; 'a'\n\n2;;"), ["1", "'a'", "2"]);
        assert_eq!(shown(""), Vec::<String>::new());
    }

    #[test]
    fn a_binary_operator_continues_onto_the_next_line() {
        assert_eq!(shown("1 +\n2"), ["(1 + 2)"]);
        assert_eq!(shown("1\n-2"), ["1", "-2"]);
    }

    #[test]
    fn exit_takes_an_optional_expression() {
        assert_eq!(shown("exit"), ["exit"]);
        assert_eq!(shown("exit 1 + 2; 9"), ["exit (1 + 2)", "9"]);
    }

    #[test]
    fn refuses_what_it_does_not_cover() {
        assert_eq!(refused("1 2"), SYNTAX_ERRORS);
        assert_eq!(refused("(1"), SYNTAX_ERRORS);
        assert_eq!(refused("()"), SYNTAX_ERRORS);
        assert_eq!(refused("1 +"), SYNTAX_ERRORS);
        assert_eq!(refused("+1"), "unary plus");
        assert_eq!(refused("(1\n)"), "newlines inside parentheses");
        assert_eq!(
            refused("if (1) { 2 }"),
            "statement keywords (if, foreach, function, ...)"
        );
    }

    #[test]
    fn caps_the_size_of_one_expression() {
        let long = vec!["1"; 600].join(" + ");
        assert_eq!(refused(&long), "expressions with more than 1000 nodes");
        assert!(parse(&vec!["1"; 400].join(" + ")).is_ok());
    }
}
