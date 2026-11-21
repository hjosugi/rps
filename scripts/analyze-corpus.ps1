#!/usr/bin/env pwsh
# Counts what a directory of .ps1 files uses, with the reference parser.
# Writes JSON: file count, parse errors, AST nodes, commands, types, members,
# operators. Used by scripts/analyze-corpus.py, which adds rpsh's refusals and
# writes docs/08_corpus.md.
param(
    [Parameter(Mandatory)] [string] $Root,
    [Parameter(Mandatory)] [string] $Out
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$nodes = @{}
$commands = @{}
$types = @{}
$members = @{}
$operators = @{}
$files = @(Get-ChildItem -Path $Root -Recurse -File -Filter '*.ps1')
$parseErrors = 0

foreach ($file in $files) {
    $tokens = $null
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref] $tokens, [ref] $errors)
    if ($errors.Count -gt 0) { $parseErrors++ }
    foreach ($node in $ast.FindAll({ $true }, $true)) {
        $type = $node.GetType().Name
        $nodes[$type] = 1 + ($nodes[$type] ?? 0)
        if ($node -is [System.Management.Automation.Language.CommandAst]) {
            $name = $node.GetCommandName()
            if ($name) { $commands[$name] = 1 + ($commands[$name] ?? 0) }
        } elseif ($node -is [System.Management.Automation.Language.TypeExpressionAst]) {
            $name = $node.TypeName.FullName
            $types[$name] = 1 + ($types[$name] ?? 0)
        } elseif ($node -is [System.Management.Automation.Language.MemberExpressionAst]) {
            $name = $node.Member.Extent.Text
            $members[$name] = 1 + ($members[$name] ?? 0)
        } elseif ($node -is [System.Management.Automation.Language.BinaryExpressionAst]) {
            $name = $node.Operator.ToString()
            $operators[$name] = 1 + ($operators[$name] ?? 0)
        }
    }
}

$result = [ordered]@{
    files = $files.Count
    parse_errors = $parseErrors
    nodes = $nodes
    commands = $commands
    types = $types
    members = $members
    operators = $operators
}
$result | ConvertTo-Json -Depth 4 | Set-Content -Path $Out -Encoding utf8NoBOM
"analysed $($files.Count) files, $parseErrors with parse errors"
