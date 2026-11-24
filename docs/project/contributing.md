# Contributing

Issues use **背景 / スコープ / Acceptance Criteria / 依存**. Forms exist for
features, compatibility bugs and research. A research issue is complete when a
decision memo under `docs/` records the evidence and what was adopted or
rejected.

Labels:

- `area:*` names the part of the system
- `kind:feature`, `kind:test`, `kind:infra`, `kind:research`, `kind:epic`
  name the type of work
- `compat` marks behaviour that must equal the reference `pwsh` and needs a
  conformance case
- `perf` needs a result file from the harness
- `post-parity` marks improvements that wait for M8

The full guide is [CONTRIBUTING.md](https://github.com/hjosugi/rpsh/blob/main/CONTRIBUTING.md).

## Before a pull request

```console
$ scripts/check.sh
$ cargo deny check
```

`scripts/check.sh` runs formatting, clippy with `-D warnings`, the workspace
tests and the conformance check. CI runs `cargo deny check` as its own job.

## The shape of a behaviour change

1. Write the conformance case first.
2. Record what `pwsh` does and read the golden.
3. Implement until `cargo harness conform check` passes.
4. Lock the case in with `--bless`.
5. Update the inventory status.

Anything not handled must remain a refusal, never an approximation.

## Adding a dependency

Every external crate is linked into every start. Open the issue, measure the
startup and binary-size cost, add it with `default-features = false`, gate it
behind a feature unless `rpsh -Command exit` needs it, and run
`cargo deny check`. The details are in [memo 07](../07_dependencies.md).

## Performance claims

A number in an issue, a PR or the README cites a file in `bench/baselines/`.
Copy `bench/results/latest.json` there as
`<date>-<machine>-<topic>.json` when a run is worth keeping.

## Documentation

The site is MkDocs Material. Preview it with:

```console
$ python3 -m venv .venv
$ . .venv/bin/activate
$ pip install -r requirements-docs.txt
$ mkdocs serve
```

Design memos are numbered by the M0 issue that settled the decision. Add a
memo to the navigation in `mkdocs.yml` when it lands.
