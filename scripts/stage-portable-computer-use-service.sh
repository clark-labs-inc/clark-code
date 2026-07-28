#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=""
PROFILE="debug"

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
      echo "error: unknown argument $1" >&2
      exit 2
      ;;
  esac
done

case "$TARGET" in
  *-unknown-linux-gnu)
    SUFFIX=""
    ;;
  *-pc-windows-msvc)
    SUFFIX=".exe"
    ;;
  *)
    echo "error: portable Computer Use service target must be Linux GNU or Windows MSVC" >&2
    exit 2
    ;;
esac

case "$PROFILE" in
  debug)
    PROFILE_ARGS=()
    ;;
  release)
    PROFILE_ARGS=(--release)
    ;;
  *)
    echo "error: profile must be debug or release" >&2
    exit 2
    ;;
esac

cargo build \
  --manifest-path "$ROOT_DIR/Cargo.toml" \
  --target "$TARGET" \
  "${PROFILE_ARGS[@]}" \
  -p computer-use \
  --features helper-service \
  --bin clark-computer-use-helper

SOURCE="$ROOT_DIR/target/$TARGET/$PROFILE/clark-computer-use-helper$SUFFIX"
OUTPUT="$ROOT_DIR/src-tauri/binaries/clark-computer-use-helper-$TARGET$SUFFIX"
install -m 0755 "$SOURCE" "$OUTPUT"
"$OUTPUT" --self-test

echo "staged Computer Use service: $OUTPUT"
