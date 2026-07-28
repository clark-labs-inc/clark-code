#!/bin/sh
set -eu

BUILD_ROOT=/tmp/clark-cua-build
OUTPUT_ROOT=/tmp/clark-cua-qa

rm -rf "$BUILD_ROOT"
mkdir -p "$BUILD_ROOT/computer-use" "$OUTPUT_ROOT"
tar -xzf /tmp/clark-cua-portable-src.tgz -C "$BUILD_ROOT"
cp "$BUILD_ROOT/harness/fixtures/computer-use-portable/Cargo.toml" "$BUILD_ROOT/Cargo.toml"
cp -R "$BUILD_ROOT/crates/computer-use/." "$BUILD_ROOT/computer-use/"

cd "$BUILD_ROOT"
/root/.cargo/bin/cargo build \
  --release \
  -p computer-use \
  --features helper-service \
  --bin clark-computer-use-helper
/root/.cargo/bin/cargo build \
  --release \
  -p computer-use \
  --example portable_native_smoke

install -m 0755 \
  "$BUILD_ROOT/target/release/clark-computer-use-helper" \
  "$OUTPUT_ROOT/clark-computer-use-helper"
install -m 0755 \
  "$BUILD_ROOT/target/release/examples/portable_native_smoke" \
  "$OUTPUT_ROOT/portable_native_smoke"
"$OUTPUT_ROOT/clark-computer-use-helper" --self-test
