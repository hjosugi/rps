# Compatibility contract

What "rpsh behaves like the reference pwsh" means, precisely enough that a
test can decide it. Every rule below names what enforces it; where nothing does yet,
the entry says which issue will. Settles #1; DESIGN.md §2 summarises it.

## The reference

The reference is a binary, not documentation: the latest stable PowerShell
release, resolved from GitHub by `scripts/fetch-pwsh.sh` and verified against
the SHA-256 digest the release publishes. It is self-contained and runs on
the .NET runtime it bundles (`pwsh.runtimeconfig.json`); the exact version and
runtime are recorded in `tests/conformance/reference.txt`, and every result
file names the version it ran with. ".NET behaviour" in this document means
that bundled runtime's.

### Reference environment

Both engines run every case in the same environment, built from nothing
(`Engine::command` in `crates/rpsh-bench/src/engines.rs` clears the
environment first):

| Variable | Value | Why |
|---|---|---|
| `HOME` | `<checkout>/.cache/home` | pwsh reads modules (`~/.local/share/powershell/Modules`), `powershell.config.json` and its startup JIT profile from below `$HOME`; a user's own must not leak in |
| `PATH` | `/usr/local/bin:/usr/bin:/bin` | a case names the same programs on every machine |
| `LANG`, `LC_ALL` | `en_US.UTF-8` by default | a conformance case can set both with its `LANG=` directive |
| `TZ` | `UTC` | local time |
| `NO_COLOR` | `1` | `$PSStyle.OutputRendering` is `PlainText` |
| `POWERSHELL_TELEMETRY_OPTOUT`, `POWERSHELL_UPDATECHECK` | `1`, `Off` | no network traffic during a run |
| everything else | unset | in particular `TERM`, `COLUMNS`, `PSModulePath`, `XDG_*` |

stdin is `/dev/null`, stdout and stderr are pipes, and the working directory
is the case's directory.

`TERM` is the reason the environment is built from nothing. Before this
contract the harness passed the caller's environment through, and measured
on the reference machine:

| `TERM` | `$Host.UI.RawUI.WindowSize` | `[Console]::WindowWidth` |
|---|---|---|
| unset | 80x40 | 0 |
| `xterm-256color` | 80x24 | 80 |
| `dumb` | 80x-1 | 80 |

so a golden recorded from a terminal could differ from one recorded in CI.
`COLUMNS=200` changes nothing while output is redirected. All 31 goldens
that existed before the change were re-recorded under the new environment
and came out byte-identical.

`cargo harness conform record` writes what the goldens depend on, besides
the pwsh version, to `tests/conformance/reference.txt`: the .NET version, the
OS, the system ICU version, culture, time zone and window size. At the time
of writing that is .NET 11.0.0, ICU 78.3, en-US, UTC, 80x40.

## What must match

| Level | Compared | Enforced by |
|---|---|---|
| C1 | exit code and stdout, byte for byte | `judge` in `engines.rs`, today |
| C2 | C1 plus stderr byte for byte, after the case's directory is replaced by `<case>` in both | `judge`, the `.stderr` companion, and the case's `# rpsh-conformance: level=c2`, today |
| C3 | the stream each record went to (output, error, warning, verbose, debug, information, host) and its order | `contract/streams` (record types through redirection), today |
| C4 | each `ErrorRecord`: `FullyQualifiedErrorId`, `CategoryInfo`, exception type, message, position | `contract/error-records` through stdout, today |

C1 is weaker than it looks, because under ConsoleHost pwsh writes most
streams to stdout as text. Measured with output redirected: `Write-Host`,
`WARNING: `, `VERBOSE: `, `DEBUG: ` and information records go to stdout;
only error records and `[Console]::Error` go to stderr. C2 adds the error
view, whose text includes the script's absolute path
(`Exception: /…/throw.ps1:2`), hence the placeholder. Every case declares its
level with `# rpsh-conformance: level=c1|c2`. For a case that needs a specific
culture, add the locale on the same line, for example
`# rpsh-conformance: level=c2 LANG=tr_TR.UTF-8`. Recording and checking
start each engine with that value for both `LANG` and `LC_ALL`.

A golden v2 is `rpsh-golden-v2`, a `exit=<code>` line, then stdout byte for
byte. Stderr, when non-empty, is a `<name>.stderr` companion next to it. The
case's directory is replaced by `<case>` on both sides.

Timing is not part of the contract. A case that needs time to pass prints
something derived from it instead (`$elapsed -lt 5`), never a duration.

## Refusals

An unimplemented feature ends the run with exit code 99 and
`rpsh: not implemented: <feature>` on stderr. Most refusals happen before
the script starts, because rpsh parses all of it first. Some can only
happen while it runs: a .NET member outside the native shim, a conversion
between two types nobody has written yet.

A refusal is not an exception. `try`/`catch`, `trap`,
`-ErrorAction SilentlyContinue` and `$ErrorActionPreference` cannot see
it, and `finally` blocks do not run. Otherwise a script could catch the
refusal and print something else, and the never-guess rule would be broken
by the scripts it is supposed to protect. Output written before a
run-time refusal does not count: the verdict is "not implemented" whatever
stdout already holds. Implemented by #63.

pwsh can exit with 99 itself (`exit 99`), so the harness recognises a
refusal by the marker on stderr together with the exit code. Both are
tested in `engines.rs`.

## Identity

rpsh answers every question a script can ask about its version the way the
reference pwsh does:

* `$PSVersionTable` has pwsh's nine keys with the reference version's
  values: `PSVersion` is the version in `tests/conformance/reference.txt`,
  `GitCommitId` is what that release reports, `PSEdition` Core, `Platform`
  Unix, `PSCompatibleVersions` as the reference reports it, and `OS` as .NET
  reports it (the `PRETTY_NAME` of `/etc/os-release` on the reference
  machine). Keys enumerate in pwsh's order: `PSVersion`, `PSEdition`, then
  the rest ordinal-ignore-case (`PSVersionTableComparer` in
  `engine/PSVersionInfo.cs`).
* `$PSEdition` is `Core`, `$IsCoreCLR` is `$true`, `$Host.Name` is
  `ConsoleHost`, `$Host.Version` is the reference version.

No key is added to `$PSVersionTable` to say "this is rpsh". Its keys and its
default display are output like any other, and modules gate on its values
(`#requires -PSEdition Core`, `CompatiblePSEditions` in manifests).

A script that needs to know uses a type pwsh does not have:
`'Rpsh.EngineInfo' -as [type]` is `$null` under pwsh, and under rpsh the type
exposes `Version` (rpsh's own) and `TargetPSVersion`. A type literal is the
narrowest channel available: a new variable shows up in `Get-Variable` and
`variable:`, and an environment variable is inherited by every child process,
including a real pwsh started from rpsh. Implemented with the
`System.Management.Automation` shim (#54).

The command line is compared like any other output, with one exception:
`rpsh -Version` prints `rpsh <version> (targets latest stable PowerShell)`, because
the question it answers is which program this is. Whether rpsh can be
installed under the name `pwsh` is decided with distribution (#153).

## What does not repeat

Measured over six pwsh processes running the same script:

| Observation | Distinct results in 6 runs |
|---|---|
| keys of `@{ alpha = 1; … }` (8 string keys) | 6 |
| `'alpha'.GetHashCode()` | 6 |
| `ConvertTo-Json` of that hashtable | 6 |
| keys of `@{ 1 = 'a'; 2 = 'b'; 3 = 'c'; 10 = 'd'; 20 = 'e' }` | 1 (`10,20,3,2,1`) |
| `[ordered]@{}`, `HashSet[string]`, `Dictionary[string,int]` keys | 1 (insertion order) |
| `Group-Object` names, `Select-Object -Unique` | 1 (first appearance) |
| `[pscustomobject]` property order | 1 (as written) |

.NET randomizes string hash codes per process; integer hash codes are the
integer. The rules that follow from that:

1. **A golden exists only for a deterministic answer.** `conform record`
   runs each case three times and refuses to write a golden when the runs
   disagree. A case may not print a string-keyed hashtable's order, the
   time, `$PID`, a temporary path, a GUID or an unseeded random number. It
   may print something derived from them that does not change.
2. **What pwsh leaves to chance, rpsh may fix.** rpsh's `Hashtable` uses
   .NET's bucket layout (`System.Collections.Hashtable`: double hashing over
   a prime-sized table, load factor 0.72), so integer keys come out as
   `10,20,3,2,1` like pwsh's. Its string hash is its own and fixed, so
   rpsh's string-keyed order is the same in every run. That is an order pwsh
   could also have produced, and no script can depend on pwsh's. (#43)
3. **What pwsh does repeat, rpsh reproduces, however incidental it looks.**
   Integer-key order is one example; seeded random numbers are another:
   `Get-Random -SetSeed 42 -Maximum 1000` is 177 in every run, then 467, and
   `[System.Random]::new(7).Next()` is 822959691. rpsh implements .NET's
   seeded `Random` algorithm exactly. (#96, #49)

Cases `contract/collection-order` and `contract/seeded-random` hold these
facts.

## Errors and messages

**PowerShell's own messages match exactly.** They come from 150 neutral
`.resx` files with 4,185 strings (75 files and 3,071 strings in
`System.Management.Automation`). The repository also carries translations
for 13 cultures, but the Linux release ships no satellite assemblies (no
`*.resources.dll` in the tarball), and under `LANG=ja_JP.UTF-8` pwsh prints
the English messages. So rpsh's message catalog is the neutral resources
alone, and culture changes formatting, never messages. (#8, #115)

`scripts/gen-messages.py` extracts the catalog from the latest stable source
tag, recorded with its archive SHA-256 in the generated file header, into
`crates/rpsh-messages/src/catalog.rs`. It is one table sorted by
`"<resource set>/<name>"`, read with a binary search when a message is
needed, so no message work happens at startup. `{0}`-style placeholders go
through `rpsh_messages::format`, which follows .NET's composite formatting
for string arguments: index, alignment, `{{`/`}}` escapes, and a format
specifier that a string ignores.

**.NET's messages match too, when they reach the output.** An exception
thrown by a .NET member shows its message in the error view and in
`$_.Exception.Message`, wrapped by PowerShell's own text:

```text
Exception calling "Parse" with "1" argument(s): "The input string 'x' was not in a correct format."
Exception calling "Substring" with "1" argument(s): "startIndex cannot be larger than length of string. (Parameter 'startIndex')"
```

The inner text is .NET 11.0.0's. rpsh's native shim throws the same
exception types with the same messages; the catalog is extracted from the
runtime's own resources (`System.Private.CoreLib`) by the same tool (#8).

**Error records are compared field by field** (level C4): the
`FullyQualifiedErrorId` (`PathNotFound,Microsoft.PowerShell.Commands.GetItemCommand`),
the category line (`ObjectNotFound: (/no/such/path:String) [Get-Item], ItemNotFoundException`),
the exception's full type name and the message. `contract/error-records`
covers nine common errors.

**The error view is output.** ConciseView's layout (`Line |` gutter, the
`~~~` underline, wrapping at the 80-column width of the reference
environment) is compared byte for byte from level C2 on. (#107)

## Culture

The culture comes from `LANG`. pwsh does not bundle ICU: it loads the system's
(`libicuuc.so.78.3` from `/usr/lib` on the reference machine) and takes
date patterns, number formats and collation from it. The same reference pwsh
therefore prints different text on systems with different ICU data. Under
ICU 78.3, en-US puts U+202F NARROW NO-BREAK SPACE before AM/PM:

```text
$moment.ToString()  →  1/2/2026 3:04:05 PM
'a','B','A','b','_','1' | Sort-Object  →  _,1,a,A,B,b
```

The rules:

* The reference is the ICU recorded in `reference.txt`. A golden that
  depends on culture data is correct for that ICU version.
* rpsh takes culture data from the same place pwsh does, the system ICU,
  loaded on first use of a culture-sensitive operation. It is the only way
  to agree with pwsh on every distribution rather than on one. Without ICU,
  pwsh needs `DOTNET_SYSTEM_GLOBALIZATION_INVARIANT`, and rpsh behaves as
  pwsh does in that mode. The mechanism, and its cost against the startup
  budget, belong to #36 and #44.
* Where pwsh is culture-invariant, so is rpsh: `"$moment"` prints
  `01/02/2026 15:04:05` and `"$(1.5)"` prints `1.5` under any culture,
  while `'{0:C}' -f 12.5` and `.ToString('N2')` use the current one.
* en-US is the default. The collation cases under `contract/` also run with
  Turkish, Swedish, Japanese and C locales selected in their directives.

`contract/culture` holds the en-US facts.

## Console

In the reference environment pwsh's console is 80x40, output is
redirected, `$PSStyle.OutputRendering` is `PlainText`, `$ErrorView` is
`ConciseView` and both encodings are UTF-8 (`contract/console`). Default
formatting and the error view wrap at 80 columns. Behaviour at a terminal
(colour, width from the terminal, the line editor) is compared through a
pseudo-terminal by its own harness (#15).

## Quirks

The reference pwsh's behaviour is the specification, including behaviour that looks
accidental. Changing it is an improvement in the sense of DESIGN.md §9: only
after parity, and only behind an opt-in. When the reference release moves, the
goldens are re-recorded and every changed golden is reviewed as a change in
upstream's behaviour (`.github/workflows/performance.yml` already re-records
and diffs weekly).

## Platforms

Linux x64 is the reference. Linux arm64 uses the same contract once CI runs
it. macOS and Windows get reference files of their own when they are
supported (#139, #140).

## Enforcement summary

| Rule | Enforced by |
|---|---|
| reference environment | `Engine::command`, every run |
| reference recorded | `conform record` writes `tests/conformance/reference.txt` |
| deterministic goldens | `conform record` (3 runs per case), unit test `repeated_runs_must_agree` |
| C1 | `judge`, `cargo harness conform check` in CI |
| C2 | `judge`, `<name>.stderr`, per-case level directive |
| C3–C4 | `contract/streams`, `contract/error-records` |
| PTY prompt, Ctrl-C, `less` | `cargo harness conform pty record` / `check` |
| refusal recognised only with its marker | unit test `exit_code_99_without_the_marker_is_an_ordinary_result` |
| refusal not catchable | #63 |
| identity | `contract/version-table` |
| collection order, seeded random | `contract/collection-order`, `contract/seeded-random` |
| messages, error records | `contract/error-records` |
| culture | `contract/culture` |
| console | `contract/console` |
