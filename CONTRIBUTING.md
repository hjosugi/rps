# Contributing

Issues use 背景 / スコープ / Acceptance Criteria / 依存. Forms are provided for
features, compatibility bugs and research. A research issue is complete when a
decision memo under `docs/` records the evidence and what was adopted or
rejected.

Labels: `area:*` names the part of the system; `kind:feature`, `kind:test`,
`kind:infra`, `kind:research` and `kind:epic` name the type of work; `compat`
marks behaviour that must equal the reference pwsh and needs a conformance case;
`perf` needs a result file from the harness; `post-parity` marks improvements
that wait for M8 (DESIGN.md §9).

Before a PR:

```bash
scripts/check.sh
cargo deny check
```

`scripts/check.sh` runs `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` and `cargo harness conform check`. CI runs
`cargo deny check` as its own job; run it locally when a dependency changes.

## Adding behaviour

1. Write the conformance case first: a script under
   `tests/conformance/cases/<area>/`, and declare what is compared with
   `# rpsh-conformance: level=c2` (exit code, stdout and stderr) or `c1`
   (exit code and stdout). Keep its output deterministic: no string-keyed
   hashtable order, time, `$PID`, temporary path or unseeded random number
   (docs/01_compat_contract.md).
2. Record what pwsh does: `cargo harness conform record` (needs the latest
   stable pwsh, `scripts/fetch-pwsh.sh`). It runs each case three times and
   refuses a case whose runs disagree. Read the golden; pwsh is often
   surprising. It also rewrites `tests/conformance/reference.txt`, so a
   version bump is a reviewable diff.
3. Implement until `cargo harness conform check` passes, then lock the case in
   with `cargo harness conform check --bless`.
4. Set the `status` of what you finished in `inventory/*.tsv`.

Anything a change does not handle must stay a refusal
(`rpsh: not implemented: ...`, exit code 99), never an approximation
(DESIGN.md §2).

## Adding a dependency

Every external crate is linked into every start, so it answers to the
startup budget. The rules, the candidate list and the measurement spike are
in docs/07_dependencies.md. In short: open the issue, measure the startup
and binary-size cost, put it in `[workspace.dependencies]` with
`default-features = false`, gate it behind a feature unless `rpsh -Command
exit` needs it, and run `cargo deny check`.

## Research measurements

A design decision is settled by a bench under `research/benches`, run with
`cargo bench -p rpsh-research --bench <name>`. Its result is
`bench/results/research-<name>.json`; copy it to
`bench/baselines/<date>-<machine>-<topic>.json` and cite that file from the
decision memo in `docs/`. Add the bench to `research/Cargo.toml` and the
document to the index in docs/README.md.

## Performance claims

A number in an issue, a PR or the README cites a file in `bench/baselines/`.
Copy `bench/results/latest.json` there as
`<date>-<machine>-<topic>.json` when a run is worth keeping.

## Generated files

`scripts/fetch-pwsh.sh` installs the latest stable release and writes
`.cache/pwsh/current`. Regenerate the files that depend on it from the
checkout root:

```bash
scripts/fetch-pwsh.sh
version=$(cat .cache/pwsh/current)
.cache/pwsh/$version/pwsh -NoProfile -File scripts/gen-inventory.ps1
scripts/gen-messages.py
cargo harness conform record
cargo harness inventory
```

`gen-inventory.ps1` writes a `platform` column (`linux-x64`, `macos-arm64`,
`windows-x64`, ...) and preserves the hand-edited `status` column. Running it
on another platform and merging the files is how a platform difference is
recorded; `cargo harness inventory` summarizes each platform separately.

## PTY cases

`tests/conformance/pty/*.json` describe an interactive program, a schedule of
input bytes and a timeout. Record the final screen with the reference pwsh:

```bash
cargo harness conform pty record
cargo harness conform pty check
```

`record` uses pwsh, `check` uses rpsh, and both share the golden
(`<name>.golden`: exit code, final screen, timeout). The prompt and Ctrl-C
cases are expected to fail `check` until rpsh implements interactive sessions;
`scripts/check.sh` does not run them yet. Add a native program case (such as
`less`) by naming its absolute path in `program`.
