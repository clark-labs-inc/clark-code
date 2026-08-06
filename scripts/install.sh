#!/bin/sh
set -eu

RELEASE="${CLARK_RELEASE:-latest}"
BASE_URL="${CLARK_INSTALL_BASE_URL:-https://downloads.clarkchat.com/desktop/cli}"
BIN_DIR="${CLARK_INSTALL_DIR:-$HOME/.local/bin}"
CLARK_DATA_DIR="${CLARK_HOME:-$HOME/.clark}"
PACKAGE_ROOT="$CLARK_DATA_DIR/packages/cli"
RELEASES_DIR="$PACKAGE_ROOT/releases"
CURRENT_LINK="$PACKAGE_ROOT/current"
LOCK_DIR="$PACKAGE_ROOT/install.lock.d"
TEMP_DIR=""
CURRENT_TEMP_LINK=""
CLARK_TEMP_LINK=""
WORKER_TEMP_LINK=""

step() {
  printf '==> %s\n' "$1"
}

cleanup() {
  for temporary_link in "$CURRENT_TEMP_LINK" "$CLARK_TEMP_LINK" "$WORKER_TEMP_LINK"; do
    if [ -n "$temporary_link" ] && [ -L "$temporary_link" ]; then
      rm -f "$temporary_link"
    fi
  done
  if [ -n "$TEMP_DIR" ] && [ -d "$TEMP_DIR" ]; then
    rm -rf "$TEMP_DIR"
  fi
  if [ -d "$LOCK_DIR" ]; then
    rmdir "$LOCK_DIR" 2>/dev/null || true
  fi
}
trap cleanup EXIT HUP INT TERM

download() {
  url="$1"
  output="$2"
  case "$url" in
    "$BASE_URL"/*) ;;
    *) echo "Refusing unexpected Clark download URL: $url" >&2; exit 1 ;;
  esac
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --connect-timeout 10 --max-time 300 "$url" -o "$output"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -t 1 -T 300 -O "$output" "$url"
  else
    echo "curl or wget is required to install Clark." >&2
    exit 1
  fi
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print tolower($1)}'
  else
    shasum -a 256 "$1" | awk '{print tolower($1)}'
  fi
}

validate_version() {
  if ! printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "Invalid Clark release version: $1" >&2
    exit 1
  fi
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --release)
      [ "$#" -ge 2 ] || { echo "--release requires a version" >&2; exit 2; }
      RELEASE="$2"
      shift 2
      ;;
    --help|-h)
      printf 'Usage: install.sh [--release VERSION]\n'
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Darwin/arm64|Darwin/aarch64|Darwin/x86_64)
    target="universal-apple-darwin"
    extension="zip"
    ;;
  Linux/x86_64|Linux/amd64)
    target="x86_64-unknown-linux-gnu"
    extension="tar.gz"
    ;;
  *)
    echo "Clark CLI does not yet publish a build for $os/$arch." >&2
    exit 1
    ;;
esac

mkdir -p "$PACKAGE_ROOT" "$RELEASES_DIR" "$BIN_DIR"
chmod 0700 "$PACKAGE_ROOT" "$RELEASES_DIR"
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
  echo "Another Clark installation is already running: $LOCK_DIR" >&2
  exit 1
fi

TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/clark-install.XXXXXX")"
if [ "$RELEASE" = "latest" ]; then
  download "$BASE_URL/latest/VERSION" "$TEMP_DIR/VERSION"
  version="$(tr -d '[:space:]' < "$TEMP_DIR/VERSION")"
else
  version="${RELEASE#v}"
fi
validate_version "$version"

asset="clark-$target.$extension"
release_url="$BASE_URL/releases/v$version"
step "Downloading Clark $version for $target"
download "$release_url/SHA256SUMS" "$TEMP_DIR/SHA256SUMS"
expected="$(awk -v name="$asset" '$2 == name && $1 ~ /^[0-9a-fA-F]{64}$/ { print tolower($1); exit }' "$TEMP_DIR/SHA256SUMS")"
if [ -z "$expected" ]; then
  echo "Clark release v$version has no checksum for $asset." >&2
  exit 1
fi
download "$release_url/$asset" "$TEMP_DIR/$asset"
actual="$(sha256_file "$TEMP_DIR/$asset")"
if [ "$actual" != "$expected" ]; then
  echo "Clark archive checksum mismatch: got $actual, expected $expected" >&2
  exit 1
fi

mkdir "$TEMP_DIR/unpacked"
case "$extension" in
  zip)
    command -v unzip >/dev/null 2>&1 || { echo "unzip is required to install Clark on macOS." >&2; exit 1; }
    if unzip -Z1 "$TEMP_DIR/$asset" | grep -Ev '^(bin/|bin/clark|bin/clark-code-headless)$' | grep -q .; then
      echo "Clark archive contains an unexpected path." >&2
      exit 1
    fi
    unzip -q "$TEMP_DIR/$asset" -d "$TEMP_DIR/unpacked"
    ;;
  tar.gz)
    if tar -tzf "$TEMP_DIR/$asset" | grep -Ev '^\.?/?(bin/|bin/clark|bin/clark-code-headless)$' | grep -q .; then
      echo "Clark archive contains an unexpected path." >&2
      exit 1
    fi
    tar -xzf "$TEMP_DIR/$asset" -C "$TEMP_DIR/unpacked"
    ;;
esac

for executable in clark clark-code-headless; do
  path="$TEMP_DIR/unpacked/bin/$executable"
  [ -f "$path" ] || { echo "Clark archive is missing bin/$executable" >&2; exit 1; }
  chmod 0755 "$path"
done
"$TEMP_DIR/unpacked/bin/clark" --version >/dev/null
"$TEMP_DIR/unpacked/bin/clark-code-headless" --self-test >/dev/null

release_dir="$RELEASES_DIR/$version"
if [ -e "$release_dir" ]; then
  for executable in clark clark-code-headless; do
    cmp -s "$TEMP_DIR/unpacked/bin/$executable" "$release_dir/bin/$executable" || {
      echo "Existing Clark release directory differs from verified v$version: $release_dir" >&2
      exit 1
    }
  done
else
  mv "$TEMP_DIR/unpacked" "$release_dir"
fi

if [ -e "$CURRENT_LINK" ] && [ ! -L "$CURRENT_LINK" ]; then
  echo "Refusing to replace non-symlink Clark current path: $CURRENT_LINK" >&2
  exit 1
fi
CURRENT_TEMP_LINK="$PACKAGE_ROOT/current.new.$$"
ln -s "$release_dir" "$CURRENT_TEMP_LINK"
mv -f "$CURRENT_TEMP_LINK" "$CURRENT_LINK"
CURRENT_TEMP_LINK=""

for executable in clark clark-code-headless; do
  link="$BIN_DIR/$executable"
  if [ -e "$link" ] && [ ! -L "$link" ]; then
    echo "Refusing to replace existing non-symlink command: $link" >&2
    exit 1
  fi
  if [ "$executable" = "clark" ]; then
    CLARK_TEMP_LINK="$BIN_DIR/clark.new.$$"
    temporary_link="$CLARK_TEMP_LINK"
  else
    WORKER_TEMP_LINK="$BIN_DIR/clark-code-headless.new.$$"
    temporary_link="$WORKER_TEMP_LINK"
  fi
  ln -s "$CURRENT_LINK/bin/$executable" "$temporary_link"
  mv -f "$temporary_link" "$link"
  if [ "$executable" = "clark" ]; then
    CLARK_TEMP_LINK=""
  else
    WORKER_TEMP_LINK=""
  fi
done

step "Installed Clark $version"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    printf '\nAdd Clark to this shell:\n  export PATH="%s:$PATH"\n' "$BIN_DIR"
    ;;
esac
printf 'Run `clark` for the TUI, or `clark login --device-code` over SSH.\n'
