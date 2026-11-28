//! ConsoleHost reads redirected input when this script's AST uses $input.
//! Nested function and script block bodies have their own automatic variables.
use crate::{
    AssignmentValue, CommandArgument, Expr, ExprKind, PipelineChain, PipelineCommand, Redirection,
    ScriptBlockAst, Statement, StringPart,
};

impl ScriptBlockAst {
    pub fn uses_pipeline_input(&self) -> bool {
        statements(&self.statements) || self.named_blocks.iter().any(|b| statements(&b.statements))
    }
}

fn input(name: &str) -> bool {
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

fn statements(list: &[Statement]) -> bool {
    list.iter().any(statement)
}

fn statement(s: &Statement) -> bool {
    use Statement::*;
    match s {
        Expression(e) | StaticMemberAssign { value: e, .. } => expr(e),
        Assign { name, value, .. }
        | AppendAssign { name, value, .. }
        | CompoundAssign { name, value, .. }
        | CoalesceAssign { name, value, .. } => input(name) || expr(value),
        AssignPipeline { name, commands, .. } => input(name) || commands.iter().any(stage),
        AssignChain { name, chain: c, .. } => input(name) || chain(c),
        MultiAssign { targets, value, .. } => {
            targets.iter().any(expr)
                || match value {
                    AssignmentValue::Expression(v) => expr(v),
                    AssignmentValue::Pipeline(p) => p.iter().any(stage),
                    AssignmentValue::Chain(c) => chain(c),
                }
        }
        MemberAssign { target, value, .. } => expr(target) || expr(value),
        IndexAssign {
            target,
            index,
            value,
            ..
        } => expr(target) || expr(index) || expr(value),
        Command { arguments, .. } => arguments.iter().any(argument),
        Pipeline { commands, .. } => commands.iter().any(stage),
        PipelineChain { chain: c, .. } => chain(c),
        Switch {
            condition, clauses, ..
        } => {
            expr(condition)
                || clauses.iter().any(|clause| {
                    clause.label.as_ref().is_some_and(expr) || statements(&clause.body)
                })
        }
        If {
            condition,
            then,
            elseif,
            otherwise,
            ..
        } => {
            expr(condition)
                || statements(then)
                || elseif.iter().any(|(c, b)| expr(c) || statements(b))
                || statements(otherwise)
        }
        While {
            condition, body, ..
        }
        | Do {
            condition, body, ..
        } => expr(condition) || statements(body),
        For {
            init,
            condition,
            step,
            body,
            ..
        } => {
            init.as_deref().is_some_and(statement)
                || condition.as_ref().is_some_and(expr)
                || step.as_deref().is_some_and(statement)
                || statements(body)
        }
        Foreach {
            name,
            collection,
            body,
            ..
        } => input(name) || expr(collection) || statements(body),
        Try {
            body,
            catches,
            finally,
            ..
        } => {
            statements(body)
                || catches.iter().any(|c| statements(&c.body))
                || finally.as_deref().is_some_and(statements)
        }
        Trap { body, .. } => statements(body),
        Param { parameters, .. } => parameters
            .iter()
            .any(|p| input(&p.name) || p.default.as_ref().is_some_and(expr)),
        Throw { value, .. } | Return { value, .. } | Exit { code: value, .. } => {
            value.as_ref().is_some_and(expr)
        }
        Function { .. } | Class { .. } | Enum { .. } | Break { .. } | Continue { .. } => false,
    }
}

fn chain(c: &PipelineChain) -> bool {
    c.first.iter().any(stage) || c.rest.iter().any(|(_, stages)| stages.iter().any(stage))
}

fn stage(s: &PipelineCommand) -> bool {
    match s {
        PipelineCommand::Expression(e) => expr(e),
        PipelineCommand::Command { arguments, .. } => arguments.iter().any(argument),
        PipelineCommand::Call {
            target, arguments, ..
        } => expr(target) || arguments.iter().any(argument),
        PipelineCommand::Redirected {
            command,
            redirections,
        } => {
            stage(command)
                || redirections
                    .iter()
                    .any(|r| matches!(r, Redirection::File{path,..} if expr(path)))
        }
    }
}

fn argument(a: &CommandArgument) -> bool {
    match a {
        CommandArgument::Value(e) => expr(e),
        CommandArgument::Splat(name) => input(name),
        CommandArgument::Parameter { value, .. } => value.as_ref().is_some_and(expr),
        CommandArgument::Bareword(_) => false,
    }
}

fn expr(e: &Expr) -> bool {
    use ExprKind::*;
    match &e.kind {
        Variable(name) => input(name),
        Unary { operand, .. }
        | Paren(operand)
        | Subexpression(operand)
        | Member {
            target: operand, ..
        }
        | Cast { value: operand, .. } => expr(operand),
        Binary { left, right, .. }
        | Coalesce { left, right }
        | Index {
            target: left,
            index: right,
        } => expr(left) || expr(right),
        ExpandableString(parts) | ExpandableBareword(parts) => parts.iter().any(|p| match p {
            StringPart::Variable(name) => input(name),
            StringPart::Subexpression(e) => expr(e),
            StringPart::Literal(_) => false,
        }),
        ArrayExpression(body) | Group(body) => statements(body),
        Array(items)
        | CommandArray {
            elements: items, ..
        }
        | InvokeStatic {
            arguments: items, ..
        } => items.iter().any(expr),
        InvokeMember {
            target, arguments, ..
        } => expr(target) || arguments.iter().any(expr),
        Object(items) => items.iter().any(|(_, v)| expr(v)),
        Ternary {
            condition,
            when_true,
            when_false,
        } => expr(condition) || expr(when_true) || expr(when_false),
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
