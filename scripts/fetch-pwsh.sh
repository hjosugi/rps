#!/usr/bin/env sh
# Download the pinned upstream PowerShell release that rps is measured against.
#
# The archive is verified against the SHA-256 digest published with the
# upstream release and unpacked into .cache/pwsh/<version>/ (git-ignored).
# `cargo xtask bench` and `cargo xtask conform record` pick it up from there.
#
# Usage: scripts/fetch-pwsh.sh            # install into <repo>/.cache/pwsh/<version>
#        RPS_PWSH_DIR=/some/dir scripts/fetch-pwsh.sh
set -eu

VERSION=7.7.0

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    ASSET="powershell-${VERSION}-linux-x64.tar.gz"
    SHA256=c89d9c682d2c403dfe726f3df948e359675235a5bd6fab535776de69729ca0cb
    ;;
  Linux-aarch64)
    ASSET="powershell-${VERSION}-linux-arm64.tar.gz"
    SHA256=e59c89776eaa59790d44b3edd14277435225ab08b6e7603763101b89de83d935
    ;;
  Darwin-arm64)
    ASSET="powershell-${VERSION}-osx-arm64.tar.gz"
    SHA256=76449f078f30193fce9cdf6610b77fda14f4224cafcbdf230245ec0dab9ef10d
    ;;
  *)
    echo "fetch-pwsh: no pinned PowerShell ${VERSION} archive for $(uname -s)-$(uname -m)" >&2
    exit 1
    ;;
esac

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DEST=${RPS_PWSH_DIR:-"$ROOT/.cache/pwsh/$VERSION"}

if [ -x "$DEST/pwsh" ]; then
  echo "fetch-pwsh: already installed: $DEST/pwsh"
  exit 0
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

echo "fetch-pwsh: downloading $ASSET"
curl -fsSL -o "$TMP/$ASSET" \
  "https://github.com/PowerShell/PowerShell/releases/download/v${VERSION}/${ASSET}"

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
