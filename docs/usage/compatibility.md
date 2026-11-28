# Compatibility

The compatibility contract is in [Design memo 01](../01_compat_contract.md).
The short version:

1. **`pwsh` is the reference.** Behaviour is pinned by recorded runs of the
   latest stable PowerShell, not by memory or documentation alone.
2. **A case compares more than success.** C2 compares the exit code, stdout and
   stderr; C1 compares the exit code and stdout.
3. **Unsupported means refused.** `rpsh` exits `99` with
   `rpsh: not implemented: ...`. It never substitutes a similar result.

## Desired State Configuration on Linux

The Linux compatibility target leaves legacy DSC configuration compilation
to installations that provide the OMI schema store and DSC resources. When
`rpsh` encounters a `configuration` declaration, it refuses the script
during parsing with exit code 99. The word can still be passed as a command
argument or used to name a function after a pipe. The Linux acceptance
case covers those contexts and command discovery. [Issue #134](https://github.com/hjosugi/rpsh/issues/134)

In the clean PowerShell 7.7.0 Linux x64 reference environment,
`Get-Module -ListAvailable PSDesiredStateConfiguration` returned no modules.
Command discovery found none of `Get-DscResource`, `Invoke-DscResource` or
`Start-DscConfiguration`. Parsing the following declaration reported
`ParserError` at `Example` because the schema store at
`/etc/opt/omi/conf/dsc/configuration` was absent. Recovery also reported
`UnexpectedToken` at the closing brace. The parser can reproduce these
observations without applying a configuration.

```powershell
$tokens = $null
$errors = $null
$null = [System.Management.Automation.Language.Parser]::ParseInput(
    'configuration Example { }', [ref]$tokens, [ref]$errors)
$errors | Select-Object ErrorId, Extent
```

DSC v3 has a separate command-line application. When installed, `dsc`
uses the ordinary native-command path. The PowerShell 7.7.0 bundle includes
the `Microsoft.PowerShell/Profile` resource, whose manifest invokes `pwsh`
for `get`, `set` and `export`. The reference machine had no `dsc` on
`PATH`. Calling the bundled resource's read-only `get` operation directly
with `{"profileType":"CurrentUserCurrentHost","_exist":false}` on stdin
returned the profile path, null content and `_exist: false` in an empty
test home. End-to-end DSC v3 orchestration requires a separate DSC installation.
[DSC v3 overview](https://learn.microsoft.com/en-us/powershell/dsc/overview?view=dsc-3.0)
and [PowerShell 7.7.0 resource manifest](https://github.com/PowerShell/PowerShell/blob/v7.7.0/dsc/pwsh.profile.dsc.resource.json).

## Running the suite

```console
$ cargo harness conform check
```

The check compares `rpsh` with the goldens in the repository, so it needs no
`pwsh`. To record or update a case, install the reference first:

```console
$ scripts/fetch-pwsh.sh
$ cargo harness conform record --filter <case>
```

`record` runs each case three times and refuses if the runs disagree. The
golden is committed with the source; that diff is the review of what `pwsh`
actually does.

## Adding a case

1. Put a script under `tests/conformance/cases/<area>/`.
2. Start it with `# rpsh-conformance: level=c2` (or `c1`).
3. Keep output deterministic: avoid time, `$PID`, unseeded random numbers and
   temporary paths.
4. Record the golden.
5. Implement until the check passes, then add the case to
   `tests/conformance/passing.txt`.
6. Update the status in `inventory/*.tsv`.

The ratchet is the passing list. A case that is recorded but not listed is
allowed to fail; a case that is listed may not.

## Determinism and platform

Goldens are recorded on Linux. Platform-dependent output is part of the case,
not a tolerance. If a behaviour cannot be deterministic, it needs a different
observation point rather than an approximate match.

## Errors

PowerShell error records are modelled as error records, not as plain strings:
the error id, category, exception type and message are carried through
`catch`, `trap` and the formatted error output. `try`/`catch` selects by type.

## Performance

Performance work follows correctness: a benchmark measures only outputs that
match between `pwsh` and `rpsh`. Results worth citing live under
`bench/baselines/`; the methodology is in
[DESIGN.md](https://github.com/hjosugi/rpsh/blob/main/DESIGN.md).
