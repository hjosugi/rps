# Scripting

The language follows PowerShell's grammar and runtime semantics where the
conformance suite has pinned them. This page is a map, not a complete
reference.

## Values and operators

- `$null`, booleans, integers, `long`, `double`, strings and `char`
- arrays and `[ordered]`/hashtables, with object identity preserved
- `+ - * / %`, comparison operators and their `c`/`i` forms
- `-like`, `-match`, `-replace`, `-split`, `-join`, `-contains`, `-in`,
  `-is`, `-as`
- `-f`, `-and`/`-or`/`-not`, ternary `? :`, null-coalescing `??` and `??=`

`$Matches` is the automatic variable from the last successful `-match`.

## Statements

`if`/`elseif`/`else`, `while`, `do`/`until`, `for`, `foreach`, `switch`
(including `-Wildcard`, `-Regex`, `-CaseSensitive`), `try`/`catch`/`finally`,
`throw`, `trap`, `break`/`continue` with labels, functions, filters,
`param()`, classes and enums.

## Pipelines

The pipeline passes values, not text. `begin`/`process`/`end` blocks and
`$_`/`$PSItem` work in functions and script blocks. Common pipeline cmdlets
include:

`Where-Object`, `ForEach-Object`, `Select-Object`, `Sort-Object`,
`Group-Object`, `Measure-Object`, `Compare-Object`, `Tee-Object`,
`Get-Unique`, `Join-String`, `Select-String`.

## Cmdlets

Implemented groups include:

- output and formatting: `Write-*`, `Out-String`, `Out-File`, `Out-Null`,
  `Format-Table`, `Format-List`
- variables and aliases: `Get/Set/New/Clear/Remove-Variable`,
  `Get/Set-Alias`
- files and paths: `Get/Set/Add/Clear-Content`, `Get-ChildItem`, `Get-Item`,
  `New/Remove/Copy/Move/Rename-Item`, `Test-Path`, `Resolve-Path`,
  `Join-Path`, `Split-Path`, `Get-FileHash`
- data: `ConvertTo/From-Json`, `ConvertTo/From-Csv`, `Import/Export-Csv`
- discovery: `Get-Command`, `Get-Module`, `Import-Module`, `Get-Member`,
  `Get-Help` is not implemented yet
- environment and location: `Get/Set-Location`, `Push/Pop-Location`,
  `Get-Culture`, `Get-Date`
- math and random: `Get-Random` (including `-SetSeed`, `-Count`, `-Shuffle`)

## Types and methods

`[int]`, `[long]`, `[double]`, `[bool]`, `[string]`, `[char]`, `[version]`
and `[datetime]` can be cast. Frequently used static methods on `[Math]`,
`[Convert]`, `[IO.Path]`, `[String]` and `[Char]` are implemented, as are
instance methods such as `Substring`, `Replace`, `Split`, `Trim` and
`PadLeft`/`PadRight`.

DateTime values support component properties, `ToString()` formats,
`AddDays`, `AddHours`, `AddMinutes` and `AddSeconds`.

## Modules

`.psm1` modules can be imported with `Import-Module`; dot-sourcing a script
runs it in the current scope. `Get-Module` reports imported modules.

## What to do when a script fails

Read the message and the exit code. `rpsh: not implemented: ...` means the
construct is outside the implemented surface. That is the project asking for
a conformance case, not a runtime bug. See
[Compatibility](compatibility.md).
