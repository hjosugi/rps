# Pester suite plan and scorecard

How the upstream PowerShell Pester suite becomes a yardstick, and what the
scorecard will look like before it can run. Relates to #16 and DESIGN.md §2.

## The suite

Upstream keeps its end-to-end tests under `test/powershell`: **409 files**
and roughly **6,650 `It` blocks**. The files per area are recorded in
`tests/pester/areas.tsv`:

| area | files | what it covers |
|---|---:|---|
| Modules | 228 | module cmdlets, providers, remoting, packaging |
| engine | 84 | session state, pipeline, binder, errors, jobs |
| Language | 82 | parser, AST, operators, type conversion, classes |
| Host | 9 | ConsoleHost, prompting, line editing |
| SDK | 3 | `Microsoft.PowerShell.SDK` packaging and hosting |
| Provider | 2 | provider contracts |
| dsc | 1 | Desired State Configuration |

The suite is not vendored. `scripts/gen-pester-plan.sh`-style extraction is
not useful before Pester itself runs, so the plan starts from what must work
first.

## Classification

Every file is classified on four axes; a test that needs any of them is
blocked until that axis works.

| axis | meaning | examples |
|---|---|---|
| platform | runs on Linux with the reference pwsh | most of Modules, engine, Language |
| Windows | needs the registry, CIM, COM or Windows-only cmdlets | Windows dsc, registry provider |
| network | reaches a remote host or the internet | remoting, `Invoke-WebRequest` |
| CLR | needs a .NET type or assembly the native shim does not cover | serialization, SDK |

A test also needs the Pester runtime: `Describe` / `Context` / `It`, `Should`,
`Mock`, `TestDrive`, `BeforeAll` / `AfterAll`, `-ForEach`, and Pester's own
module loading. That is the first list to satisfy; it is language plus cmdlets,
not a separate engine.

## Shortest path to a running suite

1. **Run Pester itself.** Import the Pester module that ships with the
   reference environment (or a pinned copy), run one trivial test and make
   `Invoke-Pester -Output Detailed` exit zero. This proves module import,
   advanced functions, `Should` and the error/stream machinery.
2. **Run one area at a time**, starting with `Language`, then a small Linux
   module area, then `engine`. Each area's blockers go into the scorecard.
3. **Extract before running the whole suite.** A file whose `It` bodies are
   plain scripts can become a conformance case (`cargo harness conform`)
   earlier than Pester can run. The extraction is per `It`, not per file, and
   is tracked as its own issue.
4. **Gate on a rate, not a count.** A new area lands when its Linux-executable
   tests pass at the agreed rate; Windows/network/CLR tests are reported
   separately and do not lower that rate.

## Scorecard

`scripts/pester-scorecard.py` reads `tests/pester/areas.tsv` and writes a
markdown sheet. Without a result file every cell is zero and the sheet says
so; a runner writes `--results FILE` as JSON keyed by area:

```json
{
  "Modules": {"tests": 1201, "passed": 10, "failed": 2, "skipped": 80, "blocked": 1109}
}
```

The CI job `pester-scorecard` writes `pester-scorecard.md` and uploads it as
an artifact. It does not run Pester yet, so it cannot fail the build. When a
runner exists, it will feed the same shape and the CI job will gain the
rate gate.

## What this does not decide

The scorecard ranks *coverage*, not performance. Pester results do not
replace `cargo harness bench` or the conformance cases; they are the third
yardstick in DESIGN.md §2.
