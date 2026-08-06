#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
APP_NAME="Clark Code Dev"
PROCESS_NAME="clark-desktop"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Load only the native OAuth build settings from the ignored local env
# file. Values are treated as data, never sourced as shell code, and are not
# printed. Non-VITE names keep them out of the WebView bundle.
OAUTH_ENV_FILE="$ROOT_DIR/app/.env.local"
if [[ -f "$OAUTH_ENV_FILE" ]]; then
  while IFS='=' read -r key value; do
    value="${value%$'\r'}"
    if [[ "$value" == \"*\" && "$value" == *\" ]]; then
      value="${value:1:${#value}-2}"
    elif [[ "$value" == \'*\' && "$value" == *\' ]]; then
      value="${value:1:${#value}-2}"
    fi
    case "$key" in
      CLARK_AUTH_ORIGIN|CLARK_GOOGLE_DESKTOP_CLIENT_ID|CLARK_GOOGLE_DESKTOP_CLIENT_SECRET)
        export "$key=$value"
        ;;
    esac
  done < "$OAUTH_ENV_FILE"
fi
DEFAULT_SIGNING_IDENTITY="CD6C1DB6CE507A17A49510839914CF22E85DAA1B"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
TARGET_DIR="$(
  cd "$ROOT_DIR"
  cargo metadata --format-version 1 --no-deps |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
)"
CLARK_LOGS_DIR="${CLARK_LOGS:-${TMPDIR:-/tmp}/clark-logs}"
CLARK_CAPTURED_LOGS_DIR="${CLARK_CAPTURED_LOGS:-$CLARK_LOGS_DIR/captured}"
QA_MODE=false
BUILD_REQUIRED=true
BUILD_ONLY=false

case "$MODE" in
  run|--debug|debug|--logs|logs|--telemetry|telemetry|--verify|verify)
    PROFILE="debug"
    BUILD_ARGS=(--debug --features debug-diagnostics --bundles app)
    ;;
  --build|build)
    PROFILE="debug"
    BUILD_ARGS=(--debug --features debug-diagnostics --bundles app)
    BUILD_ONLY=true
    ;;
  --qa-verify|qa-verify)
    PROFILE="debug"
    BUILD_ARGS=(--debug --features debug-diagnostics --bundles app)
    QA_MODE=true
    ;;
  --qa-build|qa-build)
    PROFILE="debug"
    BUILD_ARGS=(--debug --features debug-diagnostics --bundles app)
    QA_MODE=true
    BUILD_ONLY=true
    ;;
  --qa-launch|qa-launch)
    PROFILE="debug"
    BUILD_ARGS=(--debug --features debug-diagnostics --bundles app)
    QA_MODE=true
    BUILD_REQUIRED=false
    ;;
  *)
    echo "usage: $0 [run|--build|--debug|--logs|--telemetry|--verify|--qa-build|--qa-launch|--qa-verify]" >&2
    exit 2
    ;;
esac

if [[ "$CLARK_LOGS_DIR" != /* ]]; then
  echo "error: CLARK_LOGS must be an absolute directory" >&2
  exit 1
fi
mkdir -p "$CLARK_LOGS_DIR"
if [[ -L "$CLARK_LOGS_DIR" || ! -d "$CLARK_LOGS_DIR" ]]; then
  echo "error: CLARK_LOGS must be a real directory, not a symlink: $CLARK_LOGS_DIR" >&2
  exit 1
fi
chmod 700 "$CLARK_LOGS_DIR"
export CLARK_LOGS="$CLARK_LOGS_DIR"

if [[ "$CLARK_CAPTURED_LOGS_DIR" != /* ]]; then
  echo "error: CLARK_CAPTURED_LOGS must be an absolute directory" >&2
  exit 1
fi
mkdir -p "$CLARK_CAPTURED_LOGS_DIR"
if [[ -L "$CLARK_CAPTURED_LOGS_DIR" || ! -d "$CLARK_CAPTURED_LOGS_DIR" ]]; then
  echo "error: CLARK_CAPTURED_LOGS must be a real directory, not a symlink: $CLARK_CAPTURED_LOGS_DIR" >&2
  exit 1
fi
chmod 700 "$CLARK_CAPTURED_LOGS_DIR"
export CLARK_CAPTURED_LOGS="$CLARK_CAPTURED_LOGS_DIR"

APP_BUNDLE="$TARGET_DIR/$PROFILE/bundle/macos/$APP_NAME.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/$PROCESS_NAME"
SERVICE_APP="$APP_BUNDLE/Contents/Resources/Clark Computer Use.app"
SERVICE_BINARY="$SERVICE_APP/Contents/MacOS/clark-computer-use-helper"
SERVICE_SWIFT_CONCURRENCY="$SERVICE_APP/Contents/Frameworks/libswift_Concurrency.dylib"
BUILD_CONFIGS=(
  --config src-tauri/tauri.sandbox.macos.conf.json
  --config src-tauri/tauri.computer-use.macos.conf.json
  --config src-tauri/tauri.remote-workers.dev.conf.json
)
if [[ "$QA_MODE" == true ]]; then
  BUILD_CONFIGS+=(--config src-tauri/tauri.qa.macos.conf.json)
fi

if [[ "$BUILD_REQUIRED" == true ]]; then
  (
    cd "$ROOT_DIR"
    SCIENTIST_SOURCE="${CLARK_SCIENTIST_SOURCE:-$ROOT_DIR/../clark-scientist}"
    # Scientist's worker links the current Desktop provider crates by path.
    # Refresh its combined development lock before freezing the source snapshot
    # so dependency changes and worker changes cannot silently diverge.
    cargo metadata \
      --format-version 1 \
      --manifest-path "$SCIENTIST_SOURCE/Cargo.toml" \
      >/dev/null
    repository_fingerprint() {
      local repository="$1"
      {
        git -C "$repository" rev-parse HEAD
        git -C "$repository" diff --binary HEAD
        while IFS= read -r -d '' path; do
          shasum -a 256 "$repository/$path"
        done < <(git -C "$repository" ls-files --others --exclude-standard -z)
      } | shasum -a 256 | awk '{print $1}'
    }
    worker_source_fingerprint() {
      {
        repository_fingerprint "$ROOT_DIR"
        repository_fingerprint "$SCIENTIST_SOURCE"
      } | shasum -a 256 | awk '{print $1}'
    }
    WORKER_SOURCE_FINGERPRINT="$(worker_source_fingerprint)"
    CLARK_SCIENTIST_TARGET_DIR="$TARGET_DIR/scientist" \
      bash scripts/stage-scientist-worker.sh \
        --source "$SCIENTIST_SOURCE" \
        --target "$HOST_TARGET" \
        --profile debug
    if [[ "$(worker_source_fingerprint)" != "$WORKER_SOURCE_FINGERPRINT" ]]; then
      echo "error: Clark Code or Scientist source changed while staging the local worker; rebuild from one stable snapshot" >&2
      exit 1
    fi
    CLARK_SCIENTIST_TARGET_DIR="$TARGET_DIR/scientist" \
      bash scripts/stage-scientist-worker.sh \
        --source "$SCIENTIST_SOURCE" \
        --target x86_64-unknown-linux-musl \
        --profile release
    CLARK_CODE_WORKER_TARGET_DIR="$TARGET_DIR/remote-code-worker" \
      bash scripts/stage-code-worker.sh \
        --target x86_64-unknown-linux-musl \
        --profile release
    if [[ "$(worker_source_fingerprint)" != "$WORKER_SOURCE_FINGERPRINT" ]]; then
      echo "error: Clark Code or Scientist source changed while staging the remote worker; rebuild from one stable snapshot" >&2
      exit 1
    fi
    python3 scripts/stage-bundled-tools.py --target "$HOST_TARGET" --force
    bash scripts/stage-computer-use-helper.sh --target "$HOST_TARGET" --profile "$PROFILE"
    cargo tauri build \
      "${BUILD_ARGS[@]}" \
      "${BUILD_CONFIGS[@]}"
  )
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
  SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-$DEFAULT_SIGNING_IDENTITY}"
  if security find-identity -v -p codesigning | grep -Fq "$SIGNING_IDENTITY"; then
    if [[ ! -x "$SERVICE_BINARY" ]]; then
      echo "error: bundled computer-use service is missing at $SERVICE_BINARY" >&2
      exit 1
    fi
    if [[ ! -f "$SERVICE_SWIFT_CONCURRENCY" ]]; then
      echo "error: service Swift concurrency runtime is missing at $SERVICE_SWIFT_CONCURRENCY" >&2
      exit 1
    fi
    if [[ "$BUILD_REQUIRED" == true ]]; then
      # The independently launched service owns macOS privacy grants. Sign its
      # nested runtime and app identity before sealing the Clark host bundle.
      codesign --force --options runtime --timestamp=none --sign "$SIGNING_IDENTITY" "$SERVICE_SWIFT_CONCURRENCY"
      codesign --force --options runtime --timestamp=none --sign "$SIGNING_IDENTITY" "$SERVICE_APP"
      codesign --force --options runtime --timestamp=none --sign "$SIGNING_IDENTITY" "$APP_BUNDLE"
    fi
    codesign --verify --deep --strict --verbose=2 "$APP_BUNDLE"
    "$SERVICE_BINARY" --self-test
  else
    echo "error: no stable macOS signing identity found; refusing to launch an identity that would lose privacy grants after the next rebuild" >&2
    echo "set APPLE_SIGNING_IDENTITY to an installed development certificate and retry" >&2
    exit 1
  fi
fi

pkill -x "$PROCESS_NAME" >/dev/null 2>&1 || true
if [[ "$BUILD_ONLY" == true ]]; then
  exit 0
fi

open_app() {
  if [[ "$QA_MODE" == true ]]; then
    if [[ -z "${CLARK_QA_MACOS_HOME:-}" ]]; then
      echo "error: CLARK_QA_MACOS_HOME is required for the isolated QA profile" >&2
      exit 1
    fi
    if [[ ! -d "$CLARK_QA_MACOS_HOME" || -L "$CLARK_QA_MACOS_HOME" ]]; then
      echo "error: CLARK_QA_MACOS_HOME must be a real existing directory" >&2
      exit 1
    fi
    QA_HOME="$(cd "$CLARK_QA_MACOS_HOME" && pwd -P)"
    case "$QA_HOME" in
      "$ROOT_DIR"/target/*) ;;
      *)
        echo "error: CLARK_QA_MACOS_HOME must stay inside the repository target directory" >&2
        exit 1
        ;;
    esac
    QA_MODE_BITS="$(stat -f '%Lp' "$QA_HOME")"
    if (( (8#$QA_MODE_BITS & 077) != 0 )); then
      echo "error: CLARK_QA_MACOS_HOME must not be accessible by group or other users" >&2
      exit 1
    fi
    mkdir -p \
      "$QA_HOME/tmp" \
      "$QA_HOME/Library/Application Support/Clark Code/Computer Use"
    chmod 700 \
      "$QA_HOME/tmp" \
      "$QA_HOME/Library" \
      "$QA_HOME/Library/Application Support" \
      "$QA_HOME/Library/Application Support/Clark Code" \
      "$QA_HOME/Library/Application Support/Clark Code/Computer Use"
    /usr/bin/open -n \
      --env "HOME=$QA_HOME" \
      --env "CFFIXED_USER_HOME=$QA_HOME" \
      --env "TMPDIR=$QA_HOME/tmp" \
      --env "CLARK_LOGS=$CLARK_LOGS_DIR" \
      --env "CLARK_CAPTURED_LOGS=$CLARK_CAPTURED_LOGS_DIR" \
      --env "CLARK_COMPUTER_USE_DATA_DIR=$QA_HOME/Library/Application Support/Clark Code/Computer Use" \
      "$APP_BUNDLE"
  else
    /usr/bin/open -n \
      --env "CLARK_LOGS=$CLARK_LOGS_DIR" \
      --env "CLARK_CAPTURED_LOGS=$CLARK_CAPTURED_LOGS_DIR" \
      "$APP_BUNDLE"
  fi
}

case "$MODE" in
  --debug|debug)
    lldb -- "$APP_BINARY"
    ;;
  --logs|logs)
    open_app
    /usr/bin/log stream --info --style compact --predicate "process == \"$PROCESS_NAME\""
    ;;
  --telemetry|telemetry)
    open_app
    /usr/bin/log stream --info --style compact --predicate 'process == "clark-desktop" OR subsystem == "com.clark.desktop"'
    ;;
  --verify|verify|--qa-launch|qa-launch|--qa-verify|qa-verify)
    open_app
    sleep 2
    pgrep -x "$PROCESS_NAME" >/dev/null
    ;;
  run)
    open_app
    ;;
esac
