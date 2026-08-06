#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_DIR="${CLARK_SCIENTIST_SOURCE:-$ROOT_DIR/../clark-scientist}"
AUTORESEARCH_SOURCE="${CLARK_AUTORESEARCH_SOURCE:-}"
SCIENTIST_TARGET_DIR="${CLARK_SCIENTIST_TARGET_DIR:-$ROOT_DIR/target}"
TARGET=""
PROFILE="release"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source)
      SOURCE_DIR="${2:-}"
      shift 2
      ;;
    --autoresearch-source)
      AUTORESEARCH_SOURCE="${2:-}"
      shift 2
      ;;
    --target)
      TARGET="${2:-}"
      shift 2
      ;;
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    *)
      echo "usage: $0 --target <target> [--source PATH] [--autoresearch-source PATH] [--profile debug|release]" >&2
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
if [[ ! -f "$SOURCE_DIR/Cargo.toml" ]]; then
  echo "error: Clark Scientist source is unavailable at $SOURCE_DIR" >&2
  exit 1
fi
mkdir -p "$SCIENTIST_TARGET_DIR"
if command -v cygpath >/dev/null 2>&1; then
  SCIENTIST_TARGET_DIR="$(cygpath -m "$SCIENTIST_TARGET_DIR")"
else
  SCIENTIST_TARGET_DIR="$(cd "$SCIENTIST_TARGET_DIR" && pwd)"
fi

HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
CARGO_SUBCOMMAND="build"
if [[ "$TARGET" != "$HOST_TARGET" && "$TARGET" == *-unknown-linux-musl ]]; then
  if ! cargo zigbuild --help >/dev/null 2>&1; then
    echo "error: cargo-zigbuild is required to stage $TARGET from $HOST_TARGET" >&2
    exit 1
  fi
  if command -v rustup >/dev/null 2>&1 \
    && ! rustup target list --installed | grep -Fqx "$TARGET"; then
    rustup target add "$TARGET"
  fi
  CARGO_SUBCOMMAND="zigbuild"
fi
CARGO_ARGS=("$CARGO_SUBCOMMAND" --locked)
AUTORESEARCH_GIT_URL=""
if [[ -n "$AUTORESEARCH_SOURCE" ]]; then
  if [[ ! -f "$AUTORESEARCH_SOURCE/Cargo.toml" ]]; then
    echo "error: Clark Autoresearch source is unavailable at $AUTORESEARCH_SOURCE" >&2
    exit 1
  fi
  if command -v cygpath >/dev/null 2>&1; then
    AUTORESEARCH_SOURCE="$(cygpath -m "$AUTORESEARCH_SOURCE")"
  else
    AUTORESEARCH_SOURCE="$(cd "$AUTORESEARCH_SOURCE" && pwd)"
  fi
  AUTORESEARCH_GIT_URL="file://$AUTORESEARCH_SOURCE"
fi

case "$TARGET" in
  *-pc-windows-msvc)
    SUFFIX=".exe"
    ;;
  *-apple-darwin|*-unknown-linux-gnu|*-unknown-linux-musl|universal-apple-darwin)
    SUFFIX=""
    ;;
  *)
    echo "error: unsupported Clark Scientist worker target $TARGET" >&2
    exit 2
    ;;
esac

OUTPUT="$ROOT_DIR/src-tauri/binaries/clark-code-headless-$TARGET$SUFFIX"
mkdir -p "$(dirname "$OUTPUT")"

if [[ "$TARGET" == "universal-apple-darwin" ]]; then
  ARM="$ROOT_DIR/src-tauri/binaries/clark-code-headless-aarch64-apple-darwin"
  INTEL="$ROOT_DIR/src-tauri/binaries/clark-code-headless-x86_64-apple-darwin"
  if [[ ! -x "$ARM" || ! -x "$INTEL" ]]; then
    echo "error: stage both Darwin architectures before the universal worker" >&2
    exit 1
  fi
  /usr/bin/lipo -create "$ARM" "$INTEL" -output "$OUTPUT"
  chmod 0755 "$OUTPUT"
else
  CARGO_ARGS+=(
    --manifest-path "$SOURCE_DIR/Cargo.toml"
    --target "$TARGET"
  )
  if [[ "$PROFILE" == "release" ]]; then
    CARGO_ARGS+=(--release)
  fi
  CARGO_ARGS+=(
    -p code-worker
    --bin clark-code-headless
  )
  if [[ -n "$AUTORESEARCH_GIT_URL" ]]; then
    GIT_CONFIG_COUNT=2 \
      GIT_CONFIG_KEY_0="url.$AUTORESEARCH_GIT_URL.insteadOf" \
      GIT_CONFIG_VALUE_0="https://github.com/clark-labs-inc/clark-autoresearch" \
      GIT_CONFIG_KEY_1="protocol.file.allow" \
      GIT_CONFIG_VALUE_1="always" \
      CARGO_NET_GIT_FETCH_WITH_CLI=true \
      CARGO_TARGET_DIR="$SCIENTIST_TARGET_DIR" \
      cargo "${CARGO_ARGS[@]}"
  else
    CARGO_TARGET_DIR="$SCIENTIST_TARGET_DIR" cargo "${CARGO_ARGS[@]}"
  fi
  SOURCE="$SCIENTIST_TARGET_DIR/$TARGET/$PROFILE/clark-code-headless$SUFFIX"
  if [[ ! -f "$SOURCE" ]]; then
    echo "error: Clark Scientist build did not produce $SOURCE" >&2
    exit 1
  fi
  install -m 0755 "$SOURCE" "$OUTPUT"
fi

if [[ "$TARGET" == x86_64-unknown-linux-* && "$HOST_TARGET" != "$TARGET" ]]; then
  if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    echo "error: Docker is required to self-test the cross-built $TARGET worker" >&2
    exit 1
  fi
  SELF_TEST="$(
    docker run --rm --platform linux/amd64 \
      --volume "$(dirname "$OUTPUT"):/worker:ro" \
      alpine:3.20 \
      "/worker/$(basename "$OUTPUT")" --self-test
  )"
else
  SELF_TEST="$("$OUTPUT" --self-test)"
fi
if ! jq -e '
  .status == "passed"
  and .worker == "clark-code-headless"
  and .protocol_version == 1
  and .specialist_count == 2
  and (.catalog_sha256 | type == "string" and length == 64)
' >/dev/null <<<"$SELF_TEST"; then
  echo "error: staged Clark Scientist worker failed its typed self-test" >&2
  exit 1
fi
echo "staged Clark Scientist worker: $OUTPUT"
