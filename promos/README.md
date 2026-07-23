# Promos — Clark Code launch

Launch assets for the **Clark Agent vs Clark Code** distinction + the Clark Code
macOS download, mirroring the `/clark-code` page on clarkchat.com.

## Assets

| File | Use | Size |
| --- | --- | --- |
| `clark-code-twitter-1600x900.png` | Twitter / X post image | 1600×900 (16:9) |
| `clark-code-linkedin-1200x627.png` | LinkedIn post image | 1200×627 (1.91:1) |
| `twitter.md` | Twitter / X copy (primary + thread + short) | — |
| `linkedin.md` | LinkedIn copy (primary + short) | — |
| `promo-card.html` | Source for both images | viewport-driven |

## 20-second ad videos

| File | Positioning | Format |
| --- | --- | --- |
| `clark-code-ad-direct-vs-claude-20s-16x9.mp4` | Direct Claude Code comparison | 1280×720, 30 fps |
| `clark-code-ad-outgrow-terminal-20s-square.mp4` | Policy-safer terminal-agent comparison | 1080×1080, 30 fps |
| `clark-code-ad-parallel-research-20s-vertical.mp4` | Parallel bug-research differentiator | 1080×1920, 30 fps |

All three are exactly 20 seconds, use H.264 video plus AAC audio, include
sound-off-safe on-screen copy, and keep the MP4 index at the front for fast
web playback. The product frames use the current Clark Desktop UI captures in
`source-ui-*-current.png`, rather than the older simulated IDE layout.

Regenerate the videos and posters with:

```sh
python3 render_google_ads_videos.py
```

Render review frames without encoding the videos:

```sh
python3 render_google_ads_videos.py --preview
```

## Regenerating the images

The card is a single viewport-filling HTML; render it at each platform size:

```sh
playwright screenshot --viewport-size=1600,900 promo-card.html clark-code-twitter-1600x900.png
playwright screenshot --viewport-size=1200,627 promo-card.html clark-code-linkedin-1200x627.png
```

Both use the Clark Labs brand system (warm `#f7f5f1` paper, Newsreader display,
DM Sans, JetBrains Mono) and the same light-browser / dark-IDE product frames as
the `/clark-code` page — so the post, the image, and the landing page all match.

## Landing page

The page these promote: `clarkchat.com/clark-code`
(`clark/clark-ui/src/components/Public/ClarkCodePage.tsx`).
