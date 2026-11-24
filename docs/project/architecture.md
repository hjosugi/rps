# Architecture

`rpsh` is a workspace of small crates. The boundaries exist so the startup
budget can be argued about per dependency, and so the parser, runtime and host
can be tested in isolation.

| Crate | Responsibility |
|---|---|
| `rpsh-syntax` | Full tokenizer, parser, AST and source spans |
| `rpsh-runtime` | Values, conversions, operators, strings, regex, errors |
| `rpsh-engine` | Execution, session state, binding, pipelines, cmdlets |
| `rpsh-host` | Command line, script files, sessions, `PSScriptRoot` |
| `rpsh-cli` | The `rpsh` executable |
| `rpsh-dotnet` | The .NET surface: native shim and lazy CLR bridge |
| `rpsh-providers` | FileSystem, Environment, Variable, Function, Alias |
| `rpsh-native` | Native commands, processes, pipes and the terminal |
| `rpsh-format` | `Format-*`, `Out-*` and default views |
| `rpsh-commands` | Cmdlets grouped by area |
| `rpsh-messages` | Upstream message catalog and placeholder formatting |
| `rpsh-bench` | `cargo harness`: benchmarks, conformance, inventory |

## Execution

Source text becomes tokens, then an AST, then a tree-walking interpreter over
the AST. Values are 16 bytes and strings are stored compactly: ASCII inline,
UTF-16 when needed. Objects and arrays are reference counted; identity is
preserved where PowerShell preserves it.

The engine owns the session: variables, functions, aliases, modules, classes,
the random generator and the pipeline runner. A command is bound from
positional and named arguments, then either a builtin, a user function, a
cmdlet, a class constructor or a native process handles it.

See [memo 04](../04_execution.md) for the execution design and
[memo 02](../02_value_model.md) for the value model.

## Compatibility boundary

The parser is deliberately strict. A construct that is not implemented is
refused by the parser or by the engine, with a span and a feature name. That
is how the project avoids the class of bug where a script runs and quietly
produces the wrong object.

The reference is `pwsh`; the tests are differential. Read
[Compatibility](../usage/compatibility.md) and [memo 01](../01_compat_contract.md).

## Startup and dependencies

Startup is a first-class feature: every linked crate is paid for by every
process, including `rpsh -Command exit`. Dependencies are admitted one at a
time with a measured cost, as described in [memo 07](../07_dependencies.md).
The startup analysis is in [memo 05](../05_startup.md).

## .NET

The design keeps a path to the .NET surface without making it mandatory at
startup. [Memo 06](../06_dotnet_bridge.md) describes the tiers: a native shim,
`hostfxr` and a sidecar. Until a script needs them, none of that is loaded.

## Testing

Three layers:

1. Unit tests in each crate.
2. The conformance suite, which is the compatibility contract.
3. Benchmarks under `bench/` and `research/`, which keep performance claims
   tied to result files.

Token kinds are compared against the reference parser, and expression ASTs
are compared the same way (`scripts/token-diff.py`).
