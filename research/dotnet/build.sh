#!/bin/sh
# Builds the T1 probe of docs/06_dotnet_bridge.md into .cache/dotnet-probe:
# the native host (hostfxr-probe.c) and the managed library it calls
# (Probe.cs, compiled by the Roslyn that ships with the pinned pwsh).
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
pwsh_dir="$root/.cache/pwsh/7.7.0"
out="$root/.cache/dotnet-probe"

if [ ! -x "$pwsh_dir/pwsh" ]; then
    echo "no pinned pwsh in $pwsh_dir; run scripts/fetch-pwsh.sh" >&2
    exit 1
fi
mkdir -p "$out"
"${CC:-cc}" -O2 -Wall -Wextra -o "$out/hostfxr-probe" "$root/research/dotnet/hostfxr-probe.c" -ldl
"$pwsh_dir/pwsh" -NoProfile -NonInteractive \
    -File "$root/research/dotnet/compile-library.ps1" \
    -Source "$root/research/dotnet/Probe.cs" -Assembly "$out/probe.dll" </dev/null
echo "built $out/hostfxr-probe and $out/probe.dll"
