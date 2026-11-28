//! Queries over the AST of one scope. Nested functions and script blocks have separate locals.
use crate::{
    AssignmentValue, CommandArgument, Expr, ExprKind, PipelineChain, PipelineCommand, Redirection,
    ScriptBlockAst, Statement, StringPart,
};

impl ScriptBlockAst {
    pub fn uses_pipeline_input(&self) -> bool {
        Query::PipelineInput.block(self)
    }
    /// Dot sourcing and variable-mutating commands require dictionary-backed locals.
    pub fn uses_dynamic_locals(&self) -> bool {
        Query::DynamicLocals.block(self)
    }
}

#[derive(Clone, Copy)]
enum Query {
    PipelineInput,
    DynamicLocals,
}
impl Query {
    fn block(self, ast: &ScriptBlockAst) -> bool {
        self.statements(&ast.statements)
            || ast
                .named_blocks
                .iter()
                .any(|b| self.statements(&b.statements))
    }
    fn dynamic_command(self, name: &str) -> bool {
        if matches!(self, Self::PipelineInput) {
            return false;
        }
        let short = match name.split_once('\\') {
            Some((module, n)) if module.eq_ignore_ascii_case("Microsoft.PowerShell.Utility") => n,
            Some(_) => return false,
            None => name,
        };
        [
            "New-Variable",
            "Remove-Variable",
            "Set-Variable",
            "Set-PSBreakpoint",
            "nv",
            "rv",
            "sbp",
            "sv",
            "set",
        ]
        .iter()
        .any(|n| n.eq_ignore_ascii_case(short))
    }
    fn input(self, name: &str) -> bool {
        if matches!(self, Self::DynamicLocals) {
            return false;
        }
        let unqualified = if let Some((scope, name)) = name.split_once(':') {
            if !["local", "private", "script", "global", "variable"]
                .iter()
                .any(|s| scope.eq_ignore_ascii_case(s))
            {
                return false;
            }
            name
        } else {
            name
        };
        unqualified.eq_ignore_ascii_case("input")
    }

    fn statements(self, list: &[Statement]) -> bool {
        list.iter().any(|item| self.statement(item))
    }

    fn statement(self, s: &Statement) -> bool {
        use Statement::*;
        match s {
            Expression(e) | StaticMemberAssign { value: e, .. } => self.expr(e),
            Assign { name, value, .. }
            | AppendAssign { name, value, .. }
            | CompoundAssign { name, value, .. }
            | CoalesceAssign { name, value, .. } => self.input(name) || self.expr(value),
            AssignPipeline { name, commands, .. } => {
                self.input(name) || commands.iter().any(|item| self.stage(item))
            }
            AssignChain { name, chain: c, .. } => self.input(name) || self.chain(c),
            MultiAssign { targets, value, .. } => {
                targets.iter().any(|item| self.expr(item))
                    || match value {
                        AssignmentValue::Expression(v) => self.expr(v),
                        AssignmentValue::Pipeline(p) => p.iter().any(|item| self.stage(item)),
                        AssignmentValue::Chain(c) => self.chain(c),
                    }
            }
            MemberAssign { target, value, .. } => self.expr(target) || self.expr(value),
            IndexAssign {
                target,
                index,
                value,
                ..
            } => self.expr(target) || self.expr(index) || self.expr(value),
            Command {
                name, arguments, ..
            } => self.dynamic_command(name) || arguments.iter().any(|item| self.argument(item)),
            Pipeline { commands, .. } => commands.iter().any(|item| self.stage(item)),
            PipelineChain { chain: c, .. } => self.chain(c),
            Switch {
                condition, clauses, ..
            } => {
                self.expr(condition)
                    || clauses.iter().any(|clause| {
                        clause.label.as_ref().is_some_and(|item| self.expr(item))
                            || self.statements(&clause.body)
                    })
            }
            If {
                condition,
                then,
                elseif,
                otherwise,
                ..
            } => {
                self.expr(condition)
                    || self.statements(then)
                    || elseif
                        .iter()
                        .any(|(c, b)| self.expr(c) || self.statements(b))
                    || self.statements(otherwise)
            }
            While {
                condition, body, ..
            }
            | Do {
                condition, body, ..
            } => self.expr(condition) || self.statements(body),
            For {
                init,
                condition,
                step,
                body,
                ..
            } => {
                init.as_deref().is_some_and(|item| self.statement(item))
                    || condition.as_ref().is_some_and(|item| self.expr(item))
                    || step.as_deref().is_some_and(|item| self.statement(item))
                    || self.statements(body)
            }
            Foreach {
                name,
                collection,
                body,
                ..
            } => self.input(name) || self.expr(collection) || self.statements(body),
            Try {
                body,
                catches,
                finally,
                ..
            } => {
                self.statements(body)
                    || catches.iter().any(|c| self.statements(&c.body))
                    || finally.as_deref().is_some_and(|item| self.statements(item))
            }
            Trap { body, .. } => matches!(self, Self::PipelineInput) && self.statements(body),
            Param { parameters, .. } => parameters.iter().any(|p| {
                self.input(&p.name) || p.default.as_ref().is_some_and(|item| self.expr(item))
            }),
            Throw { value, .. } | Return { value, .. } | Exit { code: value, .. } => {
                value.as_ref().is_some_and(|item| self.expr(item))
            }
            Function { .. } | Class { .. } | Enum { .. } | Break { .. } | Continue { .. } => false,
        }
    }

    fn chain(self, c: &PipelineChain) -> bool {
        c.first.iter().any(|item| self.stage(item))
            || c.rest
                .iter()
                .any(|(_, stages)| stages.iter().any(|item| self.stage(item)))
    }

    fn stage(self, s: &PipelineCommand) -> bool {
        match s {
            PipelineCommand::Expression(e) => self.expr(e),
            PipelineCommand::Command {
                name, arguments, ..
            } => self.dynamic_command(name) || arguments.iter().any(|item| self.argument(item)),
            PipelineCommand::Call {
                operator,
                target,
                arguments,
            } => {
                (matches!(self, Self::DynamicLocals) && *operator == crate::InvocationOperator::Dot)
                    || matches!(&target.kind, ExprKind::String(n)|ExprKind::Bareword(n) if self.dynamic_command(n))
                    || self.expr(target)
                    || arguments.iter().any(|item| self.argument(item))
            }
            PipelineCommand::Redirected {
                command,
                redirections,
            } => {
                self.stage(command)
                    || redirections
                        .iter()
                        .any(|r| matches!(r, Redirection::File{path,..} if self.expr(path)))
            }
        }
    }

    fn argument(self, a: &CommandArgument) -> bool {
        match a {
            CommandArgument::Value(e) => self.expr(e),
            CommandArgument::Splat(name) => self.input(name),
            CommandArgument::Parameter { value, .. } => {
                value.as_ref().is_some_and(|item| self.expr(item))
            }
            CommandArgument::Bareword(_) => false,
        }
    }

    fn expr(self, e: &Expr) -> bool {
        use ExprKind::*;
        match &e.kind {
            Variable(name) => self.input(name),
            Unary { operand, .. }
            | Paren(operand)
            | Subexpression(operand)
            | Member {
                target: operand, ..
            }
            | Cast { value: operand, .. } => self.expr(operand),
            Binary { left, right, .. }
            | Coalesce { left, right }
            | Index {
                target: left,
                index: right,
            } => self.expr(left) || self.expr(right),
            ExpandableString(parts) | ExpandableBareword(parts) => parts.iter().any(|p| match p {
                StringPart::Variable(name) => self.input(name),
                StringPart::Subexpression(e) => self.expr(e),
                StringPart::Literal(_) => false,
            }),
            ArrayExpression(body) | Group(body) => self.statements(body),
            Array(items)
            | CommandArray {
                elements: items, ..
            }
            | InvokeStatic {
                arguments: items, ..
            } => items.iter().any(|item| self.expr(item)),
            InvokeMember {
                target, arguments, ..
            } => self.expr(target) || arguments.iter().any(|item| self.expr(item)),
            Object(items) => items.iter().any(|(_, v)| self.expr(v)),
            Ternary {
                condition,
                when_true,
                when_false,
            } => self.expr(condition) || self.expr(when_true) || self.expr(when_false),
            Int32(_)
            | Number(_)
            | CommandNumber { .. }
            | String(_)
            | Bareword(_)
            | TypeLiteral(_)
            | StaticMember { .. }
            | ScriptBlock { .. } => false,
        }
    }
}
