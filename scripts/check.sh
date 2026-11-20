#!/usr/bin/env sh
# The pre-PR gate. CI runs the same four commands.
set -eu
cd "$(dirname -- "$0")/.."

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo harness conform check
