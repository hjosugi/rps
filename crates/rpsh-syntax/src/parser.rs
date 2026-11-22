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
            && self
                .tokens
                .get(self.pos + 1)
                .is_some_and(|token| token.kind == TokenKind::Equals)
        {
            let name = name.clone();
            let start = self.peek().span;
            self.pos += 2;
            let value = self.additive()?;
            let span = start.to(value.span);
            if !self.at_statement_end() {
                return self.refuse(SYNTAX_ERRORS);
            }
            return Ok(Statement::Assign { name, value, span });
        }
        if self.peek().kind != TokenKind::Exit {
            return Ok(Statement::Expression(self.additive()?));
        }
        let exit = self.peek().span;
        self.pos += 1;
        if self.at_statement_end() {
            return Ok(Statement::Exit {
                code: None,
                span: exit,
            });
        }
        let code = self.additive()?;
        let span = exit.to(code.span);
        Ok(Statement::Exit {
            code: Some(code),
            span,
        })
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
        let mut left = self.unary()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Multiply => BinaryOp::Multiply,
                TokenKind::Divide => BinaryOp::Divide,
                TokenKind::Remainder => BinaryOp::Remainder,
                _ => return Ok(left),
            };
            self.pos += 1;
            self.skip_newlines();
            let right = self.unary()?;
            left = self.binary(op, left, right)?;
        }
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().kind {
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
        while self.peek().kind == TokenKind::Dot {
            self.pos += 1;
            let Token { kind, span } = self.peek().clone();
            let TokenKind::Identifier(name) = kind else {
                return self.refuse("member access, dot-sourcing and real literals");
            };
            self.pos += 1;
            let mut span = expression.span.to(span);
            let kind = if self.peek().kind == TokenKind::LParen {
                let open = self.peek().span;
                self.pos += 1;
                if self.peek().kind != TokenKind::RParen {
                    return self.refuse("method arguments");
                }
                span = span.to(self.peek().span);
                self.pos += 1;
                let _ = open;
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
        Ok(expression)
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
            TokenKind::DollarParen => {
                self.pos += 1;
                let inner = self.additive()?;
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
                let value = self.primary()?;
                let span = span.to(value.span);
                let _ = (name_span, close);
                self.expr(
                    ExprKind::Cast {
                        type_name,
                        value: Box::new(value),
                    },
                    span,
                )
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
                    let value = self.additive()?;
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
                let inner = self.additive()?;
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
            ExprKind::Unary { operand, .. } => format!("(-{})", show(operand)),
            ExprKind::Binary { op, left, right } => {
                let symbol = match op {
                    BinaryOp::Add => '+',
                    BinaryOp::Subtract => '-',
                    BinaryOp::Multiply => '*',
                    BinaryOp::Divide => '/',
                    BinaryOp::Remainder => '%',
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
        assert_eq!(refused("$x += 1"), "assignment");
    }

    #[test]
    fn caps_the_size_of_one_expression() {
        let long = vec!["1"; 600].join(" + ");
        assert_eq!(refused(&long), "expressions with more than 1000 nodes");
        assert!(parse(&vec!["1"; 400].join(" + ")).is_ok());
    }
}
