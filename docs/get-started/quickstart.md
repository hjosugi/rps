# Quick start

## Run a command

`-Command` takes the rest of the command line and runs it:

```console
$ rpsh -Command "1 + 2"
3
$ rpsh -Command "'a', 'b', 'c' | Sort-Object -Descending"
c
b
a
```

## Run a script

```powershell
# report.ps1
param([string] $Path = '.')

Get-ChildItem $Path -File |
    Sort-Object Length |
    Select-Object Name, Length
```

```console
$ rpsh -File report.ps1 -Path ./src
```

`$PSScriptRoot` and `$PSCommandPath` are set for script files. Script
arguments that follow the path are bound to `param()` or collected in
`$args`, as in `pwsh`.

## Work with objects

The pipeline passes objects, not text:

```powershell
$people = @(
    [pscustomobject]@{ Name = 'Ada'; Age = 36 },
    [pscustomobject]@{ Name = 'Grace'; Age = 45 }
)

$people |
    Where-Object Age -gt 40 |
    Select-Object -ExpandProperty Name
```

Output is formatted at the host: a small object becomes a table, a larger one
a list, exactly as the reference does for the same values.

## Use native commands

Native commands run with the session's environment and exit code:

```powershell
Get-ChildItem . -File |
    ForEach-Object Name |
    Select-String '\.rs$'
```

The last native exit status is in `$LASTEXITCODE`; `$?` reports whether the
last command succeeded.

## Get help

```console
$ rpsh -Help
$ rpsh -Version
```

## When something is not implemented

`rpsh` refuses by name instead of guessing:

```console
$ rpsh -Command '1..10'
rpsh: not implemented: ranges (at offset 0)
$ echo $?
99
```

That is intentional. A missing feature is a missing feature; it is not an
approximation. See [Compatibility](../usage/compatibility.md).
