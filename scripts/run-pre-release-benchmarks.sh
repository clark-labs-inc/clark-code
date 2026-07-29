#!/usr/bin/env bash

# Release-blocking Clark Code benchmark suite.
#
# The deterministic behavior matrix and skill journey always run and write
# detailed receipts. Paid Clark Platform validation through the cheapest
# supported tool-calling model is required by default; --offline is the explicit
# no-network/no-credit opt-out.
# Provider, endpoint, and model have checked-in defaults. The credential remains
# local, is loaded without executing .env, and is never written to receipts.

set -uo pipefail
umask 077

usage() {
  echo "Usage: $0 [--out NEW_DIRECTORY] [--superpowers CHECKOUT] [--offline]"
  echo "          [--utm-preflight] [--utm-observation-receipt PATH]"
  echo "          [--real-use-receipt PATH]..."
  echo "          [--native-computer-use-smoke]"
  echo "          [--computer-use-signing-identity IDENTITY]"
  echo
  echo "Without --superpowers, the receipt-producing journey uses its built-in fixture."
  echo "The default paid lane uses clark-platform / qwen/qwen3.7-flash and"
  echo "requires CLARK_CODE_API_KEY (environment or the ignored repository .env)."
  echo "--offline explicitly disables all paid/network model calls."
  echo "--utm-preflight autonomously provisions, logs in, reboots, and verifies"
  echo "Windows and Ubuntu under UTM; no user VM action can satisfy the gate."
  echo "--real-use-receipt independently verifies one guest package; a supplied set"
  echo "must contain exactly macos, windows, and ubuntu to pass."
  echo "--native-computer-use-smoke runs the signed macOS helper/fixture boundary."
  echo "Physical-input testing is a standalone non-release diagnostic."
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
output=""
superpowers=""
run_live=1
run_utm_preflight=0
utm_observation_receipt=""
real_use_receipts=()
real_use_receipt_count=0
run_native_computer_use=0
require_native_physical_takeover=0
computer_use_signing_identity=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out)
      if [[ $# -lt 2 ]]; then
        echo "--out requires a path" >&2
        exit 2
      fi
      output="$2"
      shift 2
      ;;
    --superpowers)
      if [[ $# -lt 2 ]]; then
        echo "--superpowers requires a path" >&2
        exit 2
      fi
      superpowers="$2"
      shift 2
      ;;
    --offline)
      run_live=0
      shift
      ;;
    --utm-preflight)
      run_utm_preflight=1
      shift
      ;;
    --utm-observation-receipt)
      if [[ $# -lt 2 ]]; then
        echo "--utm-observation-receipt requires a path" >&2
        exit 2
      fi
      utm_observation_receipt="$2"
      run_utm_preflight=1
      shift 2
      ;;
    --real-use-receipt)
      if [[ $# -lt 2 ]]; then
        echo "--real-use-receipt requires a path" >&2
        exit 2
      fi
      real_use_receipts+=("$2")
      real_use_receipt_count=$((real_use_receipt_count + 1))
      run_utm_preflight=1
      shift 2
      ;;
    --native-computer-use-smoke)
      run_native_computer_use=1
      shift
      ;;
    --computer-use-signing-identity)
      if [[ $# -lt 2 ]]; then
        echo "--computer-use-signing-identity requires a value" >&2
        exit 2
      fi
      computer_use_signing_identity="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$output" ]]; then
  output="$repo_root/target/pre-release-benchmarks/$(date -u +%Y%m%dT%H%M%SZ)-$$"
elif [[ "$output" != /* ]]; then
  output="$repo_root/$output"
fi
if [[ -e "$output" ]]; then
  echo "Refusing to overwrite benchmark output: $output" >&2
  exit 2
fi
if [[ -n "$superpowers" && ! -d "$superpowers" ]]; then
  echo "Superpowers checkout does not exist: $superpowers" >&2
  exit 2
fi
if [[ -n "$superpowers" ]]; then
  superpowers="$(cd "$superpowers" && pwd)"
fi
if [[ -n "$utm_observation_receipt" ]]; then
  if [[ "$utm_observation_receipt" != /* ]]; then
    utm_observation_receipt="$repo_root/$utm_observation_receipt"
  fi
  if [[ ! -f "$utm_observation_receipt" ]]; then
    echo "UTM observation receipt does not exist: $utm_observation_receipt" >&2
    exit 2
  fi
fi
resolved_real_use_receipts=()
expected_source_revision="$(git -C "$repo_root" rev-parse HEAD)"
if [[ "$real_use_receipt_count" -gt 0 ]]; then
  for real_use_receipt in "${real_use_receipts[@]}"; do
    if [[ "$real_use_receipt" != /* ]]; then
      real_use_receipt="$repo_root/$real_use_receipt"
    fi
    if [[ ! -f "$real_use_receipt" ]]; then
      echo "Real-use guest receipt does not exist: $real_use_receipt" >&2
      exit 2
    fi
    resolved_real_use_receipts+=("$real_use_receipt")
  done
  real_use_receipts=("${resolved_real_use_receipts[@]}")
fi

mkdir -p "$output"
output="$(cd "$output" && pwd)"
cd "$repo_root"
suite_started_seconds=$SECONDS

load_benchmark_env() {
  local env_path="$repo_root/.env"
  local name
  local value
  [[ -f "$env_path" ]] || return 0
  while IFS='=' read -r name value; do
    name="${name#"${name%%[![:space:]]*}"}"
    name="${name%"${name##*[![:space:]]}"}"
    case "$name" in
      CLARK_CODE_API_KEY|CLARK_CODE_PROVIDER|CLARK_CODE_BASE_URL|CLARK_CODE_MODEL|CLARK_COMPUTER_USE_SIGNING_IDENTITY)
        if [[ -z "${!name:-}" ]]; then
          value="${value%$'\r'}"
          value="${value#"${value%%[![:space:]]*}"}"
          value="${value%"${value##*[![:space:]]}"}"
          if [[ "$value" == \"*\" && "$value" == *\" ]]; then
            value="${value:1:${#value}-2}"
          elif [[ "$value" == \'*\' && "$value" == *\' ]]; then
            value="${value:1:${#value}-2}"
          fi
          printf -v "$name" '%s' "$value"
          export "$name"
        fi
        ;;
    esac
  done < "$env_path"
}

load_benchmark_env
computer_use_signing_identity="${computer_use_signing_identity:-${CLARK_COMPUTER_USE_SIGNING_IDENTITY:-}}"
if [[ "$run_native_computer_use" == "1" ]]; then
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "The signed native computer-use smoke is macOS-only." >&2
    exit 2
  fi
  if [[ -z "$computer_use_signing_identity" ]]; then
    echo "The signed native computer-use smoke requires --computer-use-signing-identity or CLARK_COMPUTER_USE_SIGNING_IDENTITY." >&2
    exit 2
  fi
fi
export CLARK_CODE_PROVIDER="${CLARK_CODE_PROVIDER:-clark-platform}"
export CLARK_CODE_BASE_URL="${CLARK_CODE_BASE_URL:-https://api.clarkslabs.com/v1}"
export CLARK_CODE_MODEL="${CLARK_CODE_MODEL:-qwen/qwen3.7-flash}"
export CLARK_CODE_MAX_ITERATIONS="${CLARK_CODE_MAX_ITERATIONS:-16}"
export CLARK_CODE_MAX_LIVE_COST_USD="${CLARK_CODE_MAX_LIVE_COST_USD:-0.50}"
if [[ ! "$CLARK_CODE_MAX_ITERATIONS" =~ ^[1-9][0-9]*$ ]]; then
  echo "CLARK_CODE_MAX_ITERATIONS must be a positive integer." >&2
  exit 2
fi
if [[ ! "$CLARK_CODE_MAX_LIVE_COST_USD" =~ ^[0-9]+([.][0-9]+)?$ ]] \
  || [[ "$CLARK_CODE_MAX_LIVE_COST_USD" == "0" ]] \
  || [[ "$CLARK_CODE_MAX_LIVE_COST_USD" == "0.0" ]]; then
  echo "CLARK_CODE_MAX_LIVE_COST_USD must be a positive number." >&2
  exit 2
fi

live_cost_below_cap() {
  python3 - "$output" "$CLARK_CODE_MAX_LIVE_COST_USD" <<'PY'
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
cap = float(sys.argv[2])
reported = 0.0
for log in root.glob("live-*.log"):
    text = log.read_text(errors="replace")
    reported += sum(
        float(value)
        for value in re.findall(r"cost_usd:\s*Some\(([0-9]+(?:\.[0-9]+)?)\)", text)
    )
raise SystemExit(0 if reported < cap else 1)
PY
}

run_family() {
  local result_variable="$1"
  local title="$2"
  local log_name="$3"
  shift 3
  echo
  echo "Running $title..."
  if "$@" 2>&1 | tee "$output/$log_name"; then
    printf -v "$result_variable" '%s' "passed"
  else
    printf -v "$result_variable" '%s' "failed"
  fi
}

inventory_status="failed"
matrix_status="failed"
native_computer_use_status="not_requested"
utm_autonomy_status="not_requested"
utm_status="not_requested"
real_use_status="not_requested"
core_status="failed"
native_status="failed"
local_status="failed"
conversation_status="failed"
remote_status="failed"
frontend_status="failed"
synthetic_status="failed"
journey_status="blocked"
ui_status="failed"
live_status="skipped"
overall_status="failed"

run_family inventory_status \
  "authoritative feature, native-command, permission, security, and platform inventory" \
  "capability-inventory.log" \
  node harness/feature-matrix.mjs --validate-only

run_family matrix_status \
  "every authoritative deterministic feature lane" \
  "feature-matrix.log" \
  node harness/feature-matrix.mjs \
    --offline \
    --out "$output/feature-matrix"

if [[ "$run_native_computer_use" == "1" ]]; then
  native_computer_use_args=(
    --output "$output/native-computer-use"
    --signing-identity "$computer_use_signing_identity"
  )
  if [[ "$require_native_physical_takeover" == "1" ]]; then
    native_computer_use_args+=(--physical-takeover)
  fi
  run_family native_computer_use_status \
    "the signed native computer-use helper and fixture boundary" \
    "native-computer-use.log" \
    ./scripts/run-computer-use-native-fixture-smoke.sh \
      "${native_computer_use_args[@]}"
fi

if [[ "$run_utm_preflight" == "1" ]]; then
  echo
  echo "Ensuring the Windows and Ubuntu UTM guests are fully autonomous..."
  if node harness/utm-autonomy.mjs ensure \
    --platform all \
    --out "$output/utm-autonomy" \
    2>&1 | tee "$output/utm-autonomy.log"; then
    utm_autonomy_status="passed"
  else
    utm_autonomy_status="failed"
  fi

  echo
  echo "Running the release-blocking Windows and Ubuntu UTM environment preflight..."
  utm_args=(--platform all --out "$output/utm-preflight")
  if [[ -n "$utm_observation_receipt" ]]; then
    utm_args+=(--observation-receipt "$utm_observation_receipt")
  fi
  if node harness/utm-real-use.mjs "${utm_args[@]}" \
    2>&1 | tee "$output/utm-preflight.log"; then
    if [[ "$utm_autonomy_status" == "passed" ]]; then
      utm_status="ready"
    else
      utm_status="blocked"
    fi
  else
    utm_status="blocked"
  fi
fi

if [[ "$real_use_receipt_count" -gt 0 ]]; then
  echo
  echo "Independently verifying supplied macOS, Windows, and Ubuntu real-use packages..."
  real_use_status="passed"
  real_use_platforms_seen=" "
  verified_real_use_platform_count=0
  for real_use_receipt in "${real_use_receipts[@]}"; do
    real_use_platform="$(python3 - "$real_use_receipt" <<'PY'
import json
import pathlib
import sys

value = json.loads(pathlib.Path(sys.argv[1]).read_text()).get("platform")
if value not in {"macos", "windows", "ubuntu"}:
    raise SystemExit(f"invalid real-use receipt platform: {value!r}")
print(value)
PY
)" || {
      real_use_status="failed"
      continue
    }
    if [[ "$real_use_platforms_seen" == *" $real_use_platform "* ]]; then
      echo "Duplicate real-use guest receipt for $real_use_platform." >&2
      real_use_status="failed"
      continue
    fi
    real_use_platforms_seen+="$real_use_platform "
    verified_real_use_platform_count=$((verified_real_use_platform_count + 1))
    if CLARK_EXPECTED_SOURCE_REVISION="$expected_source_revision" \
      node harness/platform-real-use-package.mjs \
      --verify-receipt "$real_use_receipt" \
      --copy-to "$output/real-use/$real_use_platform" \
      2>&1 | tee "$output/real-use-$real_use_platform.log"; then
      copied_real_use_status="$(python3 - "$output/real-use/$real_use_platform/receipt.json" <<'PY'
import json
import pathlib
import sys

print(json.loads(pathlib.Path(sys.argv[1]).read_text()).get("status", "invalid"))
PY
)"
      if [[ "$copied_real_use_status" == "blocked" && "$real_use_status" == "passed" ]]; then
        real_use_status="blocked"
      elif [[ "$copied_real_use_status" != "passed" && "$copied_real_use_status" != "blocked" ]]; then
        real_use_status="failed"
      fi
    else
      real_use_status="failed"
    fi
  done
  if [[ "$verified_real_use_platform_count" -ne 3 ]]; then
    echo "A complete real-use set requires exactly macos, windows, and ubuntu receipts." >&2
    if [[ "$real_use_status" == "passed" ]]; then
      real_use_status="incomplete"
    fi
  fi
fi

run_family core_status \
  "core event/projection and provider-translation contracts" \
  "core-contracts.log" \
  cargo test -p agent-core -p provider-acp -p provider-clark --lib

run_family native_status \
  "native desktop command, account-boundary, and local persistence contracts" \
  "native-host-contracts.log" \
  cargo test -p clark-desktop --lib

run_family local_status \
  "local agent capability contracts (tools, permissions, memory, planning, recovery)" \
  "local-capabilities.log" \
  cargo test -p provider-local --lib

run_family conversation_status \
  "scripted conversation and failed-continuation integrations" \
  "conversation-integrations.log" \
  cargo test -p provider-local \
    --test local_loop \
    --test failed_continuation

run_family remote_status \
  "remote execution, git, and worktree integrations" \
  "remote-workspace-integrations.log" \
  cargo test -p provider-local \
    --test remote_parity \
    --test remote_git \
    --test worktree_simulation

run_family frontend_status \
  "frontend state, projection, composer, and surface contracts" \
  "frontend-contracts.log" \
  pnpm --dir app test

run_family synthetic_status \
  "the self-contained 16-stage empty-home skill regression" \
  "synthetic-regression.log" \
  cargo test -p provider-local --example skill_experience_benchmark

if [[ "$synthetic_status" == "passed" ]]; then
  journey_args=(--out "$output/skill-experience")
  source_label="synthetic"
  if [[ -n "$superpowers" ]]; then
    journey_args+=(--superpowers "$superpowers")
    source_label="$superpowers"
  else
    journey_args+=(--synthetic)
  fi

  echo "Running the receipt-producing Read/Superpowers journey from: $source_label"
  if cargo run -p provider-local --example skill_experience_benchmark -- \
    "${journey_args[@]}" 2>&1 | tee "$output/skill-experience.log"; then
    journey_status="passed"
  else
    journey_status="failed"
  fi
else
  source_label="${superpowers:-synthetic}"
  echo "Skipping the receipt-producing journey because its self-test failed." >&2
fi

run_family ui_status \
  "the eight-case UI resilience sample (baseline, every fault, combined faults)" \
  "ui-resilience.log" \
  node harness/resilience-benchmark.mjs \
    --smoke \
    "--out=$output/ui-resilience"

contract_families_passed=1
for family_status in \
  "$inventory_status" \
  "$matrix_status" \
  "$core_status" \
  "$native_status" \
  "$local_status" \
  "$conversation_status" \
  "$remote_status" \
  "$frontend_status" \
  "$synthetic_status" \
  "$journey_status" \
  "$ui_status"; do
  if [[ "$family_status" != "passed" ]]; then
    contract_families_passed=0
  fi
done
if [[ "$run_native_computer_use" == "1" && "$native_computer_use_status" != "passed" ]]; then
  contract_families_passed=0
fi
environment_blocked=0
if [[ "$run_utm_preflight" == "1" && "$utm_status" != "ready" ]]; then
  if [[ "$utm_status" == "blocked" ]]; then
    environment_blocked=1
  else
    contract_families_passed=0
  fi
fi
if [[ "$real_use_receipt_count" -gt 0 && "$real_use_status" != "passed" ]]; then
  if [[ "$real_use_status" == "blocked" || "$real_use_status" == "incomplete" ]]; then
    environment_blocked=1
  else
    contract_families_passed=0
  fi
fi
deterministic_passed=0
if [[ "$contract_families_passed" == "1" && "$environment_blocked" == "0" ]]; then
  deterministic_passed=1
fi

if [[ "$run_live" == "1" ]]; then
  live_status="configuration_failed"
  missing=()
  for name in CLARK_CODE_PROVIDER CLARK_CODE_BASE_URL CLARK_CODE_MODEL CLARK_CODE_API_KEY; do
    if [[ -z "${!name:-}" ]]; then
      missing+=("$name")
    fi
  done
  if [[ "${#missing[@]}" -gt 0 ]]; then
    echo "Live benchmark requested, but required configuration is missing: ${missing[*]}" >&2
  elif [[ "$CLARK_CODE_PROVIDER" != "clark-platform" ]]; then
    echo "Live benchmark requires CLARK_CODE_PROVIDER=clark-platform." >&2
  elif [[ "$CLARK_CODE_BASE_URL" != "https://api.clarkslabs.com/v1" ]]; then
    echo "Live benchmark requires CLARK_CODE_BASE_URL=https://api.clarkslabs.com/v1." >&2
  elif [[ "$CLARK_CODE_MODEL" != "qwen/qwen3.7-flash" ]]; then
    echo "Live benchmark requires CLARK_CODE_MODEL=qwen/qwen3.7-flash." >&2
  elif [[ "$deterministic_passed" != "1" ]]; then
    live_status="blocked"
    echo "Skipping paid validation because the deterministic contract is already broken." >&2
  else
    live_status="failed"
    echo "Running the real managed-skill turn through $CLARK_CODE_PROVIDER / $CLARK_CODE_MODEL..."
    if CLARK_CODE_LIVE=1 cargo test -p provider-local --test live_clark_code \
      live_clark_code_skills_end_to_end -- \
      --ignored --exact --nocapture --test-threads=1 \
      2>&1 | tee "$output/live-skill-turn.log"; then
      if live_cost_below_cap; then
        echo "Running real read/search, permissioned mutation, and memory turns..."
        if CLARK_CODE_LIVE=1 cargo test -p provider-local --test live_clark_code \
          live_clark_code_feature_matrix -- \
          --ignored --exact --nocapture --test-threads=1 \
          2>&1 | tee "$output/live-feature-matrix.log"; then
          if live_cost_below_cap; then
            echo "Running real compaction and continuation through the managed Qwen 3.7 Flash route..."
            if CLARK_CODE_LIVE=1 cargo test -p provider-local --test live_clark_code \
              live_clark_code_compacts_and_continues -- \
              --ignored --exact --nocapture --test-threads=1 \
              2>&1 | tee "$output/live-compaction.log"; then
              live_status="passed"
            fi
          else
            live_status="budget_exhausted"
            echo "Paid benchmark reached its reported $CLARK_CODE_MAX_LIVE_COST_USD USD inter-test ceiling before compaction." >&2
          fi
        fi
      else
        live_status="budget_exhausted"
        echo "Paid benchmark reached its reported $CLARK_CODE_MAX_LIVE_COST_USD USD inter-test ceiling after the skill job." >&2
      fi
    fi
  fi
fi

if [[ "$contract_families_passed" == "1" ]]; then
  if [[ "$environment_blocked" == "1" ]]; then
    overall_status="blocked"
  elif [[ "$run_live" == "0" || "$live_status" == "passed" ]]; then
    overall_status="passed"
  elif [[ "$live_status" == "blocked" \
    || "$live_status" == "configuration_failed" \
    || "$live_status" == "budget_exhausted" ]]; then
    overall_status="blocked"
  fi
fi
suite_duration_seconds=$((SECONDS - suite_started_seconds))

python3 - \
  "$output/pre-release-receipt.json" \
  "$output/report.md" \
  "$overall_status" \
  "$suite_duration_seconds" \
  "$inventory_status" \
  "$matrix_status" \
  "$native_computer_use_status" \
  "$run_native_computer_use" \
  "$require_native_physical_takeover" \
  "$utm_autonomy_status" \
  "$utm_status" \
  "$run_utm_preflight" \
  "$real_use_status" \
  "$core_status" \
  "$native_status" \
  "$local_status" \
  "$conversation_status" \
  "$remote_status" \
  "$frontend_status" \
  "$synthetic_status" \
  "$journey_status" \
  "$ui_status" \
  "$live_status" \
  "$run_live" \
  "$source_label" \
  "${CLARK_CODE_PROVIDER:-}" \
  "${CLARK_CODE_MODEL:-}" \
  "${CLARK_CODE_BASE_URL:-}" \
  "$CLARK_CODE_MAX_ITERATIONS" \
  "$CLARK_CODE_MAX_LIVE_COST_USD" <<'PY'
import datetime
import json
import pathlib
import re
import sys

(
    receipt_path,
    report_path,
    overall,
    duration_seconds,
    inventory,
    matrix,
    native_computer_use,
    native_computer_use_required,
    native_physical_takeover_required,
    utm_autonomy,
    utm,
    utm_required,
    real_use,
    core,
    native,
    local,
    conversations,
    remote,
    frontend,
    synthetic,
    journey,
    ui,
    live,
    live_required,
    source,
    provider,
    model,
    base_url,
    max_iterations,
    max_live_cost_usd,
) = sys.argv[1:]

suite_dir = pathlib.Path(receipt_path).parent

def rust_test_count(name):
    path = suite_dir / name
    if not path.is_file():
        return 0
    return sum(
        int(match)
        for match in re.findall(r"test result: .*? (\d+) passed;", path.read_text())
    )

def frontend_test_count():
    path = suite_dir / "frontend-contracts.log"
    if not path.is_file():
        return 0
    match = re.search(r"Tests\s+(\d+) passed", path.read_text())
    return int(match.group(1)) if match else 0

def json_file(relative):
    path = suite_dir / relative
    return json.loads(path.read_text()) if path.is_file() else {}

skill_receipt = json_file("skill-experience/receipt.json")
ui_receipt = json_file("ui-resilience/report.json")
native_computer_use_receipt = json_file("native-computer-use/receipt.json")
utm_autonomy_receipt = json_file("utm-autonomy/receipt.json")

def first_json_line(name):
    path = suite_dir / name
    if not path.is_file():
        return {}
    for line in path.read_text(errors="replace").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    return {}

def reported_live_cost():
    total = 0.0
    for path in suite_dir.glob("live-*.log"):
        total += sum(
            float(value)
            for value in re.findall(
                r"cost_usd:\s*Some\(([0-9]+(?:\.[0-9]+)?)\)",
                path.read_text(errors="replace"),
            )
        )
    return total

capability_inventory = first_json_line("capability-inventory.log")
feature_matrix = json_file("feature-matrix/report.json")
utm_receipt = json_file("utm-preflight/receipt.json")
real_use_receipts = {}
for platform in ("macos", "windows", "ubuntu"):
    value = json_file(f"real-use/{platform}/receipt.json")
    if value:
        real_use_receipts[platform] = value

receipt = {
    "schemaVersion": 2,
    "benchmark": "clark_code_consolidated_v2",
    "status": overall,
    "generatedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "durationSeconds": int(duration_seconds),
    "families": {
        "authoritativeCapabilityInventory": {
            "status": inventory,
            "features": capability_inventory.get("features", 0),
            "modelTools": capability_inventory.get("model_tools", 0),
            "nativeCommands": capability_inventory.get("native_commands", 0),
            "securityControls": capability_inventory.get("security_controls", 0),
            "inventories": capability_inventory.get("inventories", {}),
        },
        "authoritativeFeatureMatrix": {
            "status": matrix,
            "platform": feature_matrix.get("platform"),
            "mode": feature_matrix.get("execution", {}).get("mode"),
            "summary": feature_matrix.get("summary", {}),
            "lanes": {
                result.get("id"): result.get("status")
                for result in feature_matrix.get("results", [])
                if result.get("id")
            },
            "report": "feature-matrix/report.json",
        },
        "nativeComputerUseSmoke": {
            "required": native_computer_use_required == "1",
            "status": native_computer_use,
            "physicalTakeoverRequired": native_physical_takeover_required == "1",
            "physicalTakeoverStatus": native_computer_use_receipt.get(
                "physical_takeover", {}
            ).get("status", "not_requested"),
            "checks": native_computer_use_receipt.get("checks", {}),
            "safetyAssertions": native_computer_use_receipt.get("safety_assertions", {}),
            "report": (
                "native-computer-use/receipt.json"
                if native_computer_use_receipt
                else None
            ),
        },
        "utmEnvironmentPreflight": {
            "required": utm_required == "1",
            "status": utm,
            "virtualization": "utm",
            "autonomy": {
                "status": utm_autonomy,
                "requiredUserVmActions": utm_autonomy_receipt.get(
                    "required_user_vm_actions"
                ),
                "forbiddenVirtualizationInvocations": utm_autonomy_receipt.get(
                    "forbidden_virtualization_invocations"
                ),
                "guests": {
                    platform: guest.get("status")
                    for platform, guest in utm_autonomy_receipt.get("guests", {}).items()
                },
                "report": (
                    "utm-autonomy/receipt.json"
                    if utm_autonomy_receipt
                    else None
                ),
            },
            "guests": {
                guest.get("platform"): {
                    "status": guest.get("status"),
                    "environmentReady": guest.get("environment_ready", False),
                    "productInstalled": guest.get("product_installed", False),
                }
                for guest in utm_receipt.get("guests", [])
                if guest.get("platform")
            },
            "realScenariosExecuted": utm_receipt.get("real_scenarios_executed", 0),
            "report": "utm-preflight/receipt.json" if utm_receipt else None,
        },
        "platformRealUse": {
            "required": real_use != "not_requested",
            "status": real_use,
            "completePlatformSet": set(real_use_receipts) == {"macos", "windows", "ubuntu"},
            "platforms": {
                platform: {
                    "status": value.get("status"),
                    "sourceRevision": value.get("source_revision"),
                    "supportedFeatures": value.get("coverage", {}).get("supported_features"),
                    "scenariosPassed": value.get("coverage", {}).get("scenarios_passed"),
                    "reportedCostUsd": value.get("matrix", {}).get("reported_cost_usd"),
                    "requiredUserVmActions": value.get("required_user_vm_actions"),
                    "manualVmActionsAllowed": value.get("manual_vm_actions_allowed"),
                    "humanInputObserved": value.get("human_input_observed"),
                    "receipt": f"real-use/{platform}/receipt.json",
                }
                for platform, value in real_use_receipts.items()
            },
        },
        "coreContracts": {
            "status": core,
            "tests": rust_test_count("core-contracts.log"),
        },
        "nativeHostContracts": {
            "status": native,
            "tests": rust_test_count("native-host-contracts.log"),
        },
        "localCapabilities": {
            "status": local,
            "tests": rust_test_count("local-capabilities.log"),
        },
        "conversationIntegrations": {
            "status": conversations,
            "tests": rust_test_count("conversation-integrations.log"),
        },
        "remoteWorkspaceIntegrations": {
            "status": remote,
            "tests": rust_test_count("remote-workspace-integrations.log"),
        },
        "frontendContracts": {
            "status": frontend,
            "tests": frontend_test_count(),
        },
        "syntheticRegression": {
            "status": synthetic,
            "tests": rust_test_count("synthetic-regression.log"),
        },
        "readSkillExperience": {
            "status": journey,
            "stages": len(skill_receipt.get("steps", [])),
        },
        "uiResilienceSample": {
            "status": ui,
            "cases": len(ui_receipt.get("simulated", [])),
            "selection": ui_receipt.get("selection"),
        },
    },
    "skillExperienceSource": source,
    "live": {
        "required": live_required == "1",
        "status": live,
        "scenarios": [
            "managed_skill_resource_and_write",
            "basic_response",
            "read_search",
            "permissioned_mutation",
            "memory_round_trip",
            "compaction_and_continuation",
        ],
        "provider": provider or None,
        "model": model or None,
        "baseUrl": base_url or None,
        "maxTests": 3,
        "maxIterationsPerTurn": int(max_iterations),
        "interTestCostCeilingUsd": float(max_live_cost_usd),
        "reportedCostUsd": reported_live_cost(),
        "apiKeyRecorded": False,
    },
}
utm_requirement = "required" if utm_required == "1" else "not requested"
live_identity = f" (`{provider}` / `{model}`)" if provider or model else ""
pathlib.Path(receipt_path).write_text(json.dumps(receipt, indent=2) + "\n")
pathlib.Path(report_path).write_text(
    "# Clark pre-release benchmark\n\n"
    f"**Result:** {overall}  \n"
    f"**Duration:** {duration_seconds} seconds  \n"
    f"**Authoritative capability inventory:** {inventory}  \n"
    f"**Every authoritative deterministic feature lane:** {matrix}  \n"
    f"**Signed native computer-use smoke:** {native_computer_use}  \n"
    f"**Autonomous UTM lifecycle:** {utm_autonomy}  \n"
    f"**Windows and Ubuntu UTM environment preflight:** {utm} ({utm_requirement})  \n"
    f"**macOS, Windows, and Ubuntu real-use receipts:** {real_use}  \n"
    f"**Core contracts:** {core}  \n"
    f"**Native host contracts:** {native}  \n"
    f"**Local capabilities:** {local}  \n"
    f"**Conversation integrations:** {conversations}  \n"
    f"**Remote workspace integrations:** {remote}  \n"
    f"**Frontend contracts:** {frontend}  \n"
    f"**Synthetic regression:** {synthetic}  \n"
    f"**Read/Superpowers journey:** {journey}  \n"
    f"**UI resilience sample:** {ui}  \n"
    f"**Journey source:** `{source}`  \n"
    f"**Managed Qwen 3.7 Flash chats/jobs:** {live}{live_identity}\n\n"
    "The live credential is never written to benchmark artifacts.\n"
)
PY

echo "Pre-release report: $output/report.md"
echo "Pre-release receipt: $output/pre-release-receipt.json"
if [[ "$overall_status" != "passed" ]]; then
  exit 1
fi
