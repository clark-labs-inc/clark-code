#!/bin/sh
set -eu

ready_file=${1:-/tmp/clark-cua-ubuntu-takeover.txt}
attempts=0
while ! grep -q READY_FOR_PHYSICAL_INPUT "$ready_file" 2>/dev/null; do
  attempts=$((attempts + 1))
  if [ "$attempts" -ge 1000 ]; then
    echo "takeover smoke did not become ready" >&2
    exit 1
  fi
  sleep 0.01
done

xauthority="$(find /run/user/1000 -maxdepth 1 -name '.mutter-Xwaylandauth.*' -print -quit)"
runuser -u home -- env \
  DISPLAY=:0 \
  "XAUTHORITY=${xauthority}" \
  xdotool key a
