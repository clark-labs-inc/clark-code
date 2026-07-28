#!/usr/bin/env bash
set -euo pipefail

SERVICE_APP=""
PREVIOUS_APP=""
EXPECTED_ID=""
EXPECTED_TEAM=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --service-app)
      SERVICE_APP="${2:-}"
      shift 2
      ;;
    --previous-app)
      PREVIOUS_APP="${2:-}"
      shift 2
      ;;
    --expected-id)
      EXPECTED_ID="${2:-}"
      shift 2
      ;;
    --expected-team)
      EXPECTED_TEAM="${2:-}"
      shift 2
      ;;
    *)
      echo "usage: $0 --service-app <app> --expected-id <id> --expected-team <team> [--previous-app <app>]" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: service identity verification requires macOS" >&2
  exit 2
fi
if [[ -z "$SERVICE_APP" || -z "$EXPECTED_ID" || -z "$EXPECTED_TEAM" ]]; then
  echo "error: --service-app, --expected-id, and --expected-team are required" >&2
  exit 2
fi

inspect() {
  local app="$1"
  local actual_id actual_team requirement executable
  [[ -d "$app" ]] || {
    echo "error: service app is missing at $app" >&2
    return 1
  }
  /usr/bin/codesign --verify --deep --strict --verbose=2 "$app"
  actual_id="$(
    /usr/bin/codesign -dv --verbose=4 "$app" 2>&1 \
      | sed -n 's/^Identifier=//p' \
      | head -n 1
  )"
  actual_team="$(
    /usr/bin/codesign -dv --verbose=4 "$app" 2>&1 \
      | sed -n 's/^TeamIdentifier=//p' \
      | head -n 1
  )"
  [[ "$actual_id" == "$EXPECTED_ID" ]] || {
    echo "error: service identifier is $actual_id, expected $EXPECTED_ID" >&2
    return 1
  }
  [[ "$actual_team" == "$EXPECTED_TEAM" ]] || {
    echo "error: service team is $actual_team, expected $EXPECTED_TEAM" >&2
    return 1
  }
  [[ "$(/usr/libexec/PlistBuddy -c 'Print :LSUIElement' "$app/Contents/Info.plist")" == "true" ]] || {
    echo "error: service must remain an LSUIElement background app" >&2
    return 1
  }
  requirement="$(/usr/bin/codesign -d -r- "$app" 2>&1 | sed -n 's/^designated => //p')"
  [[ "$requirement" == *"identifier \"$EXPECTED_ID\""* ]] || {
    echo "error: designated requirement does not pin $EXPECTED_ID" >&2
    return 1
  }
  [[ "$requirement" != *"cdhash"* ]] || {
    echo "error: designated requirement is pinned to an update-unstable cdhash" >&2
    return 1
  }
  executable="$app/Contents/MacOS/clark-computer-use-helper"
  [[ -x "$executable" ]] || {
    echo "error: service executable is missing at $executable" >&2
    return 1
  }
  "$executable" --self-test
  printf '%s\n' "$requirement"
}

current_requirement="$(inspect "$SERVICE_APP")"
if [[ -n "$PREVIOUS_APP" ]]; then
  previous_requirement="$(inspect "$PREVIOUS_APP")"
  if [[ "$previous_requirement" != "$current_requirement" ]]; then
    echo "error: service designated requirement changed across the update" >&2
    exit 1
  fi
fi

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 \
    | sed -n 's/^CDHash=//p' \
    | head -n 1
}

current_cdhash="$(
  cdhash "$SERVICE_APP"
)"
if [[ -n "$PREVIOUS_APP" ]]; then
  previous_cdhash="$(cdhash "$PREVIOUS_APP")"
  if [[ "$previous_cdhash" == "$current_cdhash" ]]; then
    echo "error: update qualification requires distinct signed service content" >&2
    exit 1
  fi
fi
printf 'service_identity=verified id=%s team=%s cdhash=%s update_requirement=%s\n' \
  "$EXPECTED_ID" \
  "$EXPECTED_TEAM" \
  "$current_cdhash" \
  "$([[ -n "$PREVIOUS_APP" ]] && printf stable || printf bounded)"
