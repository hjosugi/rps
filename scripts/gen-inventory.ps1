#!/usr/bin/env pwsh
# Regenerates inventory/*.tsv: the surface of upstream PowerShell that rpsh
# has to reproduce, as reported by the latest stable pwsh itself.
#
#   .cache/pwsh/$(cat .cache/pwsh/current)/pwsh -NoProfile -File scripts/gen-inventory.ps1
#
# Every file ends in `platform` and `status` columns (status is todo |
# partial | done | n/a). The platform is the one this run generated (`linux-
# x64`, `macos-arm64`, ...); merge files from other platforms to compare them.
# Regenerating keeps the status of every row that still exists, so the files
# can be refreshed after a pwsh update. `cargo harness inventory` summarizes
# them, grouped by platform.
param(
    [string] $OutDir = (Join-Path $PSScriptRoot '..' 'inventory'),
    # For testing a merge: pretend this run is another platform.
    [string] $PlatformOverride = ''
)

Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'

$os = if ($IsWindows) { 'windows' } elseif ($IsMacOS) { 'macos' } else { 'linux' }
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$Platform = if ($PlatformOverride) { $PlatformOverride } else { "$os-$arch" }

# Only what ships with pwsh, not what the user has installed.
$env:PSModulePath = Join-Path $PSHOME 'Modules'

function Write-Inventory {
    param(
        [string] $Name,
        [string[]] $Columns,
        # How many leading columns identify a row.
        [int] $KeyColumns,
        [object[]] $Rows
    )
    $path = Join-Path $OutDir "$Name.tsv"
    $known = @{}
    $otherPlatform = [System.Collections.Generic.List[string]]::new()
    if (Test-Path $path) {
        $headerFields = (Get-Content -Path $path -TotalCount 1) -split "`t"
        $platformIndex = [Array]::IndexOf($headerFields, 'platform')
        foreach ($line in Get-Content $path | Select-Object -Skip 1) {
            $fields = $line -split "`t"
            $status = $fields[-1]
            if ($platformIndex -ge 0) {
                $rowPlatform = $fields[$platformIndex]
                $key = (@($rowPlatform) + $fields[0..($KeyColumns - 1)]) -join "`t"
                $known[$key] = $status
                if ($rowPlatform -ne $Platform) { $otherPlatform.Add($line) }
            } else {
                # A v1 inventory: its rows belong to this run's platform.
                $key = (@($Platform) + $fields[0..($KeyColumns - 1)]) -join "`t"
                $known[$key] = $status
            }
        }
    }
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add((@($Columns) + @('platform', 'status')) -join "`t")
    foreach ($line in $otherPlatform) { $lines.Add($line) }
    foreach ($row in $Rows) {
        $fields = @($row | ForEach-Object { ([string] $_) -replace '[\t\r\n]+', ' ' })
        $key = (@($Platform) + $fields[0..($KeyColumns - 1)]) -join "`t"
        $status = if ($known.ContainsKey($key)) { $known[$key] } else { 'todo' }
        $lines.Add((@($fields) + @($Platform, $status)) -join "`t")
    }
    Set-Content -Path $path -Value $lines -Encoding utf8NoBOM
    '{0,-24} {1,6} rows' -f $Name, $Rows.Count
}

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

# Variables first: later steps import modules, which adds to the session.
# $OutDir is this script's own parameter, not part of a session.
$variables = Get-Variable | Where-Object Name -NotIn @('OutDir', 'PlatformOverride', 'os', 'arch', 'Platform') | Sort-Object Name | ForEach-Object {
    $type = if ($null -eq $_.Value) { '' } else { $_.Value.GetType().FullName }
    , @($_.Name, $type, $_.Options)
}
Write-Inventory variables @('name', 'type', 'options') 1 $variables

# A command reports its parameters only once its module is loaded.
Get-Module -ListAvailable | Import-Module -ErrorAction SilentlyContinue -WarningAction SilentlyContinue

$commands = Get-Command -CommandType Cmdlet, Function |
    Where-Object Name -NE 'Write-Inventory' |
    Sort-Object { if ($_.Source) { $_.Source } else { '(session)' } }, Name
$rows = $commands | ForEach-Object {
    $module = if ($_.Source) { $_.Source } else { '(session)' }
    , @($module, $_.Name, $_.CommandType)
}
Write-Inventory commands @('module', 'name', 'kind') 2 $rows

$common = [System.Management.Automation.Cmdlet]::CommonParameters +
    [System.Management.Automation.Cmdlet]::OptionalCommonParameters
$rows = foreach ($command in $commands | Where-Object CommandType -EQ 'Cmdlet') {
    # Null when the command's module cannot be loaded on this platform.
    if ($null -eq $command.Parameters) {
        Write-Warning "no parameter metadata for $($command.Name)"
        continue
    }
    foreach ($parameter in $command.Parameters.Values | Sort-Object Name) {
        if ($parameter.Name -in $common) { continue }
        $sets = ($parameter.ParameterSets.Keys | Sort-Object) -join ','
        , @(
            $command.Name,
            $parameter.Name,
            $parameter.ParameterType.FullName,
            ($sets -replace '__AllParameterSets', '*'),
            ($parameter.Aliases -join ',')
        )
    }
}
Write-Inventory parameters @('command', 'parameter', 'type', 'parameter_sets', 'aliases') 2 $rows

# Provider dynamic parameters: what appears in Get-Command's effective
# parameter set but is not declared on the cmdlet type. The FileSystem provider
# supplies these when a path-shaped argument is bound.
$providerCommands = 'Get-ChildItem', 'Get-Item', 'Set-Item', 'New-Item',
    'Remove-Item', 'Copy-Item', 'Move-Item', 'Get-Content', 'Set-Content',
    'Test-Path', 'Resolve-Path'
$common = [System.Management.Automation.Cmdlet]::CommonParameters +
    [System.Management.Automation.Cmdlet]::OptionalCommonParameters +
    'WhatIf', 'Confirm'
$rows = foreach ($name in $providerCommands) {
    $command = Get-Command $name -CommandType Cmdlet -ErrorAction SilentlyContinue
    if ($null -eq $command -or $null -eq $command.ImplementingType) { continue }
    $declared = @($command.ImplementingType.GetProperties() |
        Where-Object { $_.GetCustomAttributes([System.Management.Automation.ParameterAttribute], $true).Count -gt 0 } |
        ForEach-Object Name)
    foreach ($parameter in $command.Parameters.Values | Sort-Object Name) {
        if ($parameter.Name -in $common -or $parameter.Name -in $declared) { continue }
        $source = if ($parameter.Name -in 'FromSession', 'ToSession') { 'remoting' } else { 'FileSystem provider' }
        , @($name, $parameter.Name, $parameter.ParameterType.FullName, $source)
    }
}
Write-Inventory dynamic-parameters @('command', 'parameter', 'type', 'source') 2 $rows

$rows = Get-Alias | Sort-Object Name | ForEach-Object {
    $module = if ($_.Source) { $_.Source } else { '(session)' }
    , @($_.Name, $_.Definition, $module)
}
Write-Inventory aliases @('alias', 'definition', 'module') 1 $rows

$accelerators = [psobject].Assembly.GetType('System.Management.Automation.TypeAccelerators')::Get
$rows = $accelerators.GetEnumerator() | Sort-Object Key | ForEach-Object { , @($_.Key, $_.Value.FullName) }
Write-Inventory type-accelerators @('accelerator', 'type') 1 $rows

$language = [System.Management.Automation.Language.Ast].Assembly
$rows = $language.GetTypes() |
    Where-Object { $_.IsPublic -and $_.IsSubclassOf([System.Management.Automation.Language.Ast]) } |
    Sort-Object Name |
    ForEach-Object {
        $shape = if ($_.IsAbstract) { 'abstract' } elseif ($_.IsSealed) { 'sealed' } else { 'open' }
        , @($_.Name, $_.BaseType.Name, $shape)
    }
Write-Inventory ast-nodes @('node', 'base', 'shape') 1 $rows

$traits = [System.Management.Automation.Language.TokenTraits]
$rows = [enum]::GetValues([System.Management.Automation.Language.TokenKind]) | ForEach-Object {
    , @($_, $traits::Text($_), ($traits::GetTraits($_) -replace ', ', ','))
}
Write-Inventory token-kinds @('kind', 'text', 'flags') 1 $rows

# Operators and keywords are TokenKind values with the matching Trait flags.
$operatorMask = [int]([System.Management.Automation.Language.TokenFlags]::BinaryOperator -bor
    [System.Management.Automation.Language.TokenFlags]::UnaryOperator -bor
    [System.Management.Automation.Language.TokenFlags]::AssignmentOperator -bor
    [System.Management.Automation.Language.TokenFlags]::TernaryOperator -bor
    [System.Management.Automation.Language.TokenFlags]::SpecialOperator)
$rows = [enum]::GetValues([System.Management.Automation.Language.TokenKind]) | ForEach-Object {
    $flags = $traits::GetTraits($_)
    if (([int]$flags -band $operatorMask) -ne 0) {
        , @($traits::Text($_), $_, ($flags -replace ', ', ','))
    }
} | Sort-Object { $_[0] }, { $_[1] }
Write-Inventory operators @('operator', 'token', 'flags') 2 $rows

$keywordMask = [int][System.Management.Automation.Language.TokenFlags]::Keyword
$rows = [enum]::GetValues([System.Management.Automation.Language.TokenKind]) | ForEach-Object {
    $flags = $traits::GetTraits($_)
    if (([int]$flags -band $keywordMask) -ne 0) {
        , @($traits::Text($_), $_, ($flags -replace ', ', ','))
    }
} | Sort-Object { $_[0] }, { $_[1] }
Write-Inventory keywords @('keyword', 'token', 'flags') 2 $rows

# Error ids and the exception types that carry them, from deterministic failures.
$errorExamples = @(
    'Get-Item /no/such/path -ErrorAction Stop',
    'Get-Content /no/such/file -ErrorAction Stop',
    'Set-Location /no/such/directory -ErrorAction Stop',
    'Get-NoSuchThing',
    '1 / 0',
    '[int]::Parse(''x'')',
    '[int]''abc''',
    '$null.Foo()',
    '''abc''.Substring(10)',
    '[System.IO.File]::ReadAllText(''/no/such/file'')',
    'throw ''boom'''
)
$rows = foreach ($example in $errorExamples) {
    try {
        Invoke-Expression $example 2>$null
    } catch {
        , @(
            $_.FullyQualifiedErrorId,
            $_.CategoryInfo.Category,
            $_.Exception.GetType().FullName,
            $example,
            $_.Exception.Message
        )
    }
}
Write-Inventory error-ids @('error_id', 'category', 'exception', 'example', 'message') 4 $rows

$rows = Get-PSProvider | Sort-Object Name | ForEach-Object {
    , @($_.Name, ($_.Capabilities -replace ', ', ','), (($_.Drives.Name | Sort-Object) -join ','))
}
Write-Inventory providers @('provider', 'capabilities', 'drives') 1 $rows

$rows = Get-ExperimentalFeature | Sort-Object Name | ForEach-Object { , @($_.Name, $_.Enabled, $_.Source) }
Write-Inventory experimental-features @('feature', 'enabled_by_default', 'source') 1 $rows

$rows = Get-Verb | Sort-Object Verb | ForEach-Object { , @($_.Verb, $_.AliasPrefix, $_.Group) }
Write-Inventory verbs @('verb', 'alias_prefix', 'group') 1 $rows

$rows = Get-FormatData -PowerShellVersion $PSVersionTable.PSVersion | ForEach-Object {
    foreach ($typeName in $_.TypeNames) {
        foreach ($view in $_.FormatViewDefinition) {
            , @($typeName, $view.Name, ($view.Control.GetType().Name -replace 'Control$', ''))
        }
    }
} | Sort-Object { $_[0] }, { $_[1] }, { $_[2] } -Unique
Write-Inventory format-views @('type', 'view', 'control') 3 $rows

$rows = Get-TypeData | Sort-Object TypeName | ForEach-Object {
    $typeName = $_.TypeName
    foreach ($member in $_.Members.GetEnumerator() | Sort-Object Key) {
        , @($typeName, $member.Key, ($member.Value.GetType().Name -replace 'Data$', ''))
    }
}
Write-Inventory ets-members @('type', 'member', 'kind') 2 $rows
