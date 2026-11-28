//! Evaluate expressions against the active session. Nested expressions can run
//! commands and methods with the same scope, output and error rules as a statement.
use super::{
    Engine, EngineError, Variables, add_values, as_operator, bitwise_values, cast_in_scope,
    compare_values, expansion_text, get_type, increment, index_value, invoke_member, invoke_static,
    join_values, lookup, mark_hashtable, member, negate_value, number_value, object_class,
    order_hashtable, regex_match, regex_replace, regex_split, static_member, subtract_values,
    type_full_name, type_is,
};
use rpsh_runtime::{PsString, RuntimeError, Value, ops};
use rpsh_syntax::{BinaryOp, Expr, ExprKind, StringPart, UnaryOp};

impl Engine {
    pub(super) fn eval_expression(&mut self, expr: &Expr) -> Result<Value, EngineError> {
        let result = match &expr.kind {
            ExprKind::Int32(n) => Value::Int32(*n),
            ExprKind::Number(n) => number_value(n),
            ExprKind::CommandNumber { value, text } => {
                Value::command_number(number_value(value), text)
            }
            ExprKind::String(s) | ExprKind::Bareword(s) => Value::String(s.as_str().into()),
            ExprKind::Variable(name) => {
                lookup(name, &self.variables, !self.last_failed, &self.functions)
                    .unwrap_or(Value::Null)
            }
            ExprKind::Paren(inner) | ExprKind::Subexpression(inner) => {
                return self.eval_expression(inner);
            }
            ExprKind::ExpandableString(parts) | ExprKind::ExpandableBareword(parts) => {
                let mut text = Vec::new();
                for part in parts {
                    let value = match part {
                        StringPart::Literal(s) => {
                            text.extend(s.encode_utf16());
                            continue;
                        }
                        StringPart::Variable(name) => {
                            let Some(value) =
                                lookup(name, &self.variables, !self.last_failed, &self.functions)
                            else {
                                continue;
                            };
                            value
                        }
                        StringPart::Subexpression(e) => self.eval_expression(e)?,
                    };
                    text.extend(expansion_text(&value, &self.variables).units());
                }
                Value::String(PsString::from_units(text))
            }
            ExprKind::Group(statements) => return self.eval_group(statements),
            ExprKind::ArrayExpression(statements) => {
                Value::array(self.collect_statements(statements)?)
            }
            ExprKind::Array(elements) | ExprKind::CommandArray { elements, .. } => {
                Value::array(self.eval_operands(elements)?)
            }
            ExprKind::TypeLiteral(name) => {
                let ty = if self.classes.contains_key(&name.to_ascii_lowercase()) {
                    name.clone()
                } else {
                    type_full_name(name)
                };
                Value::Type(ty.into())
            }
            ExprKind::ScriptBlock { source, .. } => Value::ScriptBlock(source.as_str().into()),
            ExprKind::Coalesce { left, right } => {
                let value = self.eval_expression(left)?;
                if value.is_null_like() {
                    return self.eval_expression(right);
                }
                value
            }
            ExprKind::Ternary {
                condition,
                when_true,
                when_false,
            } => {
                let test = self.eval_expression(condition)?;
                return self.eval_expression(if ops::is_true(&test) {
                    when_true
                } else {
                    when_false
                });
            }
            ExprKind::Object(entries) => return self.eval_dictionary(entries, "hashtable"),
            ExprKind::Cast { type_name, value } => {
                if let ExprKind::Object(entries) = &value.kind {
                    let ty = type_name.to_ascii_lowercase();
                    if matches!(
                        ty.as_str(),
                        "ordered"
                            | "pscustomobject"
                            | "system.management.automation.pscustomobject"
                    ) {
                        return self.eval_dictionary(entries, &ty);
                    }
                }
                let value = self.eval_expression(value)?;
                cast_in_scope(type_name, value, &self.variables)?
            }
            ExprKind::Index { target, index } => {
                let target = self.eval_expression(target)?;
                let index = self.eval_expression(index)?;
                index_value(&target, &index)?
            }
            ExprKind::Member { target, name } => member(&self.eval_expression(target)?, name)?,
            ExprKind::StaticMember { type_name, name } => {
                static_member(type_name, name, &self.variables)?
            }
            ExprKind::InvokeStatic {
                type_name,
                name,
                arguments,
            } => {
                let values = self.eval_operands(arguments)?;
                if name.eq_ignore_ascii_case("new")
                    && self.classes.contains_key(&type_name.to_ascii_lowercase())
                {
                    return self.construct_class(type_name, &values);
                }
                invoke_static(type_name, name, &values, &mut self.variables)?
            }
            ExprKind::InvokeMember {
                target,
                name,
                arguments,
            } => {
                let target = self.eval_expression(target)?;
                let values = self.eval_operands(arguments)?;
                if name.eq_ignore_ascii_case("GetType") && values.is_empty() {
                    return Ok(get_type(&target));
                }
                if let Value::Object(fields) = &target
                    && let Some(class) = object_class(fields)
                    && self.classes.contains_key(&class.to_ascii_lowercase())
                {
                    return self.call_method(&class, name, &target, &values);
                }
                invoke_member(&target, name, &values)?
            }
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Increment
                | UnaryOp::Decrement
                | UnaryOp::PostIncrement
                | UnaryOp::PostDecrement => increment(
                    operand,
                    &mut self.variables,
                    !self.last_failed,
                    *op,
                    &self.functions,
                )?,
                UnaryOp::Negate => negate_value(&self.eval_expression(operand)?)?,
                UnaryOp::Bnot => ops::bnot(&self.eval_expression(operand)?)?,
                UnaryOp::Not => ops::not(&self.eval_expression(operand)?),
                UnaryOp::Join => join_values(
                    &self.eval_expression(operand)?,
                    &Value::String(Default::default()),
                    &self.variables,
                )?,
            },
            ExprKind::Binary { op, left, right } => {
                let left = self.eval_expression(left)?;
                if *op == BinaryOp::And && !ops::is_true(&left) {
                    return Ok(Value::Bool(false));
                }
                if *op == BinaryOp::Or && ops::is_true(&left) {
                    return Ok(Value::Bool(true));
                }
                let right = self.eval_expression(right)?;
                binary(*op, &left, &right, &mut self.variables)?
            }
        };
        Ok(result)
    }

    fn eval_operands(&mut self, expressions: &[Expr]) -> Result<Vec<Value>, EngineError> {
        expressions
            .iter()
            .map(|e| self.eval_expression(e))
            .collect()
    }

    fn eval_dictionary(
        &mut self,
        entries: &[(String, Expr)],
        ty: &str,
    ) -> Result<Value, EngineError> {
        let mut fields = Vec::with_capacity(entries.len());
        for (name, expression) in entries {
            fields.push((name.clone(), self.eval_expression(expression)?));
        }
        Ok(match ty {
            "hashtable" => {
                if !fields.is_empty() && fields.iter().all(|(name, _)| name.parse::<i64>().is_ok())
                {
                    fields = order_hashtable(fields);
                }
                mark_hashtable(Value::object(fields), "System.Collections.Hashtable")
            }
            "ordered" => mark_hashtable(
                Value::object(fields),
                "System.Collections.Specialized.OrderedDictionary",
            ),
            _ => Value::object(fields),
        })
    }
}

fn binary(
    op: BinaryOp,
    left: &Value,
    right: &Value,
    variables: &mut Variables,
) -> Result<Value, RuntimeError> {
    match op {
        BinaryOp::Add => add_values(left, right),
        BinaryOp::Subtract => subtract_values(left, right),
        BinaryOp::Multiply => ops::multiply(left, right),
        BinaryOp::Divide => ops::divide(left, right),
        BinaryOp::Remainder => ops::remainder(left, right),
        BinaryOp::Range => ops::range(left, right),
        BinaryOp::Join => join_values(left, right, variables),
        BinaryOp::Format => ops::format(left, right),
        BinaryOp::Eq => compare_values(left, right, ops::Comparison::Eq, false),
        BinaryOp::Ne => compare_values(left, right, ops::Comparison::Ne, false),
        BinaryOp::Ge => compare_values(left, right, ops::Comparison::Ge, false),
        BinaryOp::Gt => compare_values(left, right, ops::Comparison::Gt, false),
        BinaryOp::Lt => compare_values(left, right, ops::Comparison::Lt, false),
        BinaryOp::Le => compare_values(left, right, ops::Comparison::Le, false),
        BinaryOp::CEq => compare_values(left, right, ops::Comparison::Eq, true),
        BinaryOp::CNe => compare_values(left, right, ops::Comparison::Ne, true),
        BinaryOp::CGe => compare_values(left, right, ops::Comparison::Ge, true),
        BinaryOp::CGt => compare_values(left, right, ops::Comparison::Gt, true),
        BinaryOp::CLt => compare_values(left, right, ops::Comparison::Lt, true),
        BinaryOp::CLe => compare_values(left, right, ops::Comparison::Le, true),
        BinaryOp::Like => compare_values(left, right, ops::Comparison::Like, false),
        BinaryOp::NotLike => compare_values(left, right, ops::Comparison::NotLike, false),
        BinaryOp::CLike => compare_values(left, right, ops::Comparison::Like, true),
        BinaryOp::CNotLike => compare_values(left, right, ops::Comparison::NotLike, true),
        BinaryOp::Contains => ops::contains(left, right, false),
        BinaryOp::NotContains => ops::contains(left, right, false)
            .map(|value| Value::Bool(!matches!(value, Value::Bool(true)))),
        BinaryOp::CContains => ops::contains(left, right, true),
        BinaryOp::CNotContains => ops::contains(left, right, true)
            .map(|value| Value::Bool(!matches!(value, Value::Bool(true)))),
        BinaryOp::In => ops::contains(right, left, false),
        BinaryOp::NotIn => ops::contains(right, left, false)
            .map(|value| Value::Bool(!matches!(value, Value::Bool(true)))),
        BinaryOp::CIn => ops::contains(right, left, true),
        BinaryOp::CNotIn => ops::contains(right, left, true)
            .map(|value| Value::Bool(!matches!(value, Value::Bool(true)))),
        BinaryOp::Shl => ops::shift(left, right, true),
        BinaryOp::Shr => ops::shift(left, right, false),
        BinaryOp::Band => bitwise_values(left, right, "band"),
        BinaryOp::Bor => bitwise_values(left, right, "bor"),
        BinaryOp::Bxor => bitwise_values(left, right, "bxor"),
        BinaryOp::And => ops::logical(left, right, "and"),
        BinaryOp::Or => ops::logical(left, right, "or"),
        BinaryOp::Xor => ops::logical(left, right, "xor"),
        BinaryOp::Is => type_is(left, right, false),
        BinaryOp::IsNot => type_is(left, right, true),
        BinaryOp::As => as_operator(left, right),
        BinaryOp::Match => regex_match(left, right, false, false, variables),
        BinaryOp::NotMatch => regex_match(left, right, false, true, variables),
        BinaryOp::CMatch => regex_match(left, right, true, false, variables),
        BinaryOp::CNotMatch => regex_match(left, right, true, true, variables),
        BinaryOp::Replace => regex_replace(left, right, false),
        BinaryOp::CReplace => regex_replace(left, right, true),
        BinaryOp::Split => regex_split(left, right, false),
        BinaryOp::CSplit => regex_split(left, right, true),
    }
}
