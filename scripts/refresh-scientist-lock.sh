#!/usr/bin/env bash
set -euo pipefail

SOURCE_DIR=""
AUTORESEARCH_SOURCE=""

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
    *)
      echo "usage: $0 --source PATH --autoresearch-source PATH" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$SOURCE_DIR" || -z "$AUTORESEARCH_SOURCE" ]]; then
  echo "error: both Scientist and Autoresearch sources are required" >&2
  exit 2
fi
if [[ ! -f "$SOURCE_DIR/Cargo.toml" ]]; then
  echo "error: Clark Scientist source is unavailable at $SOURCE_DIR" >&2
  exit 1
fi
if [[ ! -f "$AUTORESEARCH_SOURCE/Cargo.toml" ]]; then
  echo "error: Clark Autoresearch source is unavailable at $AUTORESEARCH_SOURCE" >&2
  exit 1
fi

if command -v cygpath >/dev/null 2>&1; then
  SOURCE_DIR="$(cygpath -m "$SOURCE_DIR")"
  AUTORESEARCH_SOURCE="$(cygpath -m "$AUTORESEARCH_SOURCE")"
else
  SOURCE_DIR="$(cd "$SOURCE_DIR" && pwd)"
  AUTORESEARCH_SOURCE="$(cd "$AUTORESEARCH_SOURCE" && pwd)"
fi

# The official Scientist checkout is pinned and its lockfile is checked in.
# Bind the exact local Autoresearch checkout while refreshing only the lock
# entries needed by the Desktop path dependencies; the subsequent worker
# build remains --locked and therefore deterministic.
GIT_CONFIG_COUNT=2 \
  GIT_CONFIG_KEY_0="url.file://$AUTORESEARCH_SOURCE.insteadOf" \
  GIT_CONFIG_VALUE_0="https://github.com/clark-labs-inc/clark-autoresearch" \
  GIT_CONFIG_KEY_1="protocol.file.allow" \
  GIT_CONFIG_VALUE_1="always" \
  CARGO_NET_GIT_FETCH_WITH_CLI=true \
  cargo update --manifest-path "$SOURCE_DIR/Cargo.toml" --workspace
