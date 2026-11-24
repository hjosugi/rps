# Status

`rpsh` is under active development. The authoritative answer to "does it do
X?" is the conformance suite and the inventory, not this page.

```console
$ cargo harness conform check      # what the recorded cases require
$ cargo harness inventory          # how much of upstream's surface is covered
```

The ratchet is [`tests/conformance/passing.txt`](https://github.com/hjosugi/rpsh/blob/main/tests/conformance/passing.txt);
every listed case must pass. Cases that are recorded but not listed document
work in progress and may still be refusals.

## Implemented today

| Area | State |
|---|---|
| Lexer and parser | Expressions, statements, functions, classes, enums, modules and script files |
| Runtime | Compact UTF-16 strings, 16-byte values, object identity, numeric promotion |
| Execution | Pipelines, command binding, scopes, errors and `trap`, native commands |
| Output | Default table/list formatting, `Out-*`, `Format-Table`, `Format-List` |
| Pipeline cmdlets | Selection, ordering, grouping, measuring, comparison, teeing |
| Files and paths | Content, items, child items, copy/move/remove/rename, path helpers, hashing |
| Data | JSON and CSV conversion, CSV import/export |
| Discovery | Commands, aliases, modules, members |
| Types | Common casts and `[Math]`, `[Convert]`, `[IO.Path]`, `[String]`, `[Char]`, `[Version]`, `[DateTime]` methods |
| Randomness | `Get-Random` including seeded output and shuffle |

## Not implemented yet

- Interactive sessions, profiles and PSReadLine
- Scripts from standard input (`-Command -`, `-File -`)
- Remoting, jobs, workflows and runspaces
- The full .NET type surface and the CLR bridge
- Providers other than the implemented file/environment/variable paths
- The complete cmdlet and Pester surface

A missing feature is reported by name with exit code `99`. That is the
compatibility contract in [memo 01](../01_compat_contract.md).

## Versioning

Each behaviour change lands with a conformance case, then a release commit
that bumps the workspace version. `cargo harness conform check` is the gate;
the CI workflow runs the same script on Linux x64, Linux arm64 and macOS.
