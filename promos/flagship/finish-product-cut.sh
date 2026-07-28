#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "$0")" && pwd)"
silent_video="$root_dir/out/clark-code-product-cut-v2-silent.mp4"
audio_track="$root_dir/out/clark-code-product-cut-v2-sound-design.m4a"
master_video="$root_dir/out/clark-code-product-cut-v2-30s.mp4"

ffmpeg -y \
  -f lavfi -i "anoisesrc=color=pink:amplitude=0.0015:duration=30:sample_rate=48000" \
  -f lavfi -i "sine=frequency=920:duration=0.055:sample_rate=48000" \
  -f lavfi -i "sine=frequency=760:duration=0.07:sample_rate=48000" \
  -f lavfi -i "sine=frequency=1040:duration=0.055:sample_rate=48000" \
  -f lavfi -i "sine=frequency=620:duration=0.08:sample_rate=48000" \
  -filter_complex "\
    [0:a]highpass=f=90,lowpass=f=2400,volume=0.18[room];\
    [1:a]afade=t=out:st=0.018:d=0.037,volume=0.07,adelay=2500|2500[a1];\
    [1:a]afade=t=out:st=0.018:d=0.037,volume=0.065,adelay=6000|6000[a2];\
    [2:a]afade=t=out:st=0.025:d=0.045,volume=0.07,adelay=10000|10000[a3];\
    [1:a]afade=t=out:st=0.018:d=0.037,volume=0.065,adelay=14000|14000[a4];\
    [3:a]afade=t=out:st=0.018:d=0.037,volume=0.07,adelay=18500|18500[a5];\
    [3:a]afade=t=out:st=0.018:d=0.037,volume=0.075,adelay=23000|23000[a6];\
    [4:a]afade=t=out:st=0.03:d=0.05,volume=0.065,adelay=27000|27000[a7];\
    [3:a]afade=t=out:st=0.018:d=0.037,volume=0.055,adelay=28500|28500[a8];\
    [room][a1][a2][a3][a4][a5][a6][a7][a8]amix=inputs=9:normalize=0,\
    alimiter=limit=0.5,afade=t=in:st=0:d=0.25,afade=t=out:st=29.4:d=0.6[a]" \
  -map "[a]" -c:a aac -b:a 192k "$audio_track"

ffmpeg -y \
  -i "$silent_video" \
  -i "$audio_track" \
  -map 0:v:0 -map 1:a:0 \
  -c:v copy -c:a copy \
  -movflags +faststart \
  -shortest \
  "$master_video"

echo "$master_video"
