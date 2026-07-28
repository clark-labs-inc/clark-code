#!/usr/bin/env bash
set -euo pipefail

TARGET=""
PROFILE="release"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      TARGET="${2:-}"
      shift 2
      ;;
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    *)
      echo "usage: $0 --target <apple-darwin target> [--profile debug|release]" >&2
      exit 2
      ;;
  esac
done

if [[ "$TARGET" != *-apple-darwin ]] && [[ "$TARGET" != "universal-apple-darwin" ]]; then
  echo "error: computer-use helper only supports Apple Darwin targets" >&2
  exit 2
fi
if [[ "$PROFILE" != "debug" ]] && [[ "$PROFILE" != "release" ]]; then
  echo "error: profile must be debug or release" >&2
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="$ROOT_DIR/src-tauri/binaries"
OUTPUT="$OUTPUT_DIR/clark-computer-use-helper-$TARGET"
SERVICE_APP="$OUTPUT_DIR/Clark Computer Use.app"
SERVICE_CONTENTS="$SERVICE_APP/Contents"
SERVICE_MACOS="$SERVICE_CONTENTS/MacOS"
SERVICE_FRAMEWORKS="$SERVICE_CONTENTS/Frameworks"
SERVICE_RESOURCES="$SERVICE_CONTENTS/Resources"
SERVICE_EXECUTABLE="$SERVICE_MACOS/clark-computer-use-helper"
SWIFT_CONCURRENCY="/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx/libswift_Concurrency.dylib"
# Prefer the OS Swift runtime when it is present in the dyld shared cache.
# Keep the bundled Frameworks fallback for older supported macOS versions.
HELPER_RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,/usr/lib/swift -C link-arg=-Wl,-rpath,@executable_path/../Frameworks"
mkdir -p "$OUTPUT_DIR"

if [[ "$TARGET" == "universal-apple-darwin" ]]; then
  ARM="$OUTPUT_DIR/clark-computer-use-helper-aarch64-apple-darwin"
  INTEL="$OUTPUT_DIR/clark-computer-use-helper-x86_64-apple-darwin"
  if [[ ! -x "$ARM" ]] || [[ ! -x "$INTEL" ]]; then
    echo "error: stage aarch64 and x86_64 helpers before the universal helper" >&2
    exit 1
  fi
  /usr/bin/lipo -create "$ARM" "$INTEL" -output "$OUTPUT"
  chmod 755 "$OUTPUT"
else
  (
    cd "$ROOT_DIR"
    if [[ "$PROFILE" == "release" ]]; then
      RUSTFLAGS="$HELPER_RUSTFLAGS" cargo build \
        --release \
        --target "$TARGET" \
        -p computer-use \
        --features helper-service \
        --bin clark-computer-use-helper
    else
      RUSTFLAGS="$HELPER_RUSTFLAGS" cargo build \
        --target "$TARGET" \
        -p computer-use \
        --features helper-service \
        --bin clark-computer-use-helper
    fi
  )

  SOURCE="$ROOT_DIR/target/$TARGET/$PROFILE/clark-computer-use-helper"
  if [[ ! -x "$SOURCE" ]]; then
    echo "error: helper build did not produce $SOURCE" >&2
    exit 1
  fi
  install -m 755 "$SOURCE" "$OUTPUT"
fi

if [[ ! -f "$SWIFT_CONCURRENCY" ]]; then
  echo "error: Swift concurrency runtime is missing at $SWIFT_CONCURRENCY" >&2
  exit 1
fi
rm -rf -- "$SERVICE_APP"
mkdir -p "$SERVICE_MACOS" "$SERVICE_FRAMEWORKS" "$SERVICE_RESOURCES"
cp "$ROOT_DIR/harness/fixtures/computer-use-service/Info.plist" "$SERVICE_CONTENTS/Info.plist"
install -m 755 "$OUTPUT" "$SERVICE_EXECUTABLE"
cp "$SWIFT_CONCURRENCY" "$SERVICE_FRAMEWORKS/libswift_Concurrency.dylib"
cp "$ROOT_DIR/src-tauri/icons/icon.icns" "$SERVICE_RESOURCES/ClarkComputerUse.icns"

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n 1)"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read workspace version from Cargo.toml" >&2
  exit 1
fi
if [[ "$PROFILE" == "debug" ]]; then
  SERVICE_ID="com.clark.computer-use.dev"
  SERVICE_NAME="Clark Computer Use Dev"
else
  SERVICE_ID="com.clark.computer-use"
  SERVICE_NAME="Clark Computer Use"
fi
/usr/bin/plutil -replace CFBundleIdentifier -string "$SERVICE_ID" "$SERVICE_CONTENTS/Info.plist"
/usr/bin/plutil -replace CFBundleDisplayName -string "$SERVICE_NAME" "$SERVICE_CONTENTS/Info.plist"
/usr/bin/plutil -replace CFBundleName -string "$SERVICE_NAME" "$SERVICE_CONTENTS/Info.plist"
/usr/bin/plutil -replace CFBundleShortVersionString -string "$VERSION" "$SERVICE_CONTENTS/Info.plist"
/usr/bin/plutil -replace CFBundleVersion -string "${VERSION//./}" "$SERVICE_CONTENTS/Info.plist"
/usr/bin/plutil -lint "$SERVICE_CONTENTS/Info.plist" >/dev/null
