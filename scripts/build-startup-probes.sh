#!/bin/sh
# Builds the .NET startup probes of bench/startup/probes into .cache/probes.
#
# Each probe is pwsh with its application replaced: a hard-linked copy of the
# pinned pwsh directory in which pwsh.dll is a tiny program compiled from
# bench/startup/probes/<name>.cs. Starting it costs what starting pwsh costs
# up to the point the probe stops, with the same apphost, runtime, runtime
# configuration and assemblies (docs/05_startup.md).
#
# Needs the pinned pwsh (scripts/fetch-pwsh.sh), which also compiles the
# probes with the Roslyn compiler that ships with it
# (bench/startup/compile-probe.ps1).
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
pwsh_dir="$root/.cache/pwsh/7.7.0"
out="$root/.cache/probes"

if [ ! -x "$pwsh_dir/pwsh" ]; then
    echo "no pinned pwsh in $pwsh_dir; run scripts/fetch-pwsh.sh" >&2
    exit 1
fi

for source in "$root"/bench/startup/probes/*.cs; do
    name=$(basename "$source" .cs)
    target="$out/$name"
    rm -rf "$target"
    mkdir -p "$out"
    cp -al "$pwsh_dir" "$target"
    # Unlink before writing: writing through the hard link would change
    # the pinned pwsh itself.
    rm "$target/pwsh.dll"
    "$pwsh_dir/pwsh" -NoProfile -NonInteractive \
        -File "$root/bench/startup/compile-probe.ps1" \
        -Source "$source" -Assembly "$target/pwsh.dll" </dev/null
    "$target/pwsh" </dev/null
    echo "built $name"
done
