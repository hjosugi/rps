# rps

rps is an implementation of PowerShell in Rust, measured against upstream
[PowerShell](https://github.com/PowerShell/PowerShell) 7.7.0. Its command is
`rpsh`.

The aim is to run `.ps1` scripts exactly as `pwsh` does, cover what `pwsh`
ships, run Linux commands the way a shell should, and find out by measurement
how much faster that is without a .NET runtime to start. The design is
[DESIGN.md](DESIGN.md); the work is tracked in the
[issues](https://github.com/hjosugi/rps/issues).

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
* a differential test suite whose expected output is recorded from the real
  `pwsh` 7.7.0;
* an inventory of upstream's surface on Linux — 299 commands, 1,568
  parameters, 157 token kinds, 74 AST node types and more — generated from
  `pwsh` itself, with a status column.

## Quick start

```bash
cargo build --release               # target/release/rpsh
scripts/fetch-pwsh.sh               # the pinned pwsh 7.7.0, into .cache/
cargo harness bench                 # rpsh vs pwsh, per script
cargo harness conform check         # differential tests; needs no pwsh
cargo harness inventory             # how much of upstream is done
scripts/check.sh                    # fmt, clippy, tests, conformance
```

## Measurements

`cargo harness bench` prints a table and writes `bench/results/latest.json`.
Results worth citing are kept in [bench/baselines](bench/baselines).

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

The project is rps; the command is `rpsh`. `rps` was already taken as a crate
on crates.io and as a command installed by an npm package.

## License

MIT, the license of upstream PowerShell. See [LICENSE](LICENSE) and
[NOTICE](NOTICE).
