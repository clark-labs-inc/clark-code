#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
APP_NAME="Clark Code Dev"
PROCESS_NAME="clark-desktop"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEFAULT_SIGNING_IDENTITY="CD6C1DB6CE507A17A49510839914CF22E85DAA1B"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
QA_MODE=false
BUILD_REQUIRED=true
BUILD_ONLY=false

case "$MODE" in
  run|--debug|debug|--logs|logs|--telemetry|telemetry|--verify|verify)
    PROFILE="debug"
    BUILD_ARGS=(--debug --bundles app)
    ;;
  --build|build)
    PROFILE="debug"
    BUILD_ARGS=(--debug --bundles app)
    BUILD_ONLY=true
    ;;
  --qa-verify|qa-verify)
    PROFILE="debug"
    BUILD_ARGS=(--debug --bundles app)
    QA_MODE=true
    ;;
  --qa-build|qa-build)
    PROFILE="debug"
    BUILD_ARGS=(--debug --bundles app)
    QA_MODE=true
    BUILD_ONLY=true
    ;;
  --qa-launch|qa-launch)
    PROFILE="debug"
    BUILD_ARGS=(--debug --bundles app)
    QA_MODE=true
    BUILD_REQUIRED=false
    ;;
  *)
    echo "usage: $0 [run|--build|--debug|--logs|--telemetry|--verify|--qa-build|--qa-launch|--qa-verify]" >&2
    exit 2
    ;;
esac

APP_BUNDLE="$ROOT_DIR/target/$PROFILE/bundle/macos/$APP_NAME.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/$PROCESS_NAME"
SERVICE_APP="$APP_BUNDLE/Contents/Resources/Clark Computer Use.app"
SERVICE_BINARY="$SERVICE_APP/Contents/MacOS/clark-computer-use-helper"
SERVICE_SWIFT_CONCURRENCY="$SERVICE_APP/Contents/Frameworks/libswift_Concurrency.dylib"
BUILD_CONFIGS=(
  --config src-tauri/tauri.sandbox.macos.conf.json
  --config src-tauri/tauri.computer-use.macos.conf.json
)
if [[ "$QA_MODE" == true ]]; then
  BUILD_CONFIGS+=(--config src-tauri/tauri.qa.macos.conf.json)
fi

if [[ "$BUILD_REQUIRED" == true ]]; then
  (
    cd "$ROOT_DIR"
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
      --env "CLARK_COMPUTER_USE_DATA_DIR=$QA_HOME/Library/Application Support/Clark Code/Computer Use" \
      "$APP_BUNDLE"
  else
    /usr/bin/open -n "$APP_BUNDLE"
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
