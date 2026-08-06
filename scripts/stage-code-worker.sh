#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=""
PROFILE="release"
TARGET_DIR="${CLARK_CODE_WORKER_TARGET_DIR:-$ROOT_DIR/target}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) TARGET="${2:-}"; shift 2 ;;
    --profile) PROFILE="${2:-}"; shift 2 ;;
    *) echo "usage: $0 --target <target> [--profile debug|release]" >&2; exit 2 ;;
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

HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
CARGO_SUBCOMMAND="build"
if [[ "$TARGET" != "$HOST_TARGET" && "$TARGET" == *-unknown-linux-musl ]]; then
  if ! cargo zigbuild --help >/dev/null 2>&1; then
    echo "error: cargo-zigbuild is required to stage $TARGET from $HOST_TARGET" >&2
    exit 1
  fi
  CARGO_SUBCOMMAND="zigbuild"
fi

CARGO_ARGS=("$CARGO_SUBCOMMAND" --locked -p code-worker --bin clark-code-worker --target "$TARGET")
if [[ "$PROFILE" == "release" ]]; then
  CARGO_ARGS+=(--release)
fi
CARGO_TARGET_DIR="$TARGET_DIR" cargo "${CARGO_ARGS[@]}"

SUFFIX=""
if [[ "$TARGET" == *-pc-windows-msvc ]]; then SUFFIX=".exe"; fi
SOURCE="$TARGET_DIR/$TARGET/$PROFILE/clark-code-worker$SUFFIX"
OUTPUT="$ROOT_DIR/src-tauri/binaries/clark-code-worker-$TARGET$SUFFIX"
if [[ ! -f "$SOURCE" ]]; then
  echo "error: Clark Code worker build did not produce $SOURCE" >&2
  exit 1
fi
mkdir -p "$(dirname "$OUTPUT")"
install -m 0755 "$SOURCE" "$OUTPUT"

if [[ "$TARGET" == "$HOST_TARGET" ]]; then
  SELF_TEST="$($OUTPUT --self-test)"
  if ! jq -e '
    .status == "passed"
    and .worker == "clark-code-worker"
    and .protocol_version == 2
  ' >/dev/null <<<"$SELF_TEST"; then
    echo "error: staged Clark Code worker failed its typed self-test" >&2
    exit 1
  fi
else
  FILE_KIND="$(file -b "$OUTPUT")"
  case "$TARGET" in
    x86_64-unknown-linux-musl)
      EXPECTED_KIND='ELF 64-bit LSB executable, x86-64'
      ;;
    aarch64-unknown-linux-musl)
      EXPECTED_KIND='ELF 64-bit LSB executable, ARM aarch64'
      ;;
    *)
      echo "error: cross-target worker validation is not defined for $TARGET" >&2
      exit 1
      ;;
  esac
  if [[ "$FILE_KIND" != *"$EXPECTED_KIND"* || "$FILE_KIND" != *"statically linked"* ]]; then
    echo "error: staged Clark Code worker has unexpected target metadata: $FILE_KIND" >&2
    exit 1
  fi
fi
echo "staged Clark Code worker: $OUTPUT"
