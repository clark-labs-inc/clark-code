#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# The open launcher always enables compile-time diagnostics for a local build.
# Branded distributions own their signing, sidecars, and release configuration.
exec cargo tauri dev --features debug-diagnostics
