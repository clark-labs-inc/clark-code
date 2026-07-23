#!/usr/bin/env bash

# Release-blocking Clark Code benchmark suite.
#
# The deterministic behavior matrix and skill journey always run and write
# detailed receipts. A paid Clark Platform turn is opt-in and deliberately has
# no default provider, model, endpoint, or credential.

set -uo pipefail

usage() {
  echo "Usage: $0 [--out NEW_DIRECTORY] [--superpowers CHECKOUT] [--live]"
  echo
  echo "Without --superpowers, the receipt-producing journey uses its built-in fixture."
  echo "--live additionally requires CLARK_CODE_PROVIDER, CLARK_CODE_BASE_URL,"
  echo "CLARK_CODE_MODEL, and CLARK_CODE_API_KEY."
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
output=""
superpowers=""
run_live=0

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
    --live)
      run_live=1
      shift
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

mkdir -p "$output"
output="$(cd "$output" && pwd)"
cd "$repo_root"
suite_started_seconds=$SECONDS

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

core_status="failed"
local_status="failed"
conversation_status="failed"
remote_status="failed"
frontend_status="failed"
synthetic_status="failed"
journey_status="blocked"
ui_status="failed"
live_status="skipped"
overall_status="failed"

run_family core_status \
  "core event/projection and provider-translation contracts" \
  "core-contracts.log" \
  cargo test -p agent-core -p provider-acp -p provider-clark --lib

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

deterministic_passed=1
for family_status in \
  "$core_status" \
  "$local_status" \
  "$conversation_status" \
  "$remote_status" \
  "$frontend_status" \
  "$synthetic_status" \
  "$journey_status" \
  "$ui_status"; do
  if [[ "$family_status" != "passed" ]]; then
    deterministic_passed=0
  fi
done

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
  elif [[ "$CLARK_CODE_MODEL" != "clark-code" && "$CLARK_CODE_MODEL" != clark-code:* ]]; then
    echo "Live benchmark requires an explicit Clark Code model alias." >&2
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
      echo "Running real read/search, permissioned mutation, and memory turns..."
      if CLARK_CODE_LIVE=1 cargo test -p provider-local --test live_clark_code \
        live_clark_code_feature_matrix -- \
        --ignored --exact --nocapture --test-threads=1 \
        2>&1 | tee "$output/live-feature-matrix.log"; then
        live_status="passed"
      fi
    fi
  fi
fi

if [[ "$deterministic_passed" == "1" ]]; then
  if [[ "$run_live" == "0" || "$live_status" == "passed" ]]; then
    overall_status="passed"
  fi
fi
suite_duration_seconds=$((SECONDS - suite_started_seconds))

python3 - \
  "$output/pre-release-receipt.json" \
  "$output/report.md" \
  "$overall_status" \
  "$suite_duration_seconds" \
  "$core_status" \
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
  "${CLARK_CODE_BASE_URL:-}" <<'PY'
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
    core,
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

receipt = {
    "schemaVersion": 1,
    "benchmark": "clark_pre_release_v1",
    "status": overall,
    "generatedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "durationSeconds": int(duration_seconds),
    "families": {
        "coreContracts": {
            "status": core,
            "tests": rust_test_count("core-contracts.log"),
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
        ],
        "provider": provider or None,
        "model": model or None,
        "baseUrl": base_url or None,
        "apiKeyRecorded": False,
    },
}
pathlib.Path(receipt_path).write_text(json.dumps(receipt, indent=2) + "\n")
pathlib.Path(report_path).write_text(
    "# Clark pre-release benchmark\n\n"
    f"**Result:** {overall}  \n"
    f"**Duration:** {duration_seconds} seconds  \n"
    f"**Core contracts:** {core}  \n"
    f"**Local capabilities:** {local}  \n"
    f"**Conversation integrations:** {conversations}  \n"
    f"**Remote workspace integrations:** {remote}  \n"
    f"**Frontend contracts:** {frontend}  \n"
    f"**Synthetic regression:** {synthetic}  \n"
    f"**Read/Superpowers journey:** {journey}  \n"
    f"**UI resilience sample:** {ui}  \n"
    f"**Journey source:** `{source}`  \n"
    f"**Live Clark Code turn:** {live}"
    + (f" (`{provider}` / `{model}`)" if provider or model else "")
    + "\n\n"
    "The live credential is never written to benchmark artifacts.\n"
)
PY

echo "Pre-release report: $output/report.md"
echo "Pre-release receipt: $output/pre-release-receipt.json"
if [[ "$overall_status" != "passed" ]]; then
  exit 1
fi
