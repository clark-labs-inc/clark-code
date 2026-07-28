#!/usr/bin/env bash
set -euo pipefail

# Tauri treats the Computer Use service as a bundled resource rather than an
# executable it owns. Sign its nested Mach-O code before Tauri signs and
# notarizes the outer Clark Code app.
if [[ "$(uname -s)" != "Darwin" ]]; then
  exit 0
fi

SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
if [[ -z "$SIGNING_IDENTITY" ]]; then
  echo "APPLE_SIGNING_IDENTITY is unset; leaving Computer Use service unsigned"
  exit 0
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVICE_APP="$ROOT_DIR/src-tauri/binaries/Clark Computer Use.app"
SERVICE_EXECUTABLE="$SERVICE_APP/Contents/MacOS/clark-computer-use-helper"
SWIFT_RUNTIME="$SERVICE_APP/Contents/Frameworks/libswift_Concurrency.dylib"

for component in "$SERVICE_EXECUTABLE" "$SWIFT_RUNTIME"; do
  if [[ ! -f "$component" ]]; then
    echo "required Computer Use service component is missing: $component" >&2
    exit 1
  fi
  /usr/bin/codesign --force --sign "$SIGNING_IDENTITY" --options runtime --timestamp "$component"
done

/usr/bin/codesign --force --sign "$SIGNING_IDENTITY" --options runtime --timestamp "$SERVICE_APP"
/usr/bin/codesign --verify --deep --strict --verbose=4 "$SERVICE_APP"
