#!/bin/sh
set -eu

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y \
  build-essential \
  clang \
  curl \
  libegl1-mesa-dev \
  libgbm-dev \
  libpipewire-0.3-dev \
  libwayland-dev \
  libxcb1-dev \
  libxdo-dev \
  pkg-config \
  python3-tk \
  xdotool

if [ ! -x /root/.cargo/bin/cargo ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
    sh -s -- -y --profile minimal --default-toolchain stable
fi

/root/.cargo/bin/rustc --version
/root/.cargo/bin/cargo --version
