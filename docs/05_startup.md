# Startup

What pwsh spends before it runs the first statement, what rpsh spends, and
the budget that keeps rpsh at 5 ms as the engine grows. Settles #5;
DESIGN.md §4 summarises it.

Measurements: `bench/baselines/2026-11-21-ryzen5-4650u-startup-stages.json`,
`...-startup-jit.json` and `...-startup-tables.json`, written by
`scripts/time-commands.py` and `cargo bench -p rpsh-research --bench
startup_tables`.

## Method

`cargo harness bench` times scripts under both engines. Startup needs
something else: whole command lines, interleaved, with a process's own
resource use. `scripts/time-commands.py` runs a JSON list of commands
through `scripts/spawn-measure.c`, which forks the program directly (so peak
RSS is the program's, not Python's), waits with `wait4` and reports wall
time, CPU time, peak RSS and page faults. Every round starts at a different
command so that drift spreads over all of them; medians are reported and
every sample is kept.

All commands run in the reference environment of
docs/01_compat_contract.md: own `HOME`, fixed `PATH`, en-US, UTC, no `TERM`.

## pwsh, stage by stage

The probes are the reference pwsh with `pwsh.dll` replaced by a small program
compiled by the Roslyn that ships with it (`scripts/build-startup-probes.sh`,
`bench/startup/probes`). They keep the same apphost, runtime, runtime
configuration and assemblies, and stop at one stage:

* `empty`: an empty `Main`; the runtime starts and exits.
* `load-sma`: additionally loads `System.Management.Automation` and touches
  one of its types.
* `runspace`: additionally creates a default `InitialSessionState`, opens a
  runspace and runs `exit` in it.

Median of 40 runs, milliseconds:

| command | wall | min | p90 | CPU | RSS MiB |
|---|---:|---:|---:|---:|---:|
| `/usr/bin/true` | 0.79 | 0.47 | 1.19 | 0.4 | 1.6 |
| `rpsh -Version` | 1.24 | 0.66 | 1.65 | 0.9 | 2.2 |
| `rpsh -NoProfile -Command exit` | 1.22 | 0.70 | 1.71 | 0.7 | 2.2 |
| `rpsh -NoProfile -File empty.ps1` | 1.00 | 0.63 | 1.66 | 0.7 | 2.4 |
| probe: empty `Main` | 19.77 | 17.90 | 29.37 | 21.1 | 21.4 |
| probe: load SMA | 21.99 | 18.77 | 34.38 | 22.1 | 25.6 |
| probe: runspace + `exit` | 169.78 | 150.70 | 186.75 | 206.0 | 76.6 |
| `pwsh -Version` | 70.00 | 60.60 | 77.75 | 93.2 | 54.6 |
| `pwsh -NoProfile -Command exit` | 156.41 | 134.12 | 179.69 | 255.5 | 89.0 |
| `pwsh -NoProfile -File empty.ps1` | 155.14 | 143.11 | 173.65 | 255.1 | 88.8 |
| `pwsh -Command exit` (profile lookup) | 158.03 | 140.72 | 179.35 | 254.7 | 89.1 |

What that says:

* **The runtime alone costs 20 ms** — 20 times the whole of rpsh today — and
  loads 21 MiB before any PowerShell code exists.
* **Loading SMA costs another 2.2 ms.** The assembly is not the expensive
  part; building session state is.
* **The runspace is the bulk:** 170 ms and 77 MiB with no ConsoleHost, no
  profile and no PSReadLine. The probe is a little slower than `pwsh -File`
  because it builds a fresh default session state and adds a script rather
  than using pwsh's own startup path, so read it as "the engine costs about
  as much as all of pwsh", not as a lower bound.
* **Reading the profile is cheap; the profile is not.** Looking for it adds
  2 ms. What a profile *does* (importing modules, `tool init |
  Invoke-Expression`) is where the rest would go, and that is why
  DESIGN.md §4 rule 3 caches compiled output.

rpsh's 1.0 ms today is a floor: the skeleton parses almost nothing and
builds no session state. It is the number the budget is enforced against,
not a result (DESIGN.md §7 rule 6).

## What JIT and the caches contribute

`bench/startup/jit.json` runs `bench/startup/inside.ps1`, which reports
`JitInfo` and GC counters from inside pwsh after startup, under switches
that change code generation. Median of 20 runs:

| setting | wall | JIT | methods | assemblies | RSS MiB |
|---|---:|---:|---:|---:|---:|
| default | 211.3 ms | 46.5 ms | 324 | 66 | 94.8 |
| fresh `HOME` every run | 245.8 ms | 32.8 ms | 187 | 55 | 86.2 |
| `DOTNET_TieredPGO=0` | 206.3 ms | 37.3 ms | 230 | 66 | 94.3 |
| `DOTNET_TieredCompilation=0` | 212.9 ms | 63.7 ms | 171 | 66 | 94.8 |
| `DOTNET_ReadyToRun=0` | 832.0 ms | 1,323 ms | 10,611 | 66 | 98.9 |

* **ReadyToRun is doing most of the work.** Without the precompiled images
  the same start JITs 10,611 methods and costs 832 ms. rpsh has no
  equivalent cost because it has no JIT.
* **Tiered compilation is a small win here.** Turning it off compiles fewer
  methods more expensively (64 ms against 47 ms). The default settings are
  already near the floor for this workload.
* **A fresh `HOME` is slower, not faster.** It loads 11 fewer assemblies
  and JITs 137 fewer methods, yet takes 34 ms longer: pwsh's startup work,
  not its caches, dominates. rpsh must not need a warm `HOME` to be fast.
* The JIT lines are a floor for any design that hosts the CLR: a T1 script
  that touches .NET pays a comparable cost once (docs/06_dotnet_bridge.md).

## Static data instead of construction

`research/benches/startup_tables.rs` builds the command table from the
inventory and times construction and lookup. Median:

| operation | time |
|---|---:|
| build an eager map of 298 commands and 124 aliases | 63.9 µs |
| build eager metadata for 2,076 parameters | 1.00 ms |
| look up all 422 names in the map | 26.0 µs |
| look up all 422 names in a sorted table | 38.9 µs |

Constructing parameter metadata alone is 20% of the 5 ms budget, and no
script needs it at startup. The decision follows:

* **Compile-time tables.** Command names, aliases, type accelerators and
  flags are emitted as sorted static arrays at build time. A lookup is a
  case-folded binary search over a `&'static str` table: 38.9 µs for all 422
  names, no allocation, no construction, and no relocation work beyond what
  the loader already does paginating the binary. A perfect hash would save
  the 12.9 µs difference; it is not worth a second generated format until a
  profile shows lookups.
* **Parameter metadata per command, on first use.** Decoded from the static
  table when a command is resolved, not for the whole inventory.
* **Everything else on first use.** Cmdlet implementations, aliases, type
  and format data, providers, help, completion, the native shim and the
  CLR bridge are resolved by name and built when a script first reaches
  them. A script that prints one integer builds none of them.

## Prior art

* **fish 4.9.3 (`6cf3137`) is the sorted-table precedent:** 64 builtins in
  a `const` slice, binary-searched (`src/builtins/shared/misc.rs:111`,
  `:377`), with `share/` embedded (`src/autoload.rs:41`). Functions and
  completions autoload by file behind a 1024-entry miss LRU with 15 s
  staleness (`src/autoload.rs:268`, `:246`). Eager: `config.fish`
  (`src/bin/fish.rs:166-183`), `conf.d` (`share/config.fish:156-164`),
  universal variables (`src/env/environment.rs:768`). History is mmap-backed,
  read on use (`src/history/history.rs:1223`); autosuggestion and
  highlighting use background threads (`src/reader/reader.rs:5590`, `:5784`).
  Skips: `--no-config`/`-N`, `--no-execute`/`-n` (`src/bin/fish.rs:231`, `:232`).
* **Nushell 0.117.0 (`a481925`) registers eagerly, caches lazily:** `main`
  adds about 550 commands (source count, not runtime; `src/main.rs:159`).
  Registration is not lazy; signatures are, built once on use behind a
  `OnceLock` (`crates/nu-protocol/src/engine/signature_cache.rs:8-19`,
  `:55-60`), as are plugin commands declared from cached registry
  signatures without spawning the plugin
  (`crates/nu-plugin-engine/src/init.rs:269-286`). The embedded default env
  file is evaluated every start (`src/config_files.rs:209-218`), as is
  `use std/prelude *` (`crates/nu-std/src/lib.rs:174-203`); the prelude
  opens with `use std/dt` (`crates/nu-std/std/prelude/mod.nu:1`), so
  `std/dt` is inferred parsed too. No on-disk parsed-script cache exists
  (`crates/nu-parser/src/parse_captures_compile.rs:818-847`). Skips:
  `-n/--no-config-file`, `--no-std-lib`, `--no-history`
  (`src/command.rs:746-748`).
* **brush 0.4.0 (`96a26d0`), yash-rs 3.6.0 (`46d2f79`), ion (`e9b3ca4`)
  build builtin `HashMap`s at startup** (`brush-core/src/shell.rs:125`;
  `yash-env/src/lib.rs:125`; `src/lib/builtins/mod.rs:105`). brush reads
  history before the first prompt (`brush-core/src/shell.rs:251-253`);
  yash-rs has no line editor, history or completion
  (`docs/src/README.md:13-14`); ion runs `initrc` only when interactive
  (`src/binary/mod.rs:63`, `:198`). None autoloads. Lockfile counts (fish
  196, brush 667, yash-rs 70, ion 311) are workspace-wide, not shipped
  dependencies.

**Takeaway.** rpsh adopts fish's sorted compile-time table and Nushell's
per-command `OnceLock` metadata, rejecting startup-built maps and eager
signature construction: static like fish but wider, lazy like Nushell but
unregistered. The ~550 eager registrations and eager env/prelude parsing
are what rpsh budgets against; no surveyed shell targets a cold-process
PowerShell-compatible start, so 5 ms sits far below all of them.

## The budget

**`rpsh -NoProfile -File empty.ps1` stays under 5 ms on the reference
machine**, and `rpsh -NoProfile -Command exit` answers to the same budget.
The split, which the implementation is held to:

| stage | budget |
|---|---:|
| process creation, dynamic linking, loader | 1.0 ms |
| arguments and environment | 0.3 ms |
| parse | 0.5 ms |
| engine and session-state setup reached by an empty script | 0.7 ms |
| output and exit | 0.5 ms |
| reserve for growth | 2.0 ms |
| **total** | **5.0 ms** |

The measured floor is 1.00 ms (`-File empty.ps1`) and 0.79 ms for
`/usr/bin/true`; the remaining stages are not separable in the skeleton
yet. When the engine exists, the reserve is what pays for a new dependency
or a new eager table, and a change that needs more than the reserve is a
design change, not a tuning change.

## Enforcement

* `bench/cases/startup/*.ps1` carry `# rpsh-bench: budget-ms=5` and
  `runs=30`. The harness records `within_budget` and fails the run when a
  timed case exceeds its budget (`--budget-scale` multiplies it).
* The harness runs cases with `-File`, so `-File empty.ps1` is the case CI
  enforces. `scripts/time-commands.py` measures `-Command exit` separately
  and it is part of the same budget; a `-Command` harness case is #9's job.
* CI's `startup-budget` job runs those cases on every push to `main` and
  every pull request with `--budget-scale 2`. Shared runners are slower and
  noisier; doubling catches the regressions that matter — an eagerly built
  table, a file read, a library loaded at startup — while leaving a few
  percent to the reference machine's baselines.
* The scheduled benchmark job keeps the full comparison and the cases that
  are too noisy for pull requests.
* A dependency added to `rpsh` answers to the reserve and to binary size;
  the rules are in docs/07_dependencies.md.
