# Design notes

[DESIGN.md](https://github.com/hjosugi/rpsh/blob/main/DESIGN.md) is normative:
the architecture and the numbered sections that issues cite. The memos
collect the decisions that were open when the design was written: what was
measured, what was chosen, and what was rejected and why. Numbers in them
cite a file under `bench/baselines/`.

The memos are in the **Design memos** section of this site:

| Memo | Decision |
|---|---|
| Compatibility contract | What "the same as the reference pwsh" means and how it is enforced |
| Value model | How values are represented; compact strings, 16-byte `Value` |
| Memory | Reference counting with a backup cycle collector |
| Execution | A register IR with inline caches, interpreted |
| Startup | Where pwsh's startup time goes and rpsh's budget |
| .NET bridge | T0 shim, T1 `hostfxr`, T2 sidecar; the SMA facade |
| Dependencies | What may be linked, and how a crate is admitted |
| Pester plan | How the upstream Pester suite is classified and scored |
| Corpus | What real scripts use, by frequency, and what rpsh refuses first |
| xUnit port | Status of the upstream xUnit tests, crate by crate |

The index with issue links is
[`docs/README.md`](https://github.com/hjosugi/rpsh/blob/main/docs/README.md).

## How a decision is settled

A design decision is settled by a bench under `research/benches`, run with
`cargo bench -p rpsh-research --bench <name>`. Its result is
`bench/results/research-<name>.json`; copy it to
`bench/baselines/<date>-<machine>-<topic>.json` and cite that file from the
memo. A memo without a measurement is an opinion.
