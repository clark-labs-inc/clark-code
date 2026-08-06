#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      TARGET="${2:-}"
      shift 2
      ;;
    *)
      echo "usage: $0 --target TARGET" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$TARGET" ]]; then
  echo "error: --target is required" >&2
  exit 2
fi

RELEASE_ROOT="$ROOT_DIR/target/clark-cli-release/$TARGET"
PACKAGE_ROOT="$RELEASE_ROOT/package"
if [[ ! -d "$PACKAGE_ROOT/bin" ]]; then
  echo "error: staged Clark CLI package is missing for $TARGET" >&2
  exit 1
fi

case "$TARGET" in
  universal-apple-darwin|*-pc-windows-msvc)
    ARCHIVE="$RELEASE_ROOT/clark-$TARGET.zip"
    rm -f "$ARCHIVE"
    if command -v powershell.exe >/dev/null 2>&1; then
      PACKAGE_WINDOWS="$(cygpath -w "$PACKAGE_ROOT")"
      ARCHIVE_WINDOWS="$(cygpath -w "$ARCHIVE")"
      powershell.exe -NoProfile -NonInteractive -Command \
        "Compress-Archive -Path '$PACKAGE_WINDOWS\\*' -DestinationPath '$ARCHIVE_WINDOWS' -CompressionLevel Optimal"
    else
      (cd "$PACKAGE_ROOT" && zip -qry "$ARCHIVE" bin)
    fi
    ;;
  *-unknown-linux-gnu)
    ARCHIVE="$RELEASE_ROOT/clark-$TARGET.tar.gz"
    rm -f "$ARCHIVE"
    tar -czf "$ARCHIVE" -C "$PACKAGE_ROOT" bin
    ;;
  *)
    echo "error: unsupported Clark CLI target $TARGET" >&2
    exit 2
    ;;
esac

test -s "$ARCHIVE"
echo "packaged standalone Clark CLI: $ARCHIVE"
