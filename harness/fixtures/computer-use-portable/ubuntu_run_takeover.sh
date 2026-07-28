#!/usr/bin/env bash
set -u

result_prefix=/tmp/clark-cua-ubuntu-takeover
qa_root=${CLARK_QA_COMPUTER_USE_ROOT:-/tmp/clark-cua-qa}
data_dir=${CLARK_QA_COMPUTER_USE_DATA_DIR:-/tmp/clark-cua-home-data}
xauthority="$(find /run/user/1000 -maxdepth 1 -name '.mutter-Xwaylandauth.*' -print -quit)"
rm -f \
  "${result_prefix}.txt" \
  "${result_prefix}-error.txt" \
  "${result_prefix}-result.txt"

runuser -u home -- env \
  DISPLAY=:0 \
  "XAUTHORITY=${xauthority}" \
  XDG_RUNTIME_DIR=/run/user/1000 \
  DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
  WAYLAND_DISPLAY=wayland-0 \
  "CLARK_COMPUTER_USE_SERVICE_PATH=${qa_root}/clark-computer-use-helper" \
  "CLARK_COMPUTER_USE_DATA_DIR=${data_dir}" \
  "${qa_root}/portable_takeover_smoke" "Clark Computer Use QA" \
  > "${result_prefix}.txt" \
  2> "${result_prefix}-error.txt"
status=$?
printf 'exit=%s\n' "${status}" > "${result_prefix}-result.txt"
