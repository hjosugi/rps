# rpsh

rpsh is an implementation of PowerShell in Rust, measured against the latest stable upstream
[PowerShell](https://github.com/PowerShell/PowerShell) release. The command is
`rpsh`.

The aim is to run `.ps1` scripts exactly as `pwsh` does, cover what `pwsh`
ships, run Linux commands the way a shell should, and find out by measurement
how much faster that is without a .NET runtime to start. The user documentation is at
<https://hjosugi.github.io/rpsh/>. The design is
[DESIGN.md](DESIGN.md); the work is tracked in the
[issues](https://github.com/hjosugi/rpsh/issues), starting from the
[roadmap](https://github.com/hjosugi/rpsh/issues/182).

## Status

A scaffold. `rpsh` runs a walking skeleton of the language — integer and
single-quoted string literals, `+ - *`, parentheses, `exit` — and refuses
everything else by name:

```console
$ rpsh -Command "(1 + 2) * 3"
9
$ rpsh -Command '$x = 1'
rpsh: not implemented: variables (at offset 0)
$ echo $?
99
```

What is complete is the machinery around it:

* a benchmark harness that times every script under `pwsh` and `rpsh` and
  reports a speedup only where both print the same thing;
* a differential test suite whose exit code, stdout and stderr are recorded
  from the real `pwsh`; the exact version is in
  `tests/conformance/reference.txt`;
* an inventory of upstream's surface on Linux — 298 commands, 2,076
  parameters, 157 token kinds, 74 AST node types and more — generated from
  `pwsh` itself, with a status column;
* a message catalog extracted from upstream's neutral `.resx` resources,
  with .NET-compatible placeholder formatting.

## Quick start

```bash
cargo build --release               # target/release/rpsh
scripts/fetch-pwsh.sh               # the latest stable pwsh, into .cache/
cargo harness bench                 # rpsh vs pwsh, per script
cargo harness bench --compare bench/baselines/2026-11-20-ryzen5-4650u-scaffold.json
cargo harness bench --suite shells # bash/zsh/fish/nu/pwsh/rpsh startup
cargo harness bench --suite profiles # pwsh profile cost by fixture
cargo harness bench --export-hyperfine hyperfine.json
cargo bench -p rpsh-research --bench engine_micro
cargo harness conform check         # differential tests; needs no pwsh
cargo harness inventory             # how much of upstream is done
scripts/check.sh                    # fmt, clippy, tests, conformance
```

## Measurements

`cargo harness bench` prints a table and writes `bench/results/latest.json`
(schema `rpsh-bench-v2`). Results worth citing are kept in
[bench/baselines](bench/baselines).

Each run goes through `scripts/spawn-measure.c`, compiled into `.cache/tools/`
on first use, so the result file has more than wall time: `wait4` reports user
and system CPU time and peak RSS, and the table shows a 95% bootstrap
confidence interval for rpsh's median. Samples outside Tukey's fences are
listed as outliers and the file records the CPU governor, load average and any
warning about them. The `rpsh` block also records the binary size.

`--compare BASELINE.json` prints a before/after table; both the current schema
and the `rpsh-bench-v1` baselines already in `bench/baselines/` are readable.
`--export-hyperfine FILE` writes hyperfine-compatible JSON.

`cargo harness bench --suite shells` compares the installed shells (bash,
dash, zsh, fish, nu, pwsh and rpsh): a one-line run through `wait4`, and the
time to the first output on a pseudo-terminal, which is the prompt. A shell
that is not installed is recorded as skipped. The result is
`bench/results/shells-latest.json`; a baseline is in `bench/baselines/`.

`cargo harness bench --suite profiles` loads the fixtures under
`bench/profiles` as pwsh's profile: an empty profile, 100 functions, a module
import, `tool init | Invoke-Expression`, alias/completer registration and all
of them together. The deltas against the empty profile are the per-shape
cost. rpsh does not read profiles yet, so its row is recorded as not
implemented.

`cargo bench -p rpsh-research --bench engine_micro` measures parse throughput
and invoke cost in process, next to stopwatch loops inside the reference pwsh.
It is the in-process counterpart to `cargo harness bench`; the upstream
numbers are estimates, not BenchmarkDotNet results.

The suite has 52 cases across language, pipeline, text/data, filesystem,
native and startup, with at least two sizes in every workload category.
The first baseline was taken on the scaffold. Read it for what it is: `pwsh`'s
times are real and are the numbers to beat; `rpsh` is verified on the two
startup cases only, and there it measures a process that initializes nothing.
That is a floor, not a result (DESIGN.md §7).

## Layout

| Path | What |
|---|---|
| `crates/rpsh-syntax` | tokenizer, parser, AST |
| `crates/rpsh-runtime` | values, conversions, operators |
| `crates/rpsh-engine` | execution, session state, binding, pipeline |
| `crates/rpsh-dotnet` | the .NET surface: native shim and lazy CLR bridge |
| `crates/rpsh-providers` | FileSystem, Environment, Variable, Function, Alias |
| `crates/rpsh-native` | native commands: processes, pipes, terminal |
| `crates/rpsh-format` | `Format-*`, `Out-*`, default views |
| `crates/rpsh-commands` | the cmdlets |
| `crates/rpsh-host` | command line, script and interactive sessions |
| `crates/rpsh-cli` | the `rpsh` executable |
| `crates/rpsh-bench` | `cargo harness`: benchmarks, conformance, inventory |
| `bench/cases` | benchmark scripts |
| `tests/conformance` | differential cases and their recorded `pwsh` output |
| `inventory` | upstream's surface, with status |

## The name

The project and the command are both `rpsh`.

## License

MIT, the license of upstream PowerShell. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).
