#!/bin/sh
set -eu

BUILD_ROOT=${BUILD_ROOT:-/tmp/clark-cua-build}
OUTPUT_ROOT=${OUTPUT_ROOT:-/tmp/clark-cua-qa}
CARGO=${CARGO:-/root/.cargo/bin/cargo}

mkdir -p "$OUTPUT_ROOT"
cd "$BUILD_ROOT"
"$CARGO" test -p computer-use --features helper-service
"$CARGO" clippy \
  -p computer-use \
  --features helper-service \
  --lib \
  --bin clark-computer-use-helper \
  --example portable_native_smoke \
  --example portable_takeover_smoke \
  --example portable_xcap_diag \
  -- \
  -D warnings
"$CARGO" build \
  -p computer-use \
  --features helper-service \
  --release \
  --bin clark-computer-use-helper \
  --example portable_native_smoke \
  --example portable_takeover_smoke \
  --example portable_xcap_diag

install -m 0755 \
  "$BUILD_ROOT/target/release/clark-computer-use-helper" \
  "$OUTPUT_ROOT/clark-computer-use-helper"
install -m 0755 \
  "$BUILD_ROOT/target/release/examples/portable_native_smoke" \
  "$OUTPUT_ROOT/portable_native_smoke"
install -m 0755 \
  "$BUILD_ROOT/target/release/examples/portable_takeover_smoke" \
  "$OUTPUT_ROOT/portable_takeover_smoke"
install -m 0755 \
  "$BUILD_ROOT/target/release/examples/portable_xcap_diag" \
  "$OUTPUT_ROOT/portable_xcap_diag"

"$OUTPUT_ROOT/clark-computer-use-helper" --self-test
