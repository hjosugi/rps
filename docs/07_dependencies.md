# Dependencies

What rpsh may link, how a new crate is admitted, and what it costs. Settles
#7; DESIGN.md §4 rule 2 and §4 rule 4 are the constraints.

The workspace has no external dependency today: `cargo tree --depth 1`
shows only the workspace crates. That is a starting point to defend, not an
accident. Every crate that ends up in `rpsh` is linked into every start,
whether or not the script uses it.

## Rules

1. **A crate answers to the startup budget.** `rpsh -NoProfile -File
   empty.ps1` stays under 5 ms, and `-Command exit` answers to the same
   budget (docs/05_startup.md). A dependency whose static
   initializers, dynamic relocations or page faults show up in
   `scripts/time-commands.py` is rejected or moved behind a feature.
2. **The startup path is a separate list.** A crate needed by the parser,
   the engine or the host's argument handling belongs to `rpsh`'s default
   features. Everything else — line editing, HTTP, the CLR bridge, remoting
   — is an opt-in cargo feature, off unless the build asks for it.
3. **Default features off.** A dependency is declared with
   `default-features = false` and the smallest feature set that works.
   Enabling a feature is a dependency decision like any other.
4. **Prefer the standard library, then a workspace crate, then a small
   focused crate.** No crate is added for code shorter than the review it
   would take to read it.
5. **No `unsafe` without a named owner.** Workspace lints deny `unsafe_code`.
   A crate that needs it is accepted only with the unsafe surface described
   in its issue and a test that exercises it.
6. **Compatible with upstream's MIT.** Permissive licenses are accepted, as
   is weak copyleft limited to the crate (MPL-2.0): it must not impose
   conditions on rpsh's own code. The allow list is in `deny.toml`.
7. **Maintenance is part of the decision.** A crate that is unmaintained,
   yanked, or has an open advisory is not admitted. `cargo deny check
   advisories` is the gate.

## Candidate areas

These are the crates the milestones are expected to need. None is admitted
yet; each gets its own issue and spike, and this table records the decision.

| Area | Candidates | Where it sits | What decides it |
|---|---|---|---|
| regex | `regex`, `regex-automata` | `rpsh-runtime` default | #41: startup cost, binary size, and whether the .NET regex dialect needs a custom front-end |
| Unicode / culture | `icu` crates, system ICU via `libicu` | `rpsh-runtime` feature | #36: data size (ICU data is large), startup cost, whether the system ICU the PINNED pwsh already uses is enough |
| line editor | `reedline`, a custom editor | `rpsh-host` feature | #111: interactive-only, so it must not be on the startup path; pull in the history/completion stack by measurement |
| OS interfaces | `libc`, `rustix`, `nix` | `rpsh-native` default | #73, #76: syscall coverage, unsafe surface, pipes and process control |
| HTTP | `reqwest`/`hyper`, `ureq` | `rpsh-commands` feature | #97: TLS stack (rustls or native), binary size, and whether web cmdlets stay a feature |
| JSON | `serde_json`, a custom writer | `rpsh-commands` feature or default | #91: exact formatting of `ConvertTo-Json`, startup cost, and the `-Depth` / escaping quirks |
| terminal / pty | `rustix`, `nix` | `rpsh-native` default | #77: pty allocation, signal delivery, terminal ownership |
| CLR bridge | `libloading` or hand-written `dlopen` | `rpsh-dotnet` feature | #125: `hostfxr` handling is small enough to keep explicit |

The rule for the two big data crates (ICU and the CLR bridge) is the same:
**do not put their data in the binary by default.** ICU is loaded from the
system or the reference pwsh; the CLR is found at run time; both fail with a
precise refusal when absent, never with an approximation.

## Adding a dependency

The procedure is part of the change, not an afterthought:

1. **Open the issue.** State the script or cmdlet that needs it, the
   alternatives (including "write it"), and which cargo feature it belongs
   to. A performance claim cites a result file.
2. **Spike it.** Add the crate on a branch, then measure:
   * `scripts/time-commands.py` for `rpsh -Version` and
     `rpsh -NoProfile -Command exit`, before and after;
   * `cargo build --release` and the binary size (the budget is #150);
   * peak RSS from the same run;
   * `cargo tree -i <crate>` to see what else it brings.
3. **Check the license and advisories.** `cargo deny check`. A new license
   goes into `deny.toml`'s allow list only with the issue that needed it.
4. **Declare it once.** Add the version to `[workspace.dependencies]`, never
   to an individual crate's `Cargo.toml`, so that every crate gets the same
   version and the duplicates check has one place to look.
5. **Gate it.** If it is not needed by `rpsh -Command exit`, put it behind a
   feature of the crate that uses it and make the feature off by default.
6. **Record it.** Add a row to the table above, or update the existing one
   with what the spike measured. `CONTRIBUTING.md` points here.
7. **Run the gate.** `scripts/check.sh` and `cargo deny check` both pass.

A dependency that fails step 2 is not admitted, even if the issue needs the
feature; the issue then says what the cost was and what the alternative is.

## Enforcement

* `deny.toml` is checked by `cargo deny check` (licenses, advisories, bans,
  sources) in CI's `dependencies` job.
* `scripts/check.sh` remains the local Rust and conformance gate; it does
  not run `cargo deny check`. `CONTRIBUTING.md` names both.
* `cargo tree --duplicates` is part of review. A duplicate is allowed only
  when two versions are genuinely required, and then it is recorded in
  `deny.toml`'s `skip`, with a comment giving the reason.
* `cargo deny`'s `advisories` check reads the RustSec advisory database, so
  it supersedes a separate `cargo audit`; #7's scope named both, and this is
  the one that runs. It is still a scope deviation from the issue, recorded
  here.
* CI's `dependencies` job runs on pushes to `main` and pull requests to
  `main`. While the M0 work merged through a non-default branch it did not
  run on those PRs; it runs from the first change on `main`.
* The graph target is `x86_64-unknown-linux-gnu`, the reference platform.
  Target-specific crates for arm64, macOS and Windows are checked when those
  targets enter CI.
