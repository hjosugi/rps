#!/usr/bin/env pwsh
# Prints a normalized S-expression for the first statement in $Source.
param([Parameter(Mandatory)] [string] $Source)
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseInput($Source, [ref] $tokens, [ref] $errors)
$binary = @{
    Plus='Add'; Minus='Subtract'; Multiply='Multiply'; Divide='Divide'; Rem='Remainder';
    DotDot='Range'; Join='Join'; Ieq='Eq'; Ine='Ne'; Ige='Ge'; Igt='Gt'; Ilt='Lt'; Ile='Le';
    Ceq='CEq'; Cne='CNe'; Cge='CGe'; Cgt='CGt'; Clt='CLt'; Cle='CLe';
    Ilike='Like'; Inotlike='NotLike'; Clike='CLike'; Cnotlike='CNotLike';
    Icontains='Contains'; Inotcontains='NotContains'; Ccontains='CContains'; Cnotcontains='CNotContains';
    Iin='In'; Inotin='NotIn'; Cin='CIn'; Cnotin='CNotIn';
    Shl='Shl'; Shr='Shr'; Band='Band'; Bor='Bor'; Bxor='Bxor'; And='And'; Or='Or'; Xor='Xor';
    Is='Is'; IsNot='IsNot'; As='As'; Imatch='Match'; Inotmatch='NotMatch';
    Cmatch='CMatch'; Cnotmatch='CNotMatch'; Ireplace='Replace'; Creplace='CReplace';
    Isplit='Split'; Csplit='CSplit'; QuestionQuestion='Coalesce';
}
$unary = @{ Minus='Negate'; Not='Not'; Exclaim='Not'; Bnot='Bnot'; Join='Join' }
function Format-Value($v) {
    if ($v -is [System.Management.Automation.Language.PipelineAst]) {
        return Format-Expr $v.PipelineElements[0].Expression
    }
    return Format-Expr $v
}
function Format-Expr($e) {
    $name = $e.GetType().Name
    switch ($name) {
        'ConstantExpressionAst' {
            if ($e.Value -is [string]) { return "(str $(ConvertTo-Json $e.Value -Compress))" }
            return "(int $($e.Value))"
        }
        'StringConstantExpressionAst' { return "(str $(ConvertTo-Json $e.Value -Compress))" }
        'VariableExpressionAst' { return "(var $($e.VariablePath.UserPath))" }
        'UnaryExpressionAst' { return "(unary $($unary[$e.TokenKind.ToString()]) $(Format-Expr $e.Child))" }
        'BinaryExpressionAst' { return "(binary $($binary[$e.Operator.ToString()]) $(Format-Expr $e.Left) $(Format-Expr $e.Right))" }
        'ParenExpressionAst' { return "(paren $(Format-Expr $e.Pipeline.PipelineElements[0].Expression))" }
        'MemberExpressionAst' {
            if ($e.Static) { return "(static $($e.Expression.TypeName.FullName) $($e.Member.Value))" }
            return "(member $(Format-Expr $e.Expression) $($e.Member.Value))"
        }
        'InvokeMemberExpressionAst' { return "(invoke $(Format-Expr $e.Expression) $($e.Member.Value))" }
        'ArrayLiteralAst' { return "(array " + (($e.Elements | ForEach-Object { Format-Expr $_ }) -join ' ') + ")" }
        'IndexExpressionAst' { return "(index $(Format-Expr $e.Target) $(Format-Expr $e.Index))" }
        'TernaryExpressionAst' { return "(ternary $(Format-Expr $e.Condition) $(Format-Expr $e.IfTrue) $(Format-Expr $e.IfFalse))" }
        'SubExpressionAst' { return '(subexpr)' }
        'ConvertExpressionAst' { return "(cast $($e.Type.TypeName.FullName) $(Format-Expr $e.Child))" }
        'TypeExpressionAst' { return "(type $($e.TypeName.FullName))" }
        'HashtableAst' {
            $entries = foreach ($pair in $e.KeyValuePairs) {
                "($($pair.Item1.Value) $(Format-Value $pair.Item2))"
            }
            return "(object " + ($entries -join ' ') + ")"
        }
        'ScriptBlockExpressionAst' { return '(scriptblock)' }
        default { return "?$name" }
    }
}
function Format-Pipeline($pipeline) {
    if ($pipeline -is [System.Management.Automation.Language.PipelineChainAst]) {
        $left = Format-Pipeline $pipeline.LhsPipelineChain
        $right = Format-Pipeline $pipeline.RhsPipeline
        return "(chain $($pipeline.Operator) $left $right)"
    }
    $stages = foreach ($stage in $pipeline.PipelineElements) {
        if ($stage -is [System.Management.Automation.Language.CommandExpressionAst]) {
            $text = "(command-expression $(Format-Expr $stage.Expression))"
        } else {
            $elements = foreach ($element in $stage.CommandElements) {
                if ($element -is [System.Management.Automation.Language.CommandParameterAst]) {
                    "(parameter $($element.ParameterName))"
                    if ($null -ne $element.Argument) { Format-Expr $element.Argument }
                } else {
                    Format-Expr $element
                }
            }
            $text = "(command $($elements -join ' '))"
        }
        if ($stage.Redirections.Count) {
            $redirects = foreach ($r in $stage.Redirections) {
                if ($r -is [System.Management.Automation.Language.FileRedirectionAst]) {
                    "(file $($r.FromStream) $($r.Append.ToString().ToLowerInvariant()) $(Format-Expr $r.Location))"
                } else {
                    "(merge $($r.FromStream) $($r.ToStream))"
                }
            }
            "(redirect $text $($redirects -join ' '))"
        } else {
            $text
        }
    }
    "(pipeline $($stages -join ' '))"
}

$statement = $ast.EndBlock.Statements[0]
if ($statement -is [System.Management.Automation.Language.PipelineChainAst]) {
    Format-Pipeline $statement
} elseif ($statement -is [System.Management.Automation.Language.PipelineAst] -and
    ($statement.PipelineElements.Count -gt 1 -or $statement.PipelineElements[0].Redirections.Count)) {
    Format-Pipeline $statement
} elseif ($statement -is [System.Management.Automation.Language.AssignmentStatementAst]) {
    $right = $statement.Right
    if ($right -is [System.Management.Automation.Language.CommandExpressionAst]) {
        $rhs = "(command-expression $(Format-Expr $right.Expression))"
    } else {
        $rhs = Format-Pipeline $right
    }
    "(assign $($statement.Operator) $(Format-Expr $statement.Left) $rhs)"
} else {
    Format-Expr $statement.PipelineElements[0].Expression
}
