#!/usr/bin/env bash
set -euo pipefail

OUTPUT_DIR=""
SIGNING_IDENTITY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      OUTPUT_DIR="${2:-}"
      shift 2
      ;;
    --signing-identity)
      SIGNING_IDENTITY="${2:-}"
      shift 2
      ;;
    *)
      echo "usage: $0 --output <directory> --signing-identity <codesign identity>" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: the native computer-use fixture is macOS-only" >&2
  exit 2
fi
if [[ -z "$OUTPUT_DIR" ]] || [[ -z "$SIGNING_IDENTITY" ]]; then
  echo "error: --output and --signing-identity are required" >&2
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_ROOT="$ROOT_DIR/harness/fixtures/computer-use-native"
APP="$OUTPUT_DIR/Clark Computer Use Fixture.app"
EXECUTABLE="$APP/Contents/MacOS/clark-computer-use-fixture"

mkdir -p "$APP/Contents/MacOS"
cp "$FIXTURE_ROOT/Info.plist" "$APP/Contents/Info.plist"
xcrun swiftc \
  -framework AppKit \
  "$FIXTURE_ROOT/ComputerUseFixture.swift" \
  -o "$EXECUTABLE"
chmod 755 "$EXECUTABLE"

/usr/bin/codesign \
  --force \
  --options runtime \
  --timestamp=none \
  --sign "$SIGNING_IDENTITY" \
  "$APP"
/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP"
/usr/bin/plutil -extract ClarkDebugOnlyFixture raw "$APP/Contents/Info.plist" |
  /usr/bin/grep -qx true

printf '%s\n' "$APP"
