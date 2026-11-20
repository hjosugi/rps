# Contributing

Issues use 背景 / スコープ / Acceptance Criteria / 依存. Forms are provided for
features, compatibility bugs and research. A research issue is complete when a
decision memo under `docs/` records the evidence and what was adopted or
rejected.

Labels: `area:*` names the part of the system; `kind:feature`, `kind:test`,
`kind:infra`, `kind:research` and `kind:epic` name the type of work; `compat`
marks behaviour that must equal pwsh 7.7.0 and needs a conformance case;
`perf` needs a result file from the harness; `post-parity` marks improvements
that wait for M8 (DESIGN.md §9).

Before a PR:

```bash
scripts/check.sh
```

It runs `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` and `cargo harness conform check`.

## Adding behaviour

1. Write the conformance case first: a script under
   `tests/conformance/cases/<area>/`. Keep its output deterministic.
2. Record what pwsh does: `cargo harness conform record` (needs the pinned
   pwsh, `scripts/fetch-pwsh.sh`). Read the golden; pwsh is often surprising.
3. Implement until `cargo harness conform check` passes, then lock the case in
   with `cargo harness conform check --bless`.
4. Set the `status` of what you finished in `inventory/*.tsv`.

Anything a change does not handle must stay a refusal
(`rpsh: not implemented: ...`, exit code 99), never an approximation
(DESIGN.md §2).

## Performance claims

A number in an issue, a PR or the README cites a file in `bench/baselines/`.
Copy `bench/results/latest.json` there as
`<date>-<machine>-<topic>.json` when a run is worth keeping.
