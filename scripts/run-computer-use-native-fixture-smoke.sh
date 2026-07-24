#!/usr/bin/env bash
set -euo pipefail
umask 077

OUTPUT_DIR=""
SIGNING_IDENTITY=""
PHYSICAL_TAKEOVER=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      OUTPUT_DIR="${2:-}"
      shift 2
      ;;
    --signing-identity)
      SIGNING_IDENTITY="${2:-}"
      shift 2
      ;;
    --physical-takeover)
      PHYSICAL_TAKEOVER=1
      shift
      ;;
    *)
      echo "usage: $0 --output <directory> --signing-identity <codesign identity> [--physical-takeover]" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: the native computer-use fixture smoke is macOS-only" >&2
  exit 2
fi
if [[ -z "$OUTPUT_DIR" ]] || [[ -z "$SIGNING_IDENTITY" ]]; then
  echo "error: --output and --signing-identity are required" >&2
  exit 2
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "$(uname -m)" in
  arm64) TARGET="aarch64-apple-darwin" ;;
  x86_64) TARGET="x86_64-apple-darwin" ;;
  *)
    echo "error: unsupported macOS architecture $(uname -m)" >&2
    exit 2
    ;;
esac

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd -P)"
FIXTURE_APP="$(
  "$ROOT_DIR/scripts/build-computer-use-native-fixture.sh" \
    --output "$OUTPUT_DIR" \
    --signing-identity "$SIGNING_IDENTITY"
)"
HOST_APP="$OUTPUT_DIR/Clark Code Native Fixture Smoke.app"
HOST_MACOS="$HOST_APP/Contents/MacOS"
HOST_FRAMEWORKS="$HOST_APP/Contents/Frameworks"
SWIFT_CONCURRENCY="/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx/libswift_Concurrency.dylib"
HELPER_RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,/usr/lib/swift -C link-arg=-Wl,-rpath,@executable_path/../Frameworks"

(
  cd "$ROOT_DIR"
  RUSTFLAGS="$HELPER_RUSTFLAGS" cargo build \
    --target "$TARGET" \
    -p computer-use \
    --features helper-service \
    --example signed_helper_smoke
  RUSTFLAGS="$HELPER_RUSTFLAGS" cargo build \
    --target "$TARGET" \
    -p computer-use \
    --features helper-service \
    --bin clark-computer-use-helper
)

mkdir -p "$HOST_MACOS" "$HOST_FRAMEWORKS"
cp \
  "$ROOT_DIR/harness/fixtures/computer-use-native/HostInfo.plist" \
  "$HOST_APP/Contents/Info.plist"
install -m 755 \
  "$ROOT_DIR/target/$TARGET/debug/examples/signed_helper_smoke" \
  "$HOST_MACOS/clark-desktop"
install -m 755 \
  "$ROOT_DIR/target/$TARGET/debug/clark-computer-use-helper" \
  "$HOST_MACOS/clark-computer-use-helper"
cp "$SWIFT_CONCURRENCY" "$HOST_FRAMEWORKS/libswift_Concurrency.dylib"

/usr/bin/codesign \
  --force \
  --options runtime \
  --timestamp=none \
  --sign "$SIGNING_IDENTITY" \
  "$HOST_FRAMEWORKS/libswift_Concurrency.dylib"
/usr/bin/codesign \
  --force \
  --options runtime \
  --timestamp=none \
  --sign "$SIGNING_IDENTITY" \
  "$HOST_MACOS/clark-computer-use-helper"
/usr/bin/codesign \
  --force \
  --options runtime \
  --timestamp=none \
  --sign "$SIGNING_IDENTITY" \
  "$HOST_APP"

/usr/bin/codesign --verify --deep --strict --verbose=2 "$HOST_APP"
"$HOST_MACOS/clark-computer-use-helper" --self-test

FIXTURE_EXECUTABLE="$FIXTURE_APP/Contents/MacOS/clark-computer-use-fixture"

matching_fixture_pids() {
  local candidate_pid candidate_command
  while IFS= read -r candidate_pid; do
    [[ -n "$candidate_pid" ]] || continue
    candidate_command="$(ps -p "$candidate_pid" -o command= 2>/dev/null || true)"
    if [[ "$candidate_command" == "$FIXTURE_EXECUTABLE" ]]; then
      printf '%s\n' "$candidate_pid"
    fi
  done < <(pgrep -f clark-computer-use-fixture 2>/dev/null || true)
}

cleanup() {
  local fixture_pids fixture_pid attempt
  fixture_pids="$(matching_fixture_pids)"
  for fixture_pid in $fixture_pids; do
    kill -TERM "$fixture_pid" 2>/dev/null || true
  done
  for attempt in {1..20}; do
    [[ -z "$(matching_fixture_pids)" ]] && return
    sleep 0.1
  done
  fixture_pids="$(matching_fixture_pids)"
  for fixture_pid in $fixture_pids; do
    kill -KILL "$fixture_pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

/usr/bin/open -n "$FIXTURE_APP"
sleep 1
if [[ -z "$(matching_fixture_pids)" ]]; then
  echo "error: native fixture did not launch from $FIXTURE_EXECUTABLE" >&2
  exit 1
fi
FIXTURE_PID="$(matching_fixture_pids | head -n 1)"
if [[ "$PHYSICAL_TAKEOVER" == "1" ]]; then
  set +e
  CLARK_COMPUTER_USE_FIXTURE_BUNDLE_ID="com.clark.computer-use-fixture" \
    CLARK_COMPUTER_USE_FIXTURE_PID="$FIXTURE_PID" \
    CLARK_COMPUTER_USE_REQUIRE_PHYSICAL_TAKEOVER=1 \
    "$HOST_MACOS/clark-desktop" 2>&1 | tee "$OUTPUT_DIR/smoke.log"
  SMOKE_EXIT="${PIPESTATUS[0]}"
  set -e
else
  set +e
  CLARK_COMPUTER_USE_FIXTURE_BUNDLE_ID="com.clark.computer-use-fixture" \
    CLARK_COMPUTER_USE_FIXTURE_PID="$FIXTURE_PID" \
    "$HOST_MACOS/clark-desktop" 2>&1 | tee "$OUTPUT_DIR/smoke.log"
  SMOKE_EXIT="${PIPESTATUS[0]}"
  set -e
fi
chmod 600 "$OUTPUT_DIR/smoke.log"

SOURCE_REVISION="$(git -C "$ROOT_DIR" rev-parse HEAD)"
if [[ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]]; then
  SOURCE_DIRTY="true"
else
  SOURCE_DIRTY="false"
fi
RECEIPT_PATH="$OUTPUT_DIR/receipt.json"
python3 - \
  "$RECEIPT_PATH" \
  "$OUTPUT_DIR/smoke.log" \
  "$HOST_MACOS/clark-desktop" \
  "$HOST_MACOS/clark-computer-use-helper" \
  "$FIXTURE_EXECUTABLE" \
  "$TARGET" \
  "$SOURCE_REVISION" \
  "$SOURCE_DIRTY" \
  "$SIGNING_IDENTITY" \
  "$PHYSICAL_TAKEOVER" \
  "$SMOKE_EXIT" <<'PY'
import hashlib
import json
import pathlib
import re
import sys
from datetime import datetime, timezone

receipt_path = pathlib.Path(sys.argv[1])
log_path = pathlib.Path(sys.argv[2])
host_executable = pathlib.Path(sys.argv[3])
helper_executable = pathlib.Path(sys.argv[4])
fixture_executable = pathlib.Path(sys.argv[5])
target = sys.argv[6]
source_revision = sys.argv[7]
source_dirty = sys.argv[8] == "true"
signing_identity = sys.argv[9]
physical_takeover_required = sys.argv[10] == "1"
smoke_exit = int(sys.argv[11])

def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

log = log_path.read_text(errors="replace")
permission_match = re.search(
    r"^helper_permissions=accessibility:(true|false) screen_recording:(true|false)$",
    log,
    re.MULTILINE,
)
if permission_match is None:
    raise SystemExit("native smoke log lacks the permission result")
accessibility = permission_match.group(1) == "true"
screen_recording = permission_match.group(2) == "true"
action_match = re.search(
    r"^debug_fixture_actions=passed receipt=([^ ]+) "
    r"cancellation_helper_terminated=(true|false)$",
    log,
    re.MULTILINE,
)
actions_passed = action_match is not None
physical_takeover_passed = "physical_takeover=passed" in log
status = "passed" if accessibility and screen_recording and actions_passed else "blocked"

checks = {
    "codesign_verified": "passed",
    "helper_self_test": "passed",
    "consequential_dry_run": (
        "passed" if "consequential_dry_run=passed" in log else "failed"
    ),
    "forbidden_terminal": (
        "passed" if "forbidden_terminal=passed" in log else "failed"
    ),
    "forbidden_browser": (
        "passed" if "forbidden_browser=passed" in log else "failed"
    ),
    "fixture_launch": (
        "passed" if "debug_fixture_launch=passed" in log else "failed"
    ),
    "fixture_actions": "passed" if actions_passed else "blocked",
    "smoke_process": "passed" if smoke_exit == 0 else "failed",
}
if any(value == "failed" for value in checks.values()):
    status = "failed"
if physical_takeover_required and not physical_takeover_passed:
    status = "failed"

receipt = {
    "schema_version": 1,
    "benchmark": "clark_code_native_computer_use_smoke",
    "status": status,
    "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
    "host_platform": "macos",
    "target": target,
    "source_revision": source_revision,
    "source_dirty": source_dirty,
    "signing_identity": signing_identity,
    "permissions": {
        "accessibility": accessibility,
        "screen_recording": screen_recording,
    },
    "checks": checks,
    "safety_assertions": {
        "redacted_text_receipt": actions_passed,
        "stale_observation_rejected": actions_passed,
        "secure_field_mandatory_handoff": actions_passed,
        "cancellation_quiesced": actions_passed,
        "physical_takeover_quiesced": physical_takeover_passed,
    },
    "physical_takeover": {
        "required": physical_takeover_required,
        "status": (
            "passed"
            if physical_takeover_passed
            else "failed"
            if physical_takeover_required
            else "not_requested"
        ),
    },
    "action_receipt_id": action_match.group(1) if action_match else None,
    "cancellation_helper_terminated": (
        action_match.group(2) == "true" if action_match else None
    ),
    "credential_recorded": False,
    "artifacts": {
        "log": {
            "path": log_path.name,
            "sha256": sha256(log_path),
        },
        "host_executable": {
            "path": str(host_executable.relative_to(receipt_path.parent)),
            "sha256": sha256(host_executable),
        },
        "helper_executable": {
            "path": str(helper_executable.relative_to(receipt_path.parent)),
            "sha256": sha256(helper_executable),
        },
        "fixture_executable": {
            "path": str(fixture_executable.relative_to(receipt_path.parent)),
            "sha256": sha256(fixture_executable),
        },
    },
}
receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
PY
chmod 600 "$RECEIPT_PATH"

RECEIPT_STATUS="$(
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["status"])' \
    "$RECEIPT_PATH"
)"
printf \
  'native_fixture_smoke=%s host=%s fixture=%s receipt=%s\n' \
  "$RECEIPT_STATUS" \
  "$HOST_APP" \
  "$FIXTURE_APP" \
  "$RECEIPT_PATH"
[[ "$RECEIPT_STATUS" == "passed" ]]
