use crate::{
    BinaryOp, CatchClause, CommandArgument, Expr, ExprKind, Parameter, ParseError, PipelineCommand,
    ScriptBlockAst, Span, Statement, StringPart, SwitchClause, Token, TokenKind, UnaryOp, tokenize,
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
        stop_comma: false,
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
    /// In a method argument list a top-level comma separates arguments, so
    /// `comma()` must not build an array. Parenthesized expressions reset it.
    stop_comma: bool,
}

impl Parser {
    /// The token at the cursor. The cursor never moves past `EndOfInput`.
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn at_statement_end(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::NewLine
                | TokenKind::Semi
                | TokenKind::RCurly
                | TokenKind::RParen
                | TokenKind::EndOfInput
        )
    }

    fn previous_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
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
        if let TokenKind::Label(label) = &self.peek().kind {
            let label = label.clone();
            let start = self.peek().span;
            self.pos += 1;
            return self.labeled_statement(label, start);
        }
        if let TokenKind::Variable(name) = &self.peek().kind
            && let Some(next) = self.tokens.get(self.pos + 1)
            && matches!(
                next.kind,
                TokenKind::Equals
                    | TokenKind::PlusEquals
                    | TokenKind::MinusEquals
                    | TokenKind::MultiplyEquals
                    | TokenKind::DivideEquals
                    | TokenKind::RemainderEquals
                    | TokenKind::CoalesceEquals
            )
        {
            let compound = match next.kind {
                TokenKind::MinusEquals => Some(BinaryOp::Subtract),
                TokenKind::MultiplyEquals => Some(BinaryOp::Multiply),
                TokenKind::DivideEquals => Some(BinaryOp::Divide),
                TokenKind::RemainderEquals => Some(BinaryOp::Remainder),
                _ => None,
            };
            let append = next.kind == TokenKind::PlusEquals;
            let coalesce = next.kind == TokenKind::CoalesceEquals;
            let name = name.clone();
            let start = self.peek().span;
            self.pos += 2;
            if !append
                && !coalesce
                && compound.is_none()
                && let TokenKind::Identifier(command) | TokenKind::Bareword(command) =
                    &self.peek().kind
            {
                let command = command.clone();
                let first = self.command_statement(command)?;
                let Statement::Command {
                    name: command,
                    arguments,
                    ..
                } = first
                else {
                    return self.refuse(SYNTAX_ERRORS);
                };
                let mut commands = vec![PipelineCommand::Command {
                    name: command,
                    arguments,
                }];
                while self.peek().kind == TokenKind::Pipe {
                    self.pos += 1;
                    self.skip_newlines();
                    commands.push(self.pipeline_element()?);
                }
                let span = start.to(self.previous_span());
                return Ok(Statement::AssignPipeline {
                    name,
                    commands,
                    span,
                });
            }
            let value = self.nested_expression()?;
            let span = start.to(value.span);
            if self.peek().kind == TokenKind::Pipe && !append && !coalesce && compound.is_none() {
                let mut commands = vec![PipelineCommand::Expression(value)];
                while self.peek().kind == TokenKind::Pipe {
                    self.pos += 1;
                    self.skip_newlines();
                    commands.push(self.pipeline_element()?);
                }
                let span = start.to(self.previous_span());
                return Ok(Statement::AssignPipeline {
                    name,
                    commands,
                    span,
                });
            }
            if !self.at_statement_end() {
                return self.refuse(SYNTAX_ERRORS);
            }
            return Ok(if append {
                Statement::AppendAssign { name, value, span }
            } else if coalesce {
                Statement::CoalesceAssign { name, value, span }
            } else if let Some(op) = compound {
                Statement::CompoundAssign {
                    name,
                    op,
                    value,
                    span,
                }
            } else {
                Statement::Assign { name, value, span }
            });
        }
        match &self.peek().kind {
            TokenKind::If => self.if_statement(),
            TokenKind::While => self.while_statement(),
            TokenKind::For => self.for_statement(),
            TokenKind::Foreach => self.foreach_statement(),
            TokenKind::Do => self.do_statement(),
            TokenKind::Switch => self.switch_statement(),
            TokenKind::Try => self.try_statement(),
            TokenKind::Trap => self.trap_statement(),
            TokenKind::Throw => self.throw_statement(),
            TokenKind::Return => self.return_statement(),
            TokenKind::Break => self.break_statement(false),
            TokenKind::Continue => self.break_statement(true),
            TokenKind::Function => self.function_statement(false),
            TokenKind::Filter => self.function_statement(true),
            TokenKind::Identifier(name) | TokenKind::Bareword(name) => {
                let statement = self.command_statement(name.clone())?;
                self.pipeline_after(statement)
            }
            TokenKind::Exit => self.exit_statement(),
            _ => {
                let expression = self.expression()?;
                if self.peek().kind != TokenKind::Equals {
                    return self.pipeline_after(Statement::Expression(expression));
                }
                let span = expression.span;
                self.pos += 1;
                let value = self.nested_expression()?;
                let span = span.to(value.span);
                if !self.at_statement_end() {
                    return self.refuse(SYNTAX_ERRORS);
                }
                match expression.kind {
                    ExprKind::Member { target, name } => Ok(Statement::MemberAssign {
                        target: *target,
                        name,
                        value,
                        span,
                    }),
                    ExprKind::Index { target, index } => Ok(Statement::IndexAssign {
                        target: *target,
                        index: *index,
                        value,
                        span,
                    }),
                    _ => self.refuse(SYNTAX_ERRORS),
                }
            }
        }
    }

    fn labeled_statement(&mut self, label: String, start: Span) -> Result<Statement, ParseError> {
        let span = |statement_span: Span| start.to(statement_span);
        match self.statement()? {
            Statement::While {
                condition,
                body,
                span: inner,
                ..
            } => Ok(Statement::While {
                label: Some(label),
                condition,
                body,
                span: span(inner),
            }),
            Statement::For {
                init,
                condition,
                step,
                body,
                span: inner,
                ..
            } => Ok(Statement::For {
                label: Some(label),
                init,
                condition,
                step,
                body,
                span: span(inner),
            }),
            Statement::Foreach {
                name,
                collection,
                body,
                span: inner,
                ..
            } => Ok(Statement::Foreach {
                label: Some(label),
                name,
                collection,
                body,
                span: span(inner),
            }),
            Statement::Do {
                body,
                condition,
                until,
                span: inner,
                ..
            } => Ok(Statement::Do {
                label: Some(label),
                body,
                condition,
                until,
                span: span(inner),
            }),
            _ => Err(ParseError::not_implemented(
                "labels on statements other than loops",
                start,
            )),
        }
    }

    /// Parses a `{ ... }` statement block.
    fn block(&mut self) -> Result<(Vec<Statement>, Span), ParseError> {
        self.skip_newlines();
        if self.peek().kind != TokenKind::LCurly {
            return self.refuse(SYNTAX_ERRORS);
        }
        let open = self.peek().span;
        self.pos += 1;
        let mut statements = Vec::new();
        loop {
            while matches!(self.peek().kind, TokenKind::NewLine | TokenKind::Semi) {
                self.pos += 1;
            }
            if self.peek().kind == TokenKind::RCurly {
                let close = self.peek().span;
                self.pos += 1;
                return Ok((statements, open.to(close)));
            }
            if self.peek().kind == TokenKind::EndOfInput {
                return self.refuse(SYNTAX_ERRORS);
            }
            self.nodes = 0;
            statements.push(self.statement()?);
            if !self.at_statement_end() {
                return self.refuse(SYNTAX_ERRORS);
            }
        }
    }

    /// Parses `( expression )`, as a statement keyword's condition uses.
    fn condition(&mut self) -> Result<Expr, ParseError> {
        self.skip_newlines();
        if self.peek().kind != TokenKind::LParen {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        self.skip_newlines();
        let condition = self.nested_expression()?;
        self.skip_newlines();
        if self.peek().kind != TokenKind::RParen {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        Ok(condition)
    }

    /// True when a `|` occurs at the top level before the matching `)`.
    fn paren_has_pipe(&self) -> bool {
        let mut depth = 0usize;
        for token in &self.tokens[self.pos..] {
            match token.kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                TokenKind::Pipe if depth == 0 => return true,
                TokenKind::EndOfInput => return false,
                _ => {}
            }
        }
        false
    }

    fn paren_statements(&mut self) -> Result<Vec<Statement>, ParseError> {
        let mut statements = Vec::new();
        loop {
            while matches!(self.peek().kind, TokenKind::NewLine | TokenKind::Semi) {
                self.pos += 1;
            }
            if self.peek().kind == TokenKind::RParen {
                self.pos += 1;
                return Ok(statements);
            }
            if self.peek().kind == TokenKind::EndOfInput {
                return self.refuse(SYNTAX_ERRORS);
            }
            self.nodes = 0;
            statements.push(self.statement()?);
            if !self.at_statement_end() {
                return self.refuse(SYNTAX_ERRORS);
            }
        }
    }

    /// Parses a type name with dotted parts, generic arguments and array
    /// suffixes: `System.Collections.Generic.HashSet[string]`, `string[]`.
    fn parse_type_name(&mut self) -> Result<String, ParseError> {
        let Token { kind, .. } = self.peek().clone();
        let TokenKind::Identifier(identifier) = kind else {
            return self.refuse("type literals, casts and indexing");
        };
        let mut name = identifier;
        self.pos += 1;
        loop {
            match &self.peek().kind {
                TokenKind::Dot => {
                    self.pos += 1;
                    let Token { kind, .. } = self.peek().clone();
                    let TokenKind::Identifier(identifier) = kind else {
                        return self.refuse("type literals, casts and indexing");
                    };
                    name.push('.');
                    name.push_str(&identifier);
                    self.pos += 1;
                }
                TokenKind::LBracket => {
                    if self
                        .tokens
                        .get(self.pos + 1)
                        .is_some_and(|token| token.kind == TokenKind::RBracket)
                    {
                        self.pos += 2;
                        name.push_str("[]");
                        continue;
                    }
                    self.pos += 1;
                    name.push('[');
                    loop {
                        name.push_str(&self.parse_type_name()?);
                        if self.peek().kind == TokenKind::Comma {
                            self.pos += 1;
                            name.push(',');
                            continue;
                        }
                        break;
                    }
                    if self.peek().kind != TokenKind::RBracket {
                        return self.refuse("type literals, casts and indexing");
                    }
                    self.pos += 1;
                    name.push(']');
                }
                _ => break,
            }
        }
        Ok(name)
    }

    /// Parses `( argument, ... )` for a method call.
    fn arguments(&mut self) -> Result<(Vec<Expr>, Span), ParseError> {
        self.pos += 1;
        let stop_comma = self.stop_comma;
        self.stop_comma = true;
        let mut arguments = Vec::new();
        self.skip_newlines();
        if self.peek().kind != TokenKind::RParen {
            loop {
                arguments.push(self.expression()?);
                self.skip_newlines();
                if self.peek().kind == TokenKind::Comma {
                    self.pos += 1;
                    self.skip_newlines();
                    continue;
                }
                break;
            }
        }
        self.stop_comma = stop_comma;
        if self.peek().kind != TokenKind::RParen {
            return self.refuse(SYNTAX_ERRORS);
        }
        let close = self.peek().span;
        self.pos += 1;
        Ok((arguments, close))
    }

    /// Wraps the first stage in a pipeline when a `|` follows.
    fn pipeline_after(&mut self, first: Statement) -> Result<Statement, ParseError> {
        if self.peek().kind != TokenKind::Pipe {
            return Ok(first);
        }
        let start = match &first {
            Statement::Expression(expr) => expr.span,
            Statement::Command { span, .. } => *span,
            _ => return Ok(first),
        };
        let mut commands = Vec::new();
        match first {
            Statement::Expression(expr) => commands.push(PipelineCommand::Expression(expr)),
            Statement::Command {
                name, arguments, ..
            } => {
                commands.push(PipelineCommand::Command { name, arguments });
            }
            other => return Ok(other),
        }
        while self.peek().kind == TokenKind::Pipe {
            self.pos += 1;
            self.skip_newlines();
            commands.push(self.pipeline_element()?);
        }
        let span = start.to(self.previous_span());
        Ok(Statement::Pipeline { commands, span })
    }

    fn pipeline_element(&mut self) -> Result<PipelineCommand, ParseError> {
        match &self.peek().kind {
            TokenKind::Identifier(name) | TokenKind::Bareword(name) => {
                let name = name.clone();
                let statement = self.command_statement(name)?;
                let Statement::Command {
                    name, arguments, ..
                } = statement
                else {
                    return self.refuse(SYNTAX_ERRORS);
                };
                Ok(PipelineCommand::Command { name, arguments })
            }
            _ => Ok(PipelineCommand::Expression(self.nested_expression()?)),
        }
    }

    /// Parses `Name arg -Param value ...` in command mode.
    fn command_statement(&mut self, name: String) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let mut arguments = Vec::new();
        while !self.at_statement_end() && self.peek().kind != TokenKind::Pipe {
            if matches!(self.peek().kind, TokenKind::Redirection(_)) {
                self.pos += 1;
                continue;
            }
            if matches!(
                self.peek().kind,
                TokenKind::Int32(_) | TokenKind::Numeric(_)
            ) && self
                .tokens
                .get(self.pos + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Redirection(_)))
            {
                self.pos += 2;
                continue;
            }
            let parameter = match &self.peek().kind {
                TokenKind::Identifier(word) | TokenKind::Bareword(word)
                    if word.starts_with('-') =>
                {
                    let parameter = word.trim_start_matches('-').to_owned();
                    self.pos += 1;
                    Some(parameter)
                }
                TokenKind::Parameter(parameter) => {
                    let parameter = parameter.clone();
                    self.pos += 1;
                    Some(parameter)
                }
                TokenKind::Minus => match self.tokens.get(self.pos + 1).map(|token| &token.kind) {
                    Some(TokenKind::Identifier(parameter) | TokenKind::Bareword(parameter)) => {
                        let parameter = parameter.clone();
                        self.pos += 2;
                        Some(parameter)
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(parameter) = parameter {
                let value = match &self.peek().kind {
                    TokenKind::Identifier(word) | TokenKind::Bareword(word) => {
                        let word = word.clone();
                        let span = self.peek().span;
                        self.pos += 1;
                        Some(Expr {
                            kind: ExprKind::String(word),
                            span,
                        })
                    }
                    _ if self.argument_starts() => Some(self.unary()?),
                    _ => None,
                };
                arguments.push(CommandArgument::Parameter {
                    name: parameter,
                    value,
                });
                continue;
            }
            match &self.peek().kind {
                TokenKind::Identifier(word) | TokenKind::Bareword(word) => {
                    let word = word.clone();
                    self.pos += 1;
                    arguments.push(CommandArgument::Bareword(word));
                }
                _ => arguments.push(CommandArgument::Value(self.argument_value()?)),
            }
        }
        let span = start.to(self.previous_span());
        Ok(Statement::Command {
            name,
            arguments,
            span,
        })
    }

    /// A single command argument. A bracketed value stops at its closing
    /// bracket so `(1..3) -NoEnumerate` does not read as subtraction.
    fn argument_value(&mut self) -> Result<Expr, ParseError> {
        if matches!(
            self.peek().kind,
            TokenKind::LParen
                | TokenKind::AtParen
                | TokenKind::AtCurly
                | TokenKind::LBracket
                | TokenKind::DollarParen
        ) {
            self.primary()
        } else {
            self.nested_expression()
        }
    }

    fn argument_starts(&self) -> bool {
        matches!(
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
                | TokenKind::Minus
                | TokenKind::PlusPlus
                | TokenKind::MinusMinus
                | TokenKind::Exclaim
                | TokenKind::Not
                | TokenKind::Bnot
        )
    }

    fn function_statement(&mut self, filter: bool) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let name = match &self.peek().kind {
            TokenKind::Identifier(name) | TokenKind::Bareword(name) => name.clone(),
            _ => return self.refuse(SYNTAX_ERRORS),
        };
        self.pos += 1;
        let mut parameters = Vec::new();
        self.skip_newlines();
        if self.peek().kind == TokenKind::LParen {
            parameters = self.parameter_list()?;
        }
        self.skip_newlines();
        if self.peek().kind != TokenKind::LCurly {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        let mut body = Vec::new();
        let mut begin = Vec::new();
        let mut process = Vec::new();
        let mut end = Vec::new();
        loop {
            while matches!(self.peek().kind, TokenKind::NewLine | TokenKind::Semi) {
                self.pos += 1;
            }
            if self.peek().kind == TokenKind::RCurly {
                self.pos += 1;
                break;
            }
            if self.peek().kind == TokenKind::EndOfInput {
                return self.refuse(SYNTAX_ERRORS);
            }
            if self.peek().kind == TokenKind::Param {
                self.pos += 1;
                parameters = self.parameter_list()?;
                continue;
            }
            match self.peek().kind {
                TokenKind::Begin => {
                    self.pos += 1;
                    begin = self.block()?.0;
                    continue;
                }
                TokenKind::Process => {
                    self.pos += 1;
                    process = self.block()?.0;
                    continue;
                }
                TokenKind::End => {
                    self.pos += 1;
                    end = self.block()?.0;
                    continue;
                }
                _ => {}
            }
            self.nodes = 0;
            body.push(self.statement()?);
            if !self.at_statement_end() {
                return self.refuse(SYNTAX_ERRORS);
            }
        }
        let span = start.to(self.previous_span());
        Ok(Statement::Function {
            name,
            parameters,
            begin,
            process,
            end,
            body,
            filter,
            span,
        })
    }

    /// Parses `( [type]$name [= default], ... )`.
    fn parameter_list(&mut self) -> Result<Vec<Parameter>, ParseError> {
        self.pos += 1;
        let mut parameters = Vec::new();
        self.skip_newlines();
        if self.peek().kind == TokenKind::RParen {
            self.pos += 1;
            return Ok(parameters);
        }
        loop {
            self.skip_newlines();
            let type_name = if self.peek().kind == TokenKind::LBracket {
                Some(self.exception_type()?)
            } else {
                None
            };
            self.skip_newlines();
            let Token { kind, .. } = self.peek().clone();
            let TokenKind::Variable(name) = kind else {
                return self.refuse(SYNTAX_ERRORS);
            };
            self.pos += 1;
            self.skip_newlines();
            let default = if self.peek().kind == TokenKind::Equals {
                self.pos += 1;
                Some(self.nested_expression()?)
            } else {
                None
            };
            parameters.push(Parameter {
                name,
                type_name,
                default,
            });
            self.skip_newlines();
            match self.peek().kind {
                TokenKind::Comma => self.pos += 1,
                TokenKind::RParen => {
                    self.pos += 1;
                    return Ok(parameters);
                }
                _ => return self.refuse(SYNTAX_ERRORS),
            }
        }
    }

    fn switch_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let mut wildcard = false;
        let mut regex = false;
        let mut case_sensitive = false;
        loop {
            let flag = match &self.peek().kind {
                TokenKind::Parameter(flag) => {
                    let flag = flag.clone();
                    self.pos += 1;
                    flag
                }
                TokenKind::Identifier(flag) | TokenKind::Bareword(flag)
                    if flag.starts_with('-') =>
                {
                    let flag = flag.trim_start_matches('-').to_owned();
                    self.pos += 1;
                    flag
                }
                TokenKind::Minus if self.pos + 1 < self.tokens.len() => {
                    let (TokenKind::Identifier(flag) | TokenKind::Bareword(flag)) =
                        &self.tokens[self.pos + 1].kind
                    else {
                        break;
                    };
                    let flag = flag.clone();
                    self.pos += 2;
                    flag
                }
                _ => break,
            };
            if flag.eq_ignore_ascii_case("wildcard") {
                wildcard = true;
            } else if flag.eq_ignore_ascii_case("regex") {
                regex = true;
            } else if flag.eq_ignore_ascii_case("casesensitive") {
                case_sensitive = true;
            } else {
                return self.refuse("this switch flag");
            }
        }
        let condition = self.condition()?;
        self.skip_newlines();
        if self.peek().kind != TokenKind::LCurly {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        let mut clauses = Vec::new();
        loop {
            while matches!(self.peek().kind, TokenKind::NewLine | TokenKind::Semi) {
                self.pos += 1;
            }
            if self.peek().kind == TokenKind::RCurly {
                self.pos += 1;
                break;
            }
            if self.peek().kind == TokenKind::EndOfInput {
                return self.refuse(SYNTAX_ERRORS);
            }
            if self.peek().kind == TokenKind::Default {
                self.pos += 1;
                let body = self.block()?.0;
                clauses.push(SwitchClause { label: None, body });
                continue;
            }
            let label = match &self.peek().kind {
                TokenKind::Identifier(word) | TokenKind::Bareword(word) => {
                    let word = word.clone();
                    let span = self.peek().span;
                    self.pos += 1;
                    Expr {
                        kind: ExprKind::String(word),
                        span,
                    }
                }
                _ => self.nested_expression()?,
            };
            let body = self.block()?.0;
            clauses.push(SwitchClause {
                label: Some(label),
                body,
            });
        }
        let span = start.to(self.previous_span());
        Ok(Statement::Switch {
            condition,
            wildcard,
            regex,
            case_sensitive,
            clauses,
            span,
        })
    }

    fn if_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let condition = self.condition()?;
        let (then, _) = self.block()?;
        let mut elseif = Vec::new();
        let mut otherwise = Vec::new();
        loop {
            let save = self.pos;
            self.skip_newlines();
            match self.peek().kind {
                TokenKind::ElseIf => {
                    self.pos += 1;
                    let condition = self.condition()?;
                    let (body, _) = self.block()?;
                    elseif.push((condition, body));
                }
                TokenKind::Else => {
                    self.pos += 1;
                    let save = self.pos;
                    self.skip_newlines();
                    if self.peek().kind == TokenKind::If {
                        otherwise = vec![self.if_statement()?];
                    } else {
                        self.pos = save;
                        otherwise = self.block()?.0;
                    }
                    break;
                }
                _ => {
                    self.pos = save;
                    break;
                }
            }
        }
        let span = start.to(self.previous_span());
        Ok(Statement::If {
            condition,
            then,
            elseif,
            otherwise,
            span,
        })
    }

    fn while_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let condition = self.condition()?;
        let (body, _) = self.block()?;
        let span = start.to(self.previous_span());
        Ok(Statement::While {
            label: None,
            condition,
            body,
            span,
        })
    }

    fn for_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        self.skip_newlines();
        if self.peek().kind != TokenKind::LParen {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        self.skip_newlines();
        let init = if self.peek().kind == TokenKind::Semi {
            None
        } else {
            Some(Box::new(self.statement()?))
        };
        if self.peek().kind != TokenKind::Semi {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        self.skip_newlines();
        let condition = if self.peek().kind == TokenKind::Semi {
            None
        } else {
            Some(self.nested_expression()?)
        };
        if self.peek().kind != TokenKind::Semi {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        self.skip_newlines();
        let step = if self.peek().kind == TokenKind::RParen {
            None
        } else {
            Some(Box::new(self.statement()?))
        };
        self.skip_newlines();
        if self.peek().kind != TokenKind::RParen {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        let (body, _) = self.block()?;
        let span = start.to(self.previous_span());
        Ok(Statement::For {
            label: None,
            init,
            condition,
            step,
            body,
            span,
        })
    }

    fn foreach_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        self.skip_newlines();
        if self.peek().kind != TokenKind::LParen {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        self.skip_newlines();
        let Token { kind, .. } = self.peek().clone();
        let TokenKind::Variable(name) = kind else {
            return self.refuse("foreach without a variable");
        };
        self.pos += 1;
        self.skip_newlines();
        if self.peek().kind != TokenKind::In {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        self.skip_newlines();
        let collection = self.nested_expression()?;
        self.skip_newlines();
        if self.peek().kind != TokenKind::RParen {
            return self.refuse(SYNTAX_ERRORS);
        }
        self.pos += 1;
        let (body, _) = self.block()?;
        let span = start.to(self.previous_span());
        Ok(Statement::Foreach {
            label: None,
            name,
            collection,
            body,
            span,
        })
    }

    fn do_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let (body, _) = self.block()?;
        self.skip_newlines();
        let until = match self.peek().kind {
            TokenKind::While => false,
            TokenKind::Until => true,
            _ => return self.refuse(SYNTAX_ERRORS),
        };
        self.pos += 1;
        let condition = self.condition()?;
        let span = start.to(self.previous_span());
        Ok(Statement::Do {
            label: None,
            body,
            condition,
            until,
            span,
        })
    }

    fn try_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let (body, _) = self.block()?;
        let mut catches = Vec::new();
        loop {
            let save = self.pos;
            self.skip_newlines();
            if self.peek().kind != TokenKind::Catch {
                self.pos = save;
                break;
            }
            let catch_start = self.peek().span;
            self.pos += 1;
            let mut types = Vec::new();
            loop {
                self.skip_newlines();
                if self.peek().kind != TokenKind::LBracket {
                    break;
                }
                types.push(self.exception_type()?);
                self.skip_newlines();
                if self.peek().kind != TokenKind::Comma {
                    break;
                }
                self.pos += 1;
            }
            let (catch_body, catch_span) = self.block()?;
            catches.push(CatchClause {
                types,
                body: catch_body,
                span: catch_start.to(catch_span),
            });
        }
        let save = self.pos;
        self.skip_newlines();
        let finally = if self.peek().kind == TokenKind::Finally {
            self.pos += 1;
            Some(self.block()?.0)
        } else {
            self.pos = save;
            None
        };
        if catches.is_empty() && finally.is_none() {
            return self.refuse(SYNTAX_ERRORS);
        }
        let span = start.to(self.previous_span());
        Ok(Statement::Try {
            body,
            catches,
            finally,
            span,
        })
    }

    /// Parses an exception type in a `catch` clause, `[A.B.C]`.
    fn exception_type(&mut self) -> Result<String, ParseError> {
        self.pos += 1;
        let mut name = String::new();
        loop {
            self.skip_newlines();
            match &self.peek().kind {
                TokenKind::Identifier(part) => {
                    name.push_str(part);
                    self.pos += 1;
                }
                _ => return self.refuse(SYNTAX_ERRORS),
            }
            match self.peek().kind {
                TokenKind::Dot => {
                    name.push('.');
                    self.pos += 1;
                }
                TokenKind::RBracket => {
                    self.pos += 1;
                    return Ok(name);
                }
                _ => return self.refuse(SYNTAX_ERRORS),
            }
        }
    }

    fn trap_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let (body, _) = self.block()?;
        let span = start.to(self.previous_span());
        Ok(Statement::Trap { body, span })
    }

    fn throw_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        if self.at_statement_end() {
            return self.refuse("throw without an expression");
        }
        let value = self.nested_expression()?;
        let span = start.to(value.span);
        Ok(Statement::Throw { value, span })
    }

    fn return_statement(&mut self) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        if self.at_statement_end() {
            return Ok(Statement::Return {
                value: None,
                span: start,
            });
        }
        let value = self.nested_expression()?;
        let span = start.to(value.span);
        Ok(Statement::Return {
            value: Some(value),
            span,
        })
    }

    fn break_statement(&mut self, is_continue: bool) -> Result<Statement, ParseError> {
        let start = self.peek().span;
        self.pos += 1;
        let label = match &self.peek().kind {
            TokenKind::Identifier(label) | TokenKind::Label(label) => {
                let label = label.clone();
                self.pos += 1;
                Some(label)
            }
            _ => None,
        };
        let span = start.to(self.previous_span());
        Ok(if is_continue {
            Statement::Continue { label, span }
        } else {
            Statement::Break { label, span }
        })
    }

    fn exit_statement(&mut self) -> Result<Statement, ParseError> {
        let exit = self.peek().span;
        self.pos += 1;
        if self.at_statement_end() {
            return Ok(Statement::Exit {
                code: None,
                span: exit,
            });
        }
        let code = self.nested_expression()?;
        let span = exit.to(code.span);
        Ok(Statement::Exit {
            code: Some(code),
            span,
        })
    }

    fn expression(&mut self) -> Result<Expr, ParseError> {
        self.ternary()
    }

    /// An expression in a nested context, where a comma builds an array again.
    fn nested_expression(&mut self) -> Result<Expr, ParseError> {
        let stop_comma = self.stop_comma;
        self.stop_comma = false;
        let result = self.expression();
        self.stop_comma = stop_comma;
        result
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
                TokenKind::Format => BinaryOp::Format,
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
        if self.stop_comma || self.peek().kind != TokenKind::Comma {
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
            TokenKind::PlusPlus | TokenKind::MinusMinus => {
                let increment = self.peek().kind == TokenKind::PlusPlus;
                let span = self.peek().span;
                self.pos += 1;
                let operand = self.unary()?;
                let span = span.to(operand.span);
                let op = if increment {
                    UnaryOp::Increment
                } else {
                    UnaryOp::Decrement
                };
                self.expr(
                    ExprKind::Unary {
                        op,
                        operand: Box::new(operand),
                    },
                    span,
                )
            }
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
                        let (arguments, close) = self.arguments()?;
                        span = span.to(close);
                        ExprKind::InvokeMember {
                            target: Box::new(expression),
                            name,
                            arguments,
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
                    let index = self.nested_expression()?;
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
                TokenKind::PlusPlus | TokenKind::MinusMinus => {
                    let increment = self.peek().kind == TokenKind::PlusPlus;
                    let close = self.peek().span;
                    self.pos += 1;
                    let span = expression.span.to(close);
                    let op = if increment {
                        UnaryOp::PostIncrement
                    } else {
                        UnaryOp::PostDecrement
                    };
                    expression = self.expr(
                        ExprKind::Unary {
                            op,
                            operand: Box::new(expression),
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
                    let inner = self.nested_expression()?;
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
                let inner = self.nested_expression()?;
                let close = match self.peek().kind {
                    TokenKind::RParen => self.peek().span,
                    _ => return self.refuse(SYNTAX_ERRORS),
                };
                self.pos += 1;
                self.expr(ExprKind::Subexpression(Box::new(inner)), span.to(close))
            }
            TokenKind::LBracket => {
                self.pos += 1;
                let type_name = self.parse_type_name()?;
                if self.peek().kind != TokenKind::RBracket {
                    return self.refuse("type literals, casts and indexing");
                }
                let close = self.peek().span;
                self.pos += 1;
                if self.peek().kind == TokenKind::ColonColon {
                    self.pos += 1;
                    let Token {
                        kind,
                        span: name_span,
                    } = self.peek().clone();
                    let TokenKind::Identifier(name) = kind else {
                        return self.refuse("static member access");
                    };
                    self.pos += 1;
                    if self.peek().kind == TokenKind::LParen {
                        let (arguments, close) = self.arguments()?;
                        let span = span.to(close);
                        return self.expr(
                            ExprKind::InvokeStatic {
                                type_name,
                                name,
                                arguments,
                            },
                            span,
                        );
                    }
                    let span = span.to(name_span);
                    return self.expr(ExprKind::StaticMember { type_name, name }, span);
                }
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
                        TokenKind::Int32(value) => value.to_string(),
                        TokenKind::Numeric(value) => value.to_string(),
                        _ => return self.refuse("hashtable keys"),
                    };
                    self.pos += 1;
                    if self.peek().kind != TokenKind::Equals {
                        return self.refuse(SYNTAX_ERRORS);
                    }
                    self.pos += 1;
                    let value = self.nested_expression()?;
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
                if matches!(
                    self.peek().kind,
                    TokenKind::Identifier(_) | TokenKind::Bareword(_)
                ) || self.paren_has_pipe()
                {
                    let statements = self.paren_statements()?;
                    return self.expr(ExprKind::Group(statements), span.to(self.previous_span()));
                }
                let inner = self.nested_expression()?;
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
                UnaryOp::Increment => format!("(++{})", show(operand)),
                UnaryOp::Decrement => format!("(--{})", show(operand)),
                UnaryOp::PostIncrement => format!("({}++)", show(operand)),
                UnaryOp::PostDecrement => format!("({}--)", show(operand)),
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
                    BinaryOp::Format => "-f",
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
            ExprKind::Group(_) => "(...)".to_owned(),
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
            ExprKind::StaticMember { type_name, name } => format!("[{type_name}]::{name}"),
            ExprKind::InvokeStatic {
                type_name,
                name,
                arguments,
            } => format!(
                "[{type_name}]::{name}({})",
                arguments.iter().map(show).collect::<Vec<_>>().join(",")
            ),
            ExprKind::InvokeMember {
                target,
                name,
                arguments,
            } => format!(
                "{}.{}({})",
                show(target),
                name,
                arguments.iter().map(show).collect::<Vec<_>>().join(",")
            ),
            ExprKind::ScriptBlock(_) => "{...}".to_owned(),
        }
    }

    fn shown(source: &str) -> Vec<String> {
        parse(source)
            .unwrap()
            .statements
            .iter()
            .map(show_statement)
            .collect()
    }

    fn show_block(statements: &[Statement]) -> String {
        statements
            .iter()
            .map(show_statement)
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn show_label(label: &Option<String>) -> String {
        label
            .as_ref()
            .map(|label| format!(":{label} "))
            .unwrap_or_default()
    }

    fn show_statement(statement: &Statement) -> String {
        match statement {
            Statement::Expression(expr) => show(expr),
            Statement::Assign { name, value, .. } => format!("${name} = {}", show(value)),
            Statement::AppendAssign { name, value, .. } => format!("${name} += {}", show(value)),
            Statement::CoalesceAssign { name, value, .. } => {
                format!("${name} ??= {}", show(value))
            }
            Statement::CompoundAssign {
                name, op, value, ..
            } => {
                let symbol = match op {
                    BinaryOp::Subtract => "-=",
                    BinaryOp::Multiply => "*=",
                    BinaryOp::Divide => "/=",
                    BinaryOp::Remainder => "%=",
                    _ => "?=",
                };
                format!("${name} {symbol} {}", show(value))
            }
            Statement::If {
                condition,
                then,
                elseif,
                otherwise,
                ..
            } => {
                let mut text = format!("if {} {{ {} }}", show(condition), show_block(then));
                for (condition, body) in elseif {
                    text.push_str(&format!(
                        " elseif {} {{ {} }}",
                        show(condition),
                        show_block(body)
                    ));
                }
                if !otherwise.is_empty() {
                    text.push_str(&format!(" else {{ {} }}", show_block(otherwise)));
                }
                text
            }
            Statement::While {
                label,
                condition,
                body,
                ..
            } => format!(
                "{}while {} {{ {} }}",
                show_label(label),
                show(condition),
                show_block(body)
            ),
            Statement::For {
                label,
                init,
                condition,
                step,
                body,
                ..
            } => format!(
                "{}for ({}; {}; {}) {{ {} }}",
                show_label(label),
                init.as_ref()
                    .map_or_else(String::new, |c| show_statement(c)),
                condition.as_ref().map_or_else(String::new, show),
                step.as_ref()
                    .map_or_else(String::new, |c| show_statement(c)),
                show_block(body)
            ),
            Statement::Foreach {
                label,
                name,
                collection,
                body,
                ..
            } => format!(
                "{}foreach (${name} in {}) {{ {} }}",
                show_label(label),
                show(collection),
                show_block(body)
            ),
            Statement::Do {
                label,
                body,
                condition,
                until,
                ..
            } => format!(
                "{}do {{ {} }} {} ({})",
                show_label(label),
                show_block(body),
                if *until { "until" } else { "while" },
                show(condition)
            ),
            Statement::Try {
                body,
                catches,
                finally,
                ..
            } => {
                let mut text = format!("try {{ {} }}", show_block(body));
                for clause in catches {
                    text.push_str(" catch");
                    for (index, kind) in clause.types.iter().enumerate() {
                        text.push_str(if index == 0 { " [" } else { ", [" });
                        text.push_str(kind);
                        text.push(']');
                    }
                    text.push_str(&format!(" {{ {} }}", show_block(&clause.body)));
                }
                if let Some(finally) = finally {
                    text.push_str(&format!(" finally {{ {} }}", show_block(finally)));
                }
                text
            }
            Statement::Function {
                name,
                parameters,
                body,
                filter,
                ..
            } => format!(
                "{} {name}({}) {{ {} }}",
                if *filter { "filter" } else { "function" },
                parameters
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect::<Vec<_>>()
                    .join(","),
                show_block(body)
            ),
            Statement::Switch {
                condition, clauses, ..
            } => {
                let mut text = format!("switch ({}) {{", show(condition));
                for clause in clauses {
                    if let Some(label) = &clause.label {
                        text.push_str(&format!(
                            " {} {{ {} }}",
                            show(label),
                            show_block(&clause.body)
                        ));
                    } else {
                        text.push_str(&format!(" default {{ {} }}", show_block(&clause.body)));
                    }
                }
                text.push_str(" }");
                text
            }
            Statement::Pipeline { commands, .. } => commands
                .iter()
                .map(|command| match command {
                    PipelineCommand::Expression(expr) => show(expr),
                    PipelineCommand::Command { name, arguments } => {
                        let mut text = name.clone();
                        for argument in arguments {
                            match argument {
                                CommandArgument::Value(value) => {
                                    text.push(' ');
                                    text.push_str(&show(value));
                                }
                                CommandArgument::Bareword(word) => {
                                    text.push(' ');
                                    text.push_str(word);
                                }
                                CommandArgument::Parameter { name, value } => {
                                    text.push_str(&format!(" -{name}"));
                                    if let Some(value) = value {
                                        text.push(' ');
                                        text.push_str(&show(value));
                                    }
                                }
                            }
                        }
                        text
                    }
                })
                .collect::<Vec<_>>()
                .join(" | "),
            Statement::Command {
                name, arguments, ..
            } => {
                let mut text = name.clone();
                for argument in arguments {
                    match argument {
                        CommandArgument::Value(value) => {
                            text.push(' ');
                            text.push_str(&show(value));
                        }
                        CommandArgument::Bareword(word) => {
                            text.push(' ');
                            text.push_str(word);
                        }
                        CommandArgument::Parameter { name, value } => {
                            text.push_str(&format!(" -{name}"));
                            if let Some(value) = value {
                                text.push(' ');
                                text.push_str(&show(value));
                            }
                        }
                    }
                }
                text
            }
            Statement::AssignPipeline { name, commands, .. } => format!(
                "${name} = {}",
                commands
                    .iter()
                    .map(|command| match command {
                        PipelineCommand::Expression(expr) => show(expr),
                        PipelineCommand::Command { name, .. } => name.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
            Statement::MemberAssign {
                target,
                name,
                value,
                ..
            } => format!("{}.{name} = {}", show(target), show(value)),
            Statement::IndexAssign {
                target,
                index,
                value,
                ..
            } => format!("{}[{}] = {}", show(target), show(index), show(value)),
            Statement::Trap { body, .. } => format!("trap {{ {} }}", show_block(body)),
            Statement::Throw { value, .. } => format!("throw {}", show(value)),
            Statement::Return { value, .. } => match value {
                Some(value) => format!("return {}", show(value)),
                None => "return".to_owned(),
            },
            Statement::Break { label, .. } => match label {
                Some(label) => format!("break {label}"),
                None => "break".to_owned(),
            },
            Statement::Continue { label, .. } => match label {
                Some(label) => format!("continue {label}"),
                None => "continue".to_owned(),
            },
            Statement::Exit { code: None, .. } => "exit".to_owned(),
            Statement::Exit {
                code: Some(code), ..
            } => format!("exit {}", show(code)),
        }
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
            refused("class Foo { }"),
            "statement keywords (if, foreach, function, ...)"
        );
    }

    #[test]
    fn if_chains_elseif_and_else() {
        assert_eq!(
            shown("if ($a) { 1 } elseif ($b) { 2 } else { 3 }"),
            ["if $a { 1 } elseif $b { 2 } else { 3 }"]
        );
        assert_eq!(
            shown("if ($a) { 1 }\nelse { 2 }"),
            ["if $a { 1 } else { 2 }"]
        );
    }

    #[test]
    fn loops_take_a_label_and_a_block() {
        assert_eq!(
            shown(":outer foreach ($x in 1, 2) { $x }"),
            [":outer foreach ($x in [1,2]) { $x }"]
        );
        assert_eq!(shown("while ($a) { $a-- }"), ["while $a { ($a--) }"]);
        assert_eq!(
            shown("for ($i = 0; $i -lt 3; $i++) { $i }"),
            ["for ($i = 0; ($i -lt 3); ($i++)) { $i }"]
        );
        assert_eq!(shown("do { 1 } until ($a)"), ["do { 1 } until ($a)"]);
    }

    #[test]
    fn break_and_continue_take_an_optional_label() {
        assert_eq!(shown("while ($a) { break }"), ["while $a { break }"]);
        assert_eq!(
            shown(":outer while ($a) { continue outer }"),
            [":outer while $a { continue outer }"]
        );
    }

    #[test]
    fn try_takes_catch_types_and_finally() {
        assert_eq!(
            shown("try { 1 } catch [A.B] { 2 } finally { 3 }"),
            ["try { 1 } catch [A.B] { 2 } finally { 3 }"]
        );
        assert_eq!(refused("try { 1 }"), SYNTAX_ERRORS);
    }

    #[test]
    fn caps_the_size_of_one_expression() {
        let long = vec!["1"; 600].join(" + ");
        assert_eq!(refused(&long), "expressions with more than 1000 nodes");
        assert!(parse(&vec!["1"; 400].join(" + ")).is_ok());
    }
}
