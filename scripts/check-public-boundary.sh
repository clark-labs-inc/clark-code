#!/usr/bin/env bash

set -euo pipefail

base_ref="${1:-}"
head_ref="${2:-HEAD}"
root_comparison=false

if [[ -z "$base_ref" ]]; then
  base_ref="$(git rev-parse --verify "${head_ref}^" 2>/dev/null || true)"
fi

if [[ -z "$base_ref" ]]; then
  root_comparison=true
elif ! git rev-parse --verify "$base_ref^{commit}" >/dev/null 2>&1; then
  echo "public-boundary: base revision is unavailable: $base_ref" >&2
  exit 2
fi

if ! git rev-parse --verify "$head_ref^{commit}" >/dev/null 2>&1; then
  echo "public-boundary: head revision is unavailable: $head_ref" >&2
  exit 2
fi

blocked_paths=()
while IFS= read -r path; do
  [[ -z "$path" ]] && continue
  case "$path" in
    .env.example|*/.env.example)
      ;;
    .env|*/.env|.env.*|*/.env.*|\
    *.pem|*.key|*.p12|*.pfx|*.jks|*.keystore|*.mobileprovision|\
    */id_rsa|*/id_ed25519|credentials.json|*/credentials.json|\
    service-account*.json|*/service-account*.json)
      blocked_paths+=("$path")
      ;;
  esac
done < <(
  if [[ "$root_comparison" == true ]]; then
    git diff-tree --root --no-commit-id --name-only --diff-filter=ACMRTUXB -r "$head_ref" --
  else
    git diff --name-only --diff-filter=ACMRTUXB "$base_ref" "$head_ref" --
  fi
)

if ((${#blocked_paths[@]} > 0)); then
  echo "public-boundary: blocked credential-shaped path(s):" >&2
  printf '  %s\n' "${blocked_paths[@]}" >&2
  exit 1
fi

# Inspect additions only. The check deliberately reports only the rule, never
# the matched line, so a failed check cannot echo a credential into CI logs.
if {
  if [[ "$root_comparison" == true ]]; then
    git diff-tree --root --unified=0 "$head_ref" -- . \
      ':(exclude)scripts/check-public-boundary.sh'
  else
    git diff --unified=0 "$base_ref" "$head_ref" -- . \
      ':(exclude)scripts/check-public-boundary.sh'
  fi
} \
  | sed -n '/^+/p' \
  | sed '/^+++/d' \
  | grep -E -q \
      '(-----BEGIN [A-Z ]*PRIVATE KEY-----|AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{20,}|gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[baprs]-[0-9A-Za-z-]{20,}|npm_[A-Za-z0-9]{20,}|sk-(live|proj|admin|ant)-[A-Za-z0-9_-]{16,}|https?://[^[:space:]]*:[^[:space:]@]+@)'; then
  echo "public-boundary: credential-shaped content detected in added lines" >&2
  exit 1
fi

echo "public-boundary: passed"
