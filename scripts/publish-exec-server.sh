#!/usr/bin/env bash
# Publish the per-arch clark-exec-server prebuilts to the downloads CDN, where
# the SSH orchestrator (src-tauri/src/ssh.rs `fetch_from_cdn`) pulls them from:
#
#   https://downloads.clarkchat.com/exec-server/<version>/clark-exec-server-<slug>
#   …and a matching .sha256 sidecar the remote verifies after download.
#
# Build the four targets first (macOS host, cargo-zigbuild for the Linux ones):
#   rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl \
#                     aarch64-apple-darwin x86_64-apple-darwin
#   cargo zigbuild --release -p exec-server --bin clark-exec-server \
#       --target x86_64-unknown-linux-musl --target aarch64-unknown-linux-musl
#   cargo build    --release -p exec-server --bin clark-exec-server \
#       --target aarch64-apple-darwin --target x86_64-apple-darwin
#
# Then: scripts/publish-exec-server.sh v0.1.4
#
# The release workflow runs this in CI (the account's GitHub OIDC provider was
# created 2026-07-01, so CI's AWS auth works). Run it by hand with clark_root
# creds only for a local/out-of-band publish.
set -euo pipefail

VERSION="${1:?usage: publish-exec-server.sh <vX.Y.Z>}"
BUCKET="${CLARK_DESKTOP_DOWNLOAD_BUCKET:-clark-desktop-downloads-prod}"
DIST="${CLARK_DESKTOP_DOWNLOAD_DISTRIBUTION_ID:-E3RS34ZLDTMN7Z}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="exec-server/$VERSION"
IMMUTABLE="public, max-age=31536000, immutable"

# slug:cargo-target — slug must match RemoteArch::slug() in ssh.rs.
TARGETS="
linux-x86_64:x86_64-unknown-linux-musl
linux-aarch64:aarch64-unknown-linux-musl
darwin-aarch64:aarch64-apple-darwin
darwin-x86_64:x86_64-apple-darwin
"

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}';
  else shasum -a 256 "$1" | awk '{print $1}'; fi
}

manifest="{\"version\":\"$VERSION\",\"binaries\":{"
first=1
for pair in $TARGETS; do
  slug="${pair%%:*}"; target="${pair#*:}"
  bin="$ROOT/target/$target/release/clark-exec-server"
  [ -f "$bin" ] || { echo "missing build: $bin (build the $target target first)" >&2; exit 1; }
  name="clark-exec-server-$slug"
  sha="$(sha256_of "$bin")"
  size="$(wc -c < "$bin" | tr -d '[:space:]')"

  tmp="$(mktemp)"; printf '%s  %s\n' "$sha" "$name" > "$tmp"
  echo "→ $slug  ($size bytes, sha ${sha:0:12}…)"
  aws s3 cp "$bin" "s3://$BUCKET/$PREFIX/$name" \
    --content-type application/octet-stream --cache-control "$IMMUTABLE" \
    --metadata "sha256=$sha,clark-version=$VERSION" >/dev/null
  aws s3 cp "$tmp" "s3://$BUCKET/$PREFIX/$name.sha256" \
    --content-type text/plain --cache-control "$IMMUTABLE" >/dev/null
  rm -f "$tmp"

  [ $first -eq 1 ] || manifest="$manifest,"
  manifest="$manifest\"$slug\":{\"sha256\":\"$sha\",\"size\":$size}"
  first=0
done
manifest="$manifest}}"

echo "$manifest" | aws s3 cp - "s3://$BUCKET/$PREFIX/manifest.json" \
  --content-type application/json --cache-control "public, max-age=300" >/dev/null

echo "→ invalidating CloudFront $DIST"
aws cloudfront create-invalidation --distribution-id "$DIST" \
  --paths "/$PREFIX/*" --query "Invalidation.Status" --output text

echo "done: https://downloads.clarkchat.com/$PREFIX/"
