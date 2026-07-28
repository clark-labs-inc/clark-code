#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
silent="$here/out/clark-code-flagship-silent.mp4"
soundtrack="$here/out/clark-code-flagship-soundtrack.wav"
final="$here/out/clark-code-flagship-42s-16x9.mp4"

mkdir -p "$here/out"

ffmpeg -y -v error \
  -f lavfi -i "anoisesrc=color=pink:amplitude=0.018:duration=42.05:sample_rate=48000" \
  -f lavfi -i "sine=frequency=55:duration=42.05:sample_rate=48000" \
  -f lavfi -i "sine=frequency=110:duration=42.05:sample_rate=48000" \
  -f lavfi -i "sine=frequency=420:duration=0.34:sample_rate=48000" \
  -f lavfi -i "sine=frequency=520:duration=0.34:sample_rate=48000" \
  -f lavfi -i "sine=frequency=610:duration=0.34:sample_rate=48000" \
  -f lavfi -i "sine=frequency=470:duration=0.34:sample_rate=48000" \
  -f lavfi -i "sine=frequency=360:duration=0.42:sample_rate=48000" \
  -f lavfi -i "sine=frequency=680:duration=0.34:sample_rate=48000" \
  -f lavfi -i "sine=frequency=820:duration=0.42:sample_rate=48000" \
  -f lavfi -i "anoisesrc=color=white:amplitude=0.15:duration=0.65:sample_rate=48000" \
  -filter_complex "
    [0:a]lowpass=f=520,highpass=f=45,volume=0.8,afade=t=in:st=0:d=1.4,afade=t=out:st=40.6:d=1.4[bed];
    [1:a]lowpass=f=105,volume=0.10,tremolo=f=0.11:d=0.42,afade=t=in:st=0:d=2.2,afade=t=out:st=40:d=2[sub];
    [2:a]highpass=f=85,lowpass=f=700,volume=0.018,tremolo=f=0.17:d=0.55,afade=t=in:st=1:d=3,afade=t=out:st=39:d=3[air];
    [3:a]afade=t=out:st=0:d=0.34,volume=0.10,adelay=2800|2800[h1];
    [4:a]afade=t=out:st=0:d=0.34,volume=0.10,adelay=6800|6800[h2];
    [5:a]afade=t=out:st=0:d=0.34,volume=0.10,adelay=12800|12800[h3];
    [6:a]afade=t=out:st=0:d=0.34,volume=0.10,adelay=19800|19800[h4];
    [7:a]afade=t=out:st=0:d=0.42,volume=0.12,adelay=22800|22800[h5];
    [8:a]afade=t=out:st=0:d=0.34,volume=0.10,adelay=29800|29800[h6];
    [9:a]afade=t=out:st=0:d=0.42,volume=0.12,adelay=36800|36800[h7];
    [10:a]highpass=f=850,lowpass=f=4200,afade=t=in:st=0:d=0.08,afade=t=out:st=0.1:d=0.55,volume=0.10,adelay=19700|19700[whoosh];
    [bed][sub][air][h1][h2][h3][h4][h5][h6][h7][whoosh]
      amix=inputs=11:normalize=0,volume=30dB,alimiter=limit=0.82:level=false[out]
  " \
  -map "[out]" -c:a pcm_s16le -ar 48000 "$soundtrack"

ffmpeg -y -v error \
  -i "$silent" \
  -i "$soundtrack" \
  -map 0:v:0 -map 1:a:0 \
  -c:v copy \
  -c:a aac -b:a 192k -ar 48000 \
  -t 42 \
  -movflags +faststart \
  -metadata title="Clark Code — Agent work you can see and trust" \
  "$final"

echo "$final"
