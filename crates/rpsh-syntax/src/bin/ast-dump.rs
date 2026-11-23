//! Prints the same normalized S-expression as scripts/ast-dump.ps1.

use std::io::Read;

use rpsh_syntax::{BinaryOp, Expr, ExprKind, Statement, StringPart, UnaryOp, parse};

fn main() {
    let mut source = String::new();
    std::io::stdin()
        .read_to_string(&mut source)
        .expect("ast-dump: stdin is not UTF-8");
    let ast = parse(&source).expect("ast-dump: parse");
    let expression = match &ast.statements[0] {
        Statement::Expression(expr) => expr,
        _ => panic!("ast-dump: expected an expression"),
    };
    println!("{}", format_expr(expression));
}

fn format_expr(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Int32(value) => format!("(int {value})"),
        ExprKind::Number(value) => format!("(int {value})"),
        ExprKind::String(value) => format!("(str {})", json(value)),
        ExprKind::Variable(name) => format!("(var {name})"),
        ExprKind::Unary { op, operand } => {
            format!("(unary {} {})", unary(*op), format_expr(operand))
        }
        ExprKind::Binary { op, left, right } => format!(
            "(binary {} {} {})",
            binary(*op),
            format_expr(left),
            format_expr(right)
        ),
        ExprKind::Paren(inner) => format!("(paren {})", format_expr(inner)),
        ExprKind::Member { target, name } => format!("(member {} {name})", format_expr(target)),
        ExprKind::InvokeMember {
            target,
            name,
            arguments: _,
        } => format!("(invoke {} {name})", format_expr(target)),
        ExprKind::Array(items) => format!(
            "(array {})",
            items.iter().map(format_expr).collect::<Vec<_>>().join(" ")
        ),
        ExprKind::Index { target, index } => {
            format!("(index {} {})", format_expr(target), format_expr(index))
        }
        ExprKind::Ternary {
            condition,
            when_true,
            when_false,
        } => format!(
            "(ternary {} {} {})",
            format_expr(condition),
            format_expr(when_true),
            format_expr(when_false)
        ),
        ExprKind::Subexpression(_) => "(subexpr)".to_owned(),
        ExprKind::Cast {
            type_name: cast,
            value,
        } => format!("(cast {cast} {})", format_expr(value)),
        ExprKind::TypeLiteral(name) => format!("(type {name})"),
        ExprKind::Object(entries) => format!(
            "(object {})",
            entries
                .iter()
                .map(|(name, value)| format!("({name} {})", format_expr(value)))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        ExprKind::ScriptBlock(_) => "(scriptblock)".to_owned(),
        ExprKind::ExpandableString(parts) => format!(
            "(expand {})",
            parts
                .iter()
                .filter(|part| !matches!(part, StringPart::Literal(_)))
                .count()
        ),
        ExprKind::Coalesce { left, right } => {
            format!(
                "(binary Coalesce {} {})",
                format_expr(left),
                format_expr(right)
            )
        }
    }
}

fn unary(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Negate => "Negate",
        UnaryOp::Not => "Not",
        UnaryOp::Bnot => "Bnot",
        UnaryOp::Join => "Join",
        UnaryOp::Increment => "Increment",
        UnaryOp::Decrement => "Decrement",
        UnaryOp::PostIncrement => "PostIncrement",
        UnaryOp::PostDecrement => "PostDecrement",
    }
}

fn binary(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "Add",
        BinaryOp::Subtract => "Subtract",
        BinaryOp::Multiply => "Multiply",
        BinaryOp::Divide => "Divide",
        BinaryOp::Remainder => "Remainder",
        BinaryOp::Range => "Range",
        BinaryOp::Join => "Join",
        BinaryOp::Format => "Format",
        BinaryOp::Eq => "Eq",
        BinaryOp::Ne => "Ne",
        BinaryOp::Ge => "Ge",
        BinaryOp::Gt => "Gt",
        BinaryOp::Lt => "Lt",
        BinaryOp::Le => "Le",
        BinaryOp::CEq => "CEq",
        BinaryOp::CNe => "CNe",
        BinaryOp::CGe => "CGe",
        BinaryOp::CGt => "CGt",
        BinaryOp::CLt => "CLt",
        BinaryOp::CLe => "CLe",
        BinaryOp::Like => "Like",
        BinaryOp::NotLike => "NotLike",
        BinaryOp::CLike => "CLike",
        BinaryOp::CNotLike => "CNotLike",
        BinaryOp::Contains => "Contains",
        BinaryOp::NotContains => "NotContains",
        BinaryOp::CContains => "CContains",
        BinaryOp::CNotContains => "CNotContains",
        BinaryOp::In => "In",
        BinaryOp::NotIn => "NotIn",
        BinaryOp::CIn => "CIn",
        BinaryOp::CNotIn => "CNotIn",
        BinaryOp::Shl => "Shl",
        BinaryOp::Shr => "Shr",
        BinaryOp::Band => "Band",
        BinaryOp::Bor => "Bor",
        BinaryOp::Bxor => "Bxor",
        BinaryOp::And => "And",
        BinaryOp::Or => "Or",
        BinaryOp::Xor => "Xor",
        BinaryOp::Is => "Is",
        BinaryOp::IsNot => "IsNot",
        BinaryOp::As => "As",
        BinaryOp::Match => "Match",
        BinaryOp::NotMatch => "NotMatch",
        BinaryOp::CMatch => "CMatch",
        BinaryOp::CNotMatch => "CNotMatch",
        BinaryOp::Replace => "Replace",
        BinaryOp::CReplace => "CReplace",
        BinaryOp::Split => "Split",
        BinaryOp::CSplit => "CSplit",
    }
}

fn json(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}
