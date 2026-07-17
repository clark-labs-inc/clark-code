#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
APP_NAME="Clark Code"
PROCESS_NAME="clark-desktop"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL_BUNDLE_CONFIG='{"bundle":{"createUpdaterArtifacts":false}}'
DEFAULT_SIGNING_IDENTITY='Apple Development: skirdey@gmail.com (SNA4LQU6L9)'

case "$MODE" in
  run|--debug|debug|--logs|logs|--telemetry|telemetry|--verify|verify)
    PROFILE="debug"
    BUILD_ARGS=(--debug --bundles app --config "$LOCAL_BUNDLE_CONFIG")
    ;;
  *)
    echo "usage: $0 [run|--debug|--logs|--telemetry|--verify]" >&2
    exit 2
    ;;
esac

APP_BUNDLE="$ROOT_DIR/target/$PROFILE/bundle/macos/$APP_NAME.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/$PROCESS_NAME"

(
  cd "$ROOT_DIR"
  cargo tauri build "${BUILD_ARGS[@]}"
)

if [[ "$(uname -s)" == "Darwin" ]]; then
  SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-$DEFAULT_SIGNING_IDENTITY}"
  if security find-identity -v -p codesigning | grep -Fq "\"$SIGNING_IDENTITY\""; then
    codesign --force --options runtime --timestamp=none --sign "$SIGNING_IDENTITY" "$APP_BUNDLE"
  else
    echo "warning: no stable macOS signing identity found; privacy permissions may be requested again after rebuilds" >&2
  fi
fi

pkill -x "$PROCESS_NAME" >/dev/null 2>&1 || true

open_app() {
  /usr/bin/open -n "$APP_BUNDLE"
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
  --verify|verify)
    open_app
    sleep 2
    pgrep -x "$PROCESS_NAME" >/dev/null
    ;;
  run)
    open_app
    ;;
esac
