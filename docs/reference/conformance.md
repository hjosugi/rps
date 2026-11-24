# Conformance suite

The suite is a differential test: a PowerShell script, a recorded result from
the reference `pwsh`, and the same script run by `rpsh`. Everything about the
comparison is deliberate; see [Design memo 01](../01_compat_contract.md).

## Layout

```text
tests/conformance/
├── cases/                  # scripts and golden output, grouped by area
├── passing.txt             # the ratchet: cases that must pass
└── reference.txt           # the pwsh version used to record goldens
```

A case is a `.ps1` file under `cases/`. Its first line declares the level:

```powershell
# rpsh-conformance: level=c2
```

- `c1` compares the exit code and stdout.
- `c2` compares the exit code, stdout and stderr.

Cases run with the working directory set to the case directory. Fixtures
live under `cases/<area>/fixtures/`; files created by a case must be removed
by the case, or be unique to it.

## Commands

```console
$ cargo harness conform record                  # record every case with pwsh
$ cargo harness conform record --filter cmdlets # record a subset
$ cargo harness conform check                   # compare rpsh with the goldens
$ cargo harness conform check --filter cmdlets
$ cargo harness conform check --bless           # add passing cases to the ratchet
```

`record` runs each case three times and refuses a case whose runs disagree.
`check` never invokes `pwsh`; it compares against the committed goldens.

## Rules

- Output must be deterministic. No clock, `$PID`, temporary absolute paths or
  unseeded random values.
- A golden is a reviewable statement about the reference implementation. Read
  the diff before committing it.
- Unsupported constructs are refusals (`rpsh: not implemented: ...`, exit
  `99`), not approximations.
- Only cases in `passing.txt` are required to pass. Adding a new case starts
  unlisted until the implementation is ready.

## Finding the next thing to do

`cargo harness inventory` compares the implemented surface with the inventory
generated from upstream. `scripts/analyze-corpus.py` counts what real scripts
use and where `rpsh` refuses first. Those two, plus the open issues, decide
the next case.
