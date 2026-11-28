#!/usr/bin/env sh
# Install the optional RTK command-output filters in this checkout.
set -eu
version=v1.2.1
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) target=x86_64-unknown-linux-musl; digest=44aa460c4827d206d11405c652832c9f7082edccfbc3a9f15a69a851e8a9e8b4 ;;
    Linux-aarch64) target=aarch64-unknown-linux-musl; digest=b01860ae2d3f44e0f7fc4431a95dd66b29b4bd87ec7f5e60aff13681b05db5dd ;;
    Darwin-arm64) target=aarch64-apple-darwin; digest=5a37e5e8e016b559597024900e7b21cff7a0015aa189ac6ac80d461cbb4be65b ;;
    Darwin-x86_64) target=x86_64-apple-darwin; digest=5e39ee8c4613c798f8d62868d4851bacca5aefcc134301052841e458ed71c797 ;;
    *) echo "No pinned RTK OSS binary for this platform" >&2; exit 1 ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dest="$root/.cache/tools"
mkdir -p "$dest"
temp=$(mktemp -d "$dest/rtk-install.XXXXXX")
trap 'rm -rf "$temp"' EXIT
archive="$dest/rtk-oss-$version-$target.tar.gz"
if [ -f "$archive" ]; then
    cp "$archive" "$temp/archive"
else
    curl -fsSL --retry 2 -o "$temp/archive" \
        "https://github.com/rtk-ai/rtk/releases/download/$version/rtk-oss-$target.tar.gz"
fi
(
    cd "$temp"
    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s  archive\n' "$digest" | sha256sum -c -
    else
        printf '%s  archive\n' "$digest" | shasum -a 256 -c -
    fi
    tar -xzf archive rtk LICENSE
    chmod 755 rtk
    ./rtk --version
)
mv -f "$temp/archive" "$archive"
mv -f "$temp/LICENSE" "$dest/rtk-LICENSE"
mv -f "$temp/rtk" "$dest/rtk"
