use crate::{
    BinaryOp, Expr, ExprKind, ParseError, ScriptBlockAst, Span, Statement, Token, TokenKind,
    UnaryOp, tokenize,
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
        while self.peek().kind == TokenKind::Multiply {
            self.pos += 1;
            self.skip_newlines();
            let right = self.unary()?;
            left = self.binary(BinaryOp::Multiply, left, right)?;
        }
        Ok(left)
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
                };
                format!("({} {symbol} {})", show(left), show(right))
            }
            ExprKind::Paren(inner) => show(inner),
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
        assert_eq!(refused("$x = 1"), "variables");
    }

    #[test]
    fn caps_the_size_of_one_expression() {
        let long = vec!["1"; 600].join(" + ");
        assert_eq!(refused(&long), "expressions with more than 1000 nodes");
        assert!(parse(&vec!["1"; 400].join(" + ")).is_ok());
    }
}
