#!/usr/bin/env sh
# Download the initial analysis corpus: the PowerShell test suite at the
# latest stable tag. The corpus itself is not committed; only the fetch
# script and the frequency table in docs/08_corpus.md are.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TAG=$(curl -fsSIL -o /dev/null -w '%{url_effective}' \
  https://github.com/PowerShell/PowerShell/releases/latest)
TAG=${TAG##*/}
VERSION=${TAG#v}
ARCHIVE="$ROOT/.cache/corpus/PowerShell-${TAG}.tar.gz"
SOURCE="$ROOT/.cache/corpus/PowerShell-${VERSION}"

if [ -d "$SOURCE/test/powershell" ]; then
  echo "fetch-corpus: already present: $SOURCE/test/powershell"
  exit 0
fi

mkdir -p "$ROOT/.cache/corpus"
if [ ! -f "$ARCHIVE" ]; then
  echo "fetch-corpus: downloading PowerShell ${TAG}"
  curl -fsSL -o "$ARCHIVE" \
    "https://codeload.github.com/PowerShell/PowerShell/tar.gz/refs/tags/${TAG}"
fi
tar -xzf "$ARCHIVE" -C "$ROOT/.cache/corpus"
echo "fetch-corpus: extracted $SOURCE/test/powershell"
