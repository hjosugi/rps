#!/bin/sh
# Builds the T1 probe of docs/06_dotnet_bridge.md into .cache/dotnet-probe:
# the native host (hostfxr-probe.c) and the managed library it calls
# (Probe.cs, compiled by the Roslyn that ships with the reference pwsh).
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
version=$(cat "$root/.cache/pwsh/current" 2>/dev/null || true)
pwsh_dir="$root/.cache/pwsh/$version"
out="$root/.cache/dotnet-probe"

if [ -z "$version" ] || [ ! -x "$pwsh_dir/pwsh" ]; then
    echo "no latest stable pwsh in .cache/pwsh; run scripts/fetch-pwsh.sh" >&2
    exit 1
fi
mkdir -p "$out"
"${CC:-cc}" -O2 -Wall -Wextra -o "$out/hostfxr-probe" "$root/research/dotnet/hostfxr-probe.c" -ldl
"$pwsh_dir/pwsh" -NoProfile -NonInteractive \
    -File "$root/research/dotnet/compile-library.ps1" \
    -Source "$root/research/dotnet/Probe.cs" -Assembly "$out/probe.dll" </dev/null
echo "built $out/hostfxr-probe and $out/probe.dll"
