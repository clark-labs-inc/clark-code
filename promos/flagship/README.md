# Clark Code product film

30-second, 1920×1080 product cut built from a deterministic recording of the
real Clark Code interface. The scenario is a scripted recreation, not a paid
live-model run: a Windows-only reconnect failure is reproduced, investigated
in parallel, corrected after the first theory fails, and verified locally and
on a remote runner.

## Deliverables

- `out/clark-code-product-cut-v2-30s.mp4` — H.264/AAC upload master
- `out/poster-v2.png` — 1920×1080 poster
- `public/clark-ui-flagship.webm` — unedited real-UI screen recording

## Rebuild

Start the browser preview from `app/`:

```sh
pnpm dev --host 127.0.0.1
```

Capture the scripted product take:

```sh
node promos/flagship/capture-ui.mjs
```

Convert the take to the 30 fps source expected by Remotion, then extract the
two held frames:

```sh
ffmpeg -y -i promos/flagship/public/clark-ui-flagship.webm \
  -vf fps=30 -an -c:v libx264 -crf 16 -pix_fmt yuv420p \
  promos/flagship/public/clark-ui-flagship.mp4

ffmpeg -y -ss 29.1 -i promos/flagship/public/clark-ui-flagship.mp4 \
  -frames:v 1 promos/flagship/public/final-proof.png

ffmpeg -y -ss 4.6 -i promos/flagship/public/clark-ui-flagship.mp4 \
  -frames:v 1 promos/flagship/public/problem.png
```

Render and finish:

```sh
cd promos/flagship
pnpm install
pnpm render:v2
pnpm still:v2
bash finish-product-cut.sh
```

The restrained interface sound design is generated locally, so the master has
no third-party music or sound-effect licensing dependency.
