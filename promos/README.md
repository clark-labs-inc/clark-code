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
