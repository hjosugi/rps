# Fuzzing

The tokenizer and parser take arbitrary bytes from scripts and the
command line. These targets assert that they do not panic:

```bash
cargo +nightly fuzz run tokenize
cargo +nightly fuzz run parse
```

`Cargo.toml` has its own workspace so `cargo test --workspace` never pulls in
libFuzzer. `.github/workflows/fuzz.yml` runs both for 60 seconds every Monday
and on demand.

## When a target finds something

1. Reproduce: `cargo +nightly fuzz run parse artifacts/parse/crash-...`.
2. Minimize: `cargo +nightly fuzz tmin parse artifacts/parse/crash-...`.
3. If the input is valid UTF-8, add it as a conformance case under
   `tests/conformance/cases/syntax/` and record the pwsh golden; otherwise
   add a unit test in the crate that exercises the same bytes and asserts the
   same refusal or error. Never delete the input; it is the regression test.
4. Keep the minimized file in `fuzz/regressions/` (committed) as well, so the
   target starts from it next time.

Differential fuzzing against the reference pwsh (`Parser.ParseInput`) is the
next step: the oracle needs a cheap way to ask pwsh whether an input parses,
which a per-input process does not provide.
