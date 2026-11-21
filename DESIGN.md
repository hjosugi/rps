# rpsh design

rpsh is an implementation of PowerShell in Rust. It targets one pinned
upstream release, PowerShell 7.7.0, and exists to answer a question with
measurements instead of opinions: how fast is PowerShell when
there is no .NET runtime to start and no JIT to wait for?

Issues refer to the sections of this document by number.

## 1. Goals

In priority order. A later goal never justifies breaking an earlier one.

1. **Measure honestly.** The benchmark and conformance harnesses exist before
   the engine does, and every performance claim cites a result file (§7).
2. **Run `.ps1` scripts exactly as pwsh 7.7.0 does**, on Linux first.
3. **Cover what upstream ships**: every cmdlet, provider, host feature and
   engine subsystem in the inventory (§2), then macOS and Windows.
4. **Linux commands just work**: native programs run, pipe and take over the
   terminal the way they do in any other shell (§6).
5. **Start instantly and stay small**: nothing is initialized before it is
   used, and the profile is not re-compiled on every start (§4).
6. **Then improve on upstream** (§9), once the above hold.

## 2. What "a complete port" means

The reference is the pinned pwsh binary, not documentation and not memory.
`scripts/fetch-pwsh.sh` installs it, verified by SHA-256. The full contract is
[docs/01_compat_contract.md](docs/01_compat_contract.md); in short:

* **One environment.** Both engines run every case in an environment built
  from nothing: own `HOME`, fixed `PATH`, en-US, UTC, no `TERM`. What the
  answers still depend on (.NET 11.0.0, the system ICU) is recorded in
  `tests/conformance/reference.txt`.
* **What is compared** grows in levels: exit code and stdout today, then
  stderr, the stream of every record, and every `ErrorRecord` field.
  Messages match exactly: PowerShell's own (English only — the Linux release
  ships no translations) and .NET's.
* **Identity.** `$PSVersionTable`, `$PSEdition` and `$Host` read as pwsh
  7.7.0's. A script that needs to know it runs under rpsh asks for the type
  `[Rpsh.EngineInfo]`.
* **Determinism.** A golden is recorded only if three pwsh runs agree. What
  pwsh randomizes (string hash order) rpsh may fix; what pwsh repeats
  (integer-key order, seeded random numbers) rpsh reproduces.
* **Quirks are the specification.** Changing one is a §9 improvement.

**The never-guess rule.** rpsh parses a script in full before running any of
it. Anything it does not implement — syntax, an operator on some pair of
types, a parameter — ends the run with exit code 99 and
`rpsh: not implemented: <feature>` on stderr. It never produces an
approximate answer. This is what makes the measurements in §7 meaningful: a
result is either pwsh's result or an explicit refusal. A refusal is not an
exception: no `catch`, `trap` or error preference can intercept it.

Progress is counted three ways:

| Yardstick | Where | Size today |
|---|---|---|
| Conformance cases with recorded pwsh output | `tests/conformance` | grows with every feature |
| Upstream surface: commands, parameters, aliases, AST nodes, tokens, format views, ... | `inventory/*.tsv` | 298 commands, 2,076 parameters |
| Upstream's own Pester suite | `test/powershell` upstream | 409 files, about 6,650 `It` blocks |

Compatibility is layered, because PowerShell is defined on top of .NET and a
script may reach any .NET API:

| Level | Scope | How |
|---|---|---|
| L1 | Language and engine semantics | native, exact |
| L2 | Cmdlets, providers, formatting shipped with pwsh on Linux | native, exact |
| L3 | .NET types and members used from scripts | native shim first, CLR behind it (§5) |
| L4 | Binary modules and `Add-Type` | CLR bridge (§5) |
| L5 | Windows-only surface (registry, CIM, COM, WSMan) | after Linux parity |

"Complete" is L1 and L2 implemented natively, plus L3 and L4 reachable
through the bridge so that no script is out of reach in principle.

## 3. Architecture

### 3.1 Crates

| Crate | Upstream counterpart | Nushell analogue |
|---|---|---|
| `rpsh-syntax` | `engine/parser`: tokenizer, parser, AST | `nu-parser` |
| `rpsh-runtime` | `LanguagePrimitives`, `MshObject`, `runtime/Operations` | `nu-protocol` |
| `rpsh-engine` | compiler, session state, command discovery, binder, pipeline, modules | `nu-engine` |
| `rpsh-dotnet` | the BCL as scripts see it | — |
| `rpsh-providers` | `namespaces`: providers and path resolution | `nu-path`, `nu-glob` |
| `rpsh-native` | `NativeCommandProcessor` and friends | `nu-system` |
| `rpsh-format` | `FormatAndOutput` | `nu-table` |
| `rpsh-commands` | Core, Utility, Management, Security, Host modules | `nu-command` |
| `rpsh-host` | `Microsoft.PowerShell.ConsoleHost` | `nu-cli` |
| `rpsh-cli` | `powershell-unix` (`pwsh`) | `nu` |
| `rpsh-bench` | `test/perf`, plus differential testing | `nu-test-support` |

Upstream is about 734,000 lines of C# under `src/`, 565,000 of them in
`System.Management.Automation`.

### 3.2 Front end

A hand-written tokenizer and recursive-descent parser that follow upstream's
structure, including the tokenizer's switch between expression mode and
command mode. AST node names match upstream's 74 public `*Ast` classes so that
`[System.Management.Automation.Language.Parser]`, `$scriptBlock.Ast` and
`Ast.FindAll` can be exposed to scripts unchanged. Parse errors carry
upstream's error ids.

### 3.3 Values

A 16-byte `Value` enum with unboxed primitives. A `PSObject` wrapper exists
only where extended members are attached, as upstream does lazily. The
representation is settled in [docs/02_value_model.md](docs/02_value_model.md)
and the lifetime rules in [docs/03_memory.md](docs/03_memory.md):

* **Strings.** `.Length`, indexing and `Substring` count UTF-16 code units.
  A string is either ASCII bytes or UTF-16 code units, never both, so ASCII
  stays one byte per unit and a non-ASCII string indexes in O(1); lone
  surrogates are kept exactly and become U+FFFD only at the UTF-8 boundary.
* **Memory.** Reference counting with `Arc`, plus a generational cycle
  collector over containers, triggered by container allocations so a short
  script never pays for it. Values are `Send + Sync`: a runspace may hand an
  object to another and see the same object.
* **Numbers.** Upstream's promotion rules (`Int32` overflow becomes `Double`,
  integer division may yield `Double`, conversion rounds half to even) and
  its `G15` formatting of `Double` are reproduced exactly.

### 3.4 Execution

The AST is lowered to a compact register IR, as Nushell does, and interpreted.
Dynamic sites — member access, method overload resolution, command lookup,
conversions — carry inline caches, which is the job the DLR's call-site
caching does upstream. The prototypes and their measurements are in
[docs/04_execution.md](docs/04_execution.md): the register form is 3.2 times
faster than walking the AST with name lookups on a million-iteration loop
(1.5 times faster than the slot-resolved AST or a stack VM), and lowering a
profile-sized script costs about 220 µs, cached on disk afterwards.

Upstream interprets a script block first and JIT-compiles it once it is hot.
A long-running tight loop is therefore where rpsh is most likely to lose, and
`bench/cases/language` keeps such cases in view on purpose.

### 3.5 Pipeline and streams

Cmdlets stream one object at a time through `begin` / `process` / `end`, with
upstream's six streams and its rules for enumerating collections. Data
between native commands is a byte stream, never decoded unless a cmdlet reads
it (Nushell's `PipelineData::ByteStream`).

### 3.6 Concurrency

Engine state is immutable and shared; each runspace owns a stack (Nushell's
`EngineState` / `Stack` split). Runspaces, jobs and `ForEach-Object -Parallel`
then need no copy of the engine.

## 4. Startup and lazy initialization

pwsh pays for a runtime, assembly loading, JIT and session-state construction
on every start, and again for each line of a profile: `Import-Module` loads
assemblies, `tool init | Invoke-Expression` spawns a process and compiles its
output, and function definitions are re-parsed each time because nothing is
cached between sessions.

Measured against 7.7.0 ([docs/05_startup.md](docs/05_startup.md)): an empty
.NET application costs 19 ms and 21 MiB; loading `System.Management.Automation`
adds 1.4 ms; building a default session state and running `exit` reaches
164 ms and 77 MiB. `pwsh -NoProfile -Command exit` takes 147 ms and 89 MiB.
That is the budget rpsh is measured against.

Rules:

1. **Nothing is built before it is used.** The cmdlet table, aliases, type
   accelerators, type and format data, providers and the .NET shim are static
   data resolved by name, or are constructed on first use. A script that
   prints one integer constructs none of them.
2. **A budget, enforced.** `rpsh -NoProfile -Command exit` stays under 5 ms
   on the reference machine. `bench/cases/startup` guards it, and every
   dependency linked into `rpsh` answers to it.
3. **Compile once.** Parsed and lowered scripts, the profile included, are
   cached on disk under a content hash. Function bodies compile on first
   call. A command-to-module index lets a module load on the first use of one
   of its commands rather than at startup.
4. **Small.** Binary size and peak RSS are recorded next to wall time.
   Subsystems that few sessions need (remoting, web cmdlets, the CLR bridge)
   sit behind cargo features and load lazily at run time.
5. **Prompt first.** In an interactive session the prompt appears before the
   line editor's history, completion data or anything else is loaded.

## 5. The .NET bridge

Scripts name .NET types directly, so the question is not whether to support
.NET but when to pay for it. The measurements behind the tiers, and the part
of `System.Management.Automation` binary modules actually use, are in
[docs/06_dotnet_bridge.md](docs/06_dotnet_bridge.md).

| Tier | What | Cost |
|---|---|---|
| T0 | Native shim: the types scripts use most, in Rust, with upstream's member names, overloads and conversions | none |
| T1 | In-process CoreCLR loaded through `hostfxr` on first need: a type outside the shim, `Add-Type`, a binary module | paid once, by the scripts that need it |
| T2 | Warm sidecar: a per-user process that keeps a runtime and assemblies loaded across rpsh invocations | first use only |

* The tier a script needs is computed from its AST before it runs, so the
  harness reports results per tier and a user can ask why a script is slow.
* The shim grows by measured use: the type and member frequencies of a corpus
  of real scripts decide what comes next.
* CLR objects are handles. Member access goes through cached delegates, and
  identity is preserved across the boundary.
* Binary modules are compiled against `System.Management.Automation.dll`.
  Loading them needs a facade assembly with that public API, backed by the
  rpsh engine.
* Each module's assemblies load in their own `AssemblyLoadContext`, resolved
  lazily, so two modules can ship different versions of one dependency — a
  known source of failures upstream.
* The runtime is found through `DOTNET_ROOT`, an installed pwsh, or a bundled
  copy. Without one, rpsh runs T0 scripts and says exactly which type or
  module needs more.

## 6. Native commands on Linux

rpsh is a shell: on Linux most command lines are native programs.

* **Arguments**: `$PSNativeCommandArgumentPassing`, `--%`, array expansion,
  and the wildcard and `~` expansion pwsh performs on Unix for unquoted
  arguments.
* **Pipes**: bytes pass between native commands untouched; text decoding
  happens only at a cmdlet boundary, per `$OutputEncoding`.
* **Terminal**: interactive programs get the terminal as a foreground process
  group and it is restored afterwards; `SIGINT`, `SIGTSTP` and `SIGWINCH` go
  where a user expects.
* **Status**: `$LASTEXITCODE`, `$?`, `&&` / `||`, and
  `$PSNativeCommandUseErrorActionPreference`.
* **Names**: upstream defines no `ls`, `cp`, `mv`, `rm`, `cat`, `ps`, `kill`,
  `sort`, `tee` or `sleep` aliases on Unix, so the native programs win. rpsh
  does the same.
* **Being a shell**: `-Login`, `-c` for tools that call `$SHELL -c`, shebang
  scripts, `/etc/shells`.

Parity with pwsh comes first. Job control and other things pwsh lacks belong
to §9.

## 7. Measurement discipline

1. **Pinned reference.** Results name the pwsh version they were taken
   against; goldens are recorded from 7.7.0 only.
2. **Verified or not reported.** A speedup is computed only for a case where
   rpsh's stdout and exit code equal pwsh's. A refusal shows pwsh's time and
   the missing feature. A different answer fails the run.
3. **Same invocation.** Both engines get the same switches, locale, time zone
   and working directory.
4. **Method.** Warm-up runs, then alternating timed runs; the median is
   reported and every sample is kept. The result file records CPU, governor,
   load average, toolchain and commit.
5. **Baselines.** A result worth citing is copied to `bench/baselines/`. A
   number in the README or in an issue points at one.
6. **Floors are labelled.** While rpsh initializes nothing, its startup time
   is a lower bound, not a result. It becomes a result once the engine it
   stands for exists.
7. **Losses stay in the suite.** Cases where upstream's JIT is expected to
   win are kept and reported.
8. **Ratchet.** `tests/conformance/passing.txt` lists the cases that pass. A
   listed case that stops passing fails CI.

## 8. Phases

| Milestone | Exit criterion |
|---|---|
| M0 Foundation | harnesses, inventory, CI, baseline, the open design decisions settled |
| M1 Language Front-end | every construct pwsh 7.7.0 parses yields the same AST; same parse errors |
| M2 Runtime Core | values, conversions, operators, extended type system, T0 shim core |
| M3 Engine | functions, scopes, binder, pipeline, errors, modules: real scripts run |
| M4 Native Interop | the Linux command matrix passes |
| M5 Providers & Cmdlets | every command in `inventory/commands.tsv` is `done` |
| M6 Host & Formatting | default output matches byte for byte; interactive use is comfortable |
| M7 Upstream Parity | jobs, remoting, debugger, events, classes, CLR bridge |
| M8 Verification & Performance | Pester scorecard, published performance report, startup and size budgets met |
| M9 Improvements | features upstream does not have |

The walking skeleton in the repository today — integer and string literals,
`+ - *`, `exit` — exists to prove the harnesses end to end.

## 9. Improvements after parity

Allowed once M8's criteria hold, under two rules: a valid PowerShell script
keeps its meaning, and anything that changes behaviour is opt-in. Candidates:
job control (`Ctrl-Z`, `fg`, `bg`), diagnostics with source spans and
suggestions, structured wrappers for common Linux commands, a compile cache
for `init | Invoke-Expression` output, a language server and debug adapter, a
plugin protocol for out-of-process cmdlets, more data formats, a formatter
and linter.

## 10. Non-goals

* Windows PowerShell 5.1 compatibility.
* Reproducing .NET internals that no script can observe.
* GUI cmdlets (`Out-GridView`, `Show-Command`).
* Becoming a different language. Nushell is prior art for the architecture,
  not a target for the syntax.
