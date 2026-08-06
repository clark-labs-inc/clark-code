#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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
      echo "usage: $0 --target TARGET [--profile debug|release]" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$TARGET" ]]; then
  echo "error: --target is required" >&2
  exit 2
fi
if [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
  echo "error: profile must be debug or release" >&2
  exit 2
fi

PROFILE_FLAG=()
if [[ "$PROFILE" == "release" ]]; then
  PROFILE_FLAG=(--release)
fi

STAGE_ROOT="$ROOT_DIR/target/clark-cli-release/$TARGET/package"
BIN_DIR="$STAGE_ROOT/bin"
mkdir -p "$BIN_DIR"

case "$TARGET" in
  universal-apple-darwin)
    cargo build --locked "${PROFILE_FLAG[@]}" -p clark-cli --target aarch64-apple-darwin
    cargo build --locked "${PROFILE_FLAG[@]}" -p clark-cli --target x86_64-apple-darwin
    /usr/bin/lipo -create \
      "$ROOT_DIR/target/aarch64-apple-darwin/$PROFILE/clark" \
      "$ROOT_DIR/target/x86_64-apple-darwin/$PROFILE/clark" \
      -output "$BIN_DIR/clark"
    WORKER="$ROOT_DIR/src-tauri/binaries/clark-code-headless-universal-apple-darwin"
    cp "$WORKER" "$BIN_DIR/clark-code-headless"
    chmod 0755 "$BIN_DIR/clark" "$BIN_DIR/clark-code-headless"
    ;;
  *-pc-windows-msvc)
    cargo build --locked "${PROFILE_FLAG[@]}" -p clark-cli --target "$TARGET"
    cp "$ROOT_DIR/target/$TARGET/$PROFILE/clark.exe" "$BIN_DIR/clark.exe"
    cp "$ROOT_DIR/src-tauri/binaries/clark-code-headless-$TARGET.exe" \
      "$BIN_DIR/clark-code-headless.exe"
    ;;
  *-unknown-linux-gnu)
    cargo build --locked "${PROFILE_FLAG[@]}" -p clark-cli --target "$TARGET"
    cp "$ROOT_DIR/target/$TARGET/$PROFILE/clark" "$BIN_DIR/clark"
    cp "$ROOT_DIR/src-tauri/binaries/clark-code-headless-$TARGET" \
      "$BIN_DIR/clark-code-headless"
    chmod 0755 "$BIN_DIR/clark" "$BIN_DIR/clark-code-headless"
    ;;
  *)
    echo "error: unsupported Clark CLI target $TARGET" >&2
    exit 2
    ;;
esac

CLI="$BIN_DIR/clark"
WORKER="$BIN_DIR/clark-code-headless"
if [[ "$TARGET" == *-pc-windows-msvc ]]; then
  CLI="$CLI.exe"
  WORKER="$WORKER.exe"
fi

"$CLI" --version
"$WORKER" --self-test
echo "staged standalone Clark CLI: $STAGE_ROOT"
