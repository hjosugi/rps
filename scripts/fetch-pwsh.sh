#!/usr/bin/env sh
# Download the latest stable upstream PowerShell release, the reference rpsh
# is measured against.
#
# The release is resolved from GitHub's latest-release redirect at fetch time.
# The archive is verified against the SHA-256 digest the release publishes and
# unpacked into .cache/pwsh/<version>/ (git-ignored). .cache/pwsh/current
# records which version is the reference; the harness reads it, and every
# result file and conformance golden records the version it actually ran.
#
# Usage: scripts/fetch-pwsh.sh            # install into <repo>/.cache/pwsh/<version>
#        RPS_PWSH_DIR=/some/dir scripts/fetch-pwsh.sh
set -eu

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    ASSET_SUFFIX=linux-x64.tar.gz
    ;;
  Linux-aarch64)
    ASSET_SUFFIX=linux-arm64.tar.gz
    ;;
  Darwin-arm64)
    ASSET_SUFFIX=osx-arm64.tar.gz
    ;;
  *)
    echo "fetch-pwsh: no PowerShell archive for $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
CACHE="$ROOT/.cache/pwsh"
DEST=${RPS_PWSH_DIR:-}

TAG=$(curl -fsSIL -o /dev/null -w '%{url_effective}' \
  https://github.com/PowerShell/PowerShell/releases/latest)
TAG=${TAG##*/}
VERSION=${TAG#v}
ASSET="powershell-${VERSION}-${ASSET_SUFFIX}"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

curl -fsSL -o "$TMP/hashes.sha256" \
  "https://github.com/PowerShell/PowerShell/releases/download/${TAG}/hashes.sha256"
SHA256=$(tr -d '\000\r' < "$TMP/hashes.sha256" |
  awk -v asset="$ASSET" '{ name = $2; sub(/^\*/, "", name); if (name == asset) print $1 }')
if [ -z "$SHA256" ]; then
  echo "fetch-pwsh: the release publishes no SHA-256 for $ASSET" >&2
  exit 1
fi

if [ -z "$DEST" ]; then
  DEST="$CACHE/$VERSION"
fi

if [ ! -x "$DEST/pwsh" ]; then
  echo "fetch-pwsh: downloading $ASSET"
  curl -fsSL -o "$TMP/$ASSET" \
    "https://github.com/PowerShell/PowerShell/releases/download/${TAG}/${ASSET}"

  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL=$(sha256sum "$TMP/$ASSET" | cut -d' ' -f1)
  else
    ACTUAL=$(shasum -a 256 "$TMP/$ASSET" | cut -d' ' -f1)
  fi
  if [ "$ACTUAL" != "$SHA256" ]; then
    echo "fetch-pwsh: SHA-256 mismatch for $ASSET" >&2
    echo "  expected $SHA256" >&2
    echo "  actual   $ACTUAL" >&2
    exit 1
  fi

  mkdir -p "$DEST"
  tar -xzf "$TMP/$ASSET" -C "$DEST"
  chmod +x "$DEST/pwsh"
  echo "fetch-pwsh: installed $DEST/pwsh"
else
  echo "fetch-pwsh: already installed: $DEST/pwsh"
fi

if [ -z "${RPS_PWSH_DIR:-}" ]; then
  mkdir -p "$CACHE"
  printf '%s\n' "$VERSION" > "$CACHE/current"
fi
echo "fetch-pwsh: reference is PowerShell $VERSION (latest stable)"
