#!/usr/bin/env python3
"""Render deterministic 20-second Clark Code ad videos from the HTML timeline."""

import argparse
import shutil
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import urlencode

from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parent
HTML = ROOT / "google-ads-video.html"
DURATION = 20
FPS = 30
CHROME = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")

VARIANTS = {
    "direct": {
        "shape": "landscape",
        "width": 1280,
        "height": 720,
        "output": "clark-code-ad-direct-vs-claude-20s-16x9.mp4",
        "poster": "clark-code-ad-direct-vs-claude-16x9-poster.png",
        "title": "Clark Code - Direct comparison",
    },
    "outgrow": {
        "shape": "square",
        "width": 1080,
        "height": 1080,
        "output": "clark-code-ad-outgrow-terminal-20s-square.mp4",
        "poster": "clark-code-ad-outgrow-terminal-square-poster.png",
        "title": "Clark Code - Outgrow the terminal",
    },
    "research": {
        "shape": "vertical",
        "width": 1080,
        "height": 1920,
        "output": "clark-code-ad-parallel-research-20s-vertical.mp4",
        "poster": "clark-code-ad-parallel-research-vertical-poster.png",
        "title": "Clark Code - Parallel research",
    },
}


def run(command):
    subprocess.run(command, check=True)


def render_variant(browser, name, config, preview_only=False):
    frame_dir = Path(tempfile.mkdtemp(prefix=f"clark-code-ad-{name}-"))
    try:
        page = browser.new_page(viewport={"width": config["width"], "height": config["height"]})
        url = HTML.as_uri() + "?" + urlencode({"variant": name, "shape": config["shape"]})
        page.goto(url, wait_until="load")
        page.wait_for_function("window.__ready === true")

        if preview_only:
            preview_dir = ROOT / "video-previews"
            preview_dir.mkdir(exist_ok=True)
            for timecode in (0.5, 3.5, 7.5, 12.0, 16.0, 18.2):
                page.evaluate("(t) => window.seek(t)", timecode)
                output = preview_dir / f"{name}-{timecode:04.1f}s.png"
                page.screenshot(path=str(output), type="png")
            print(f"[{name}] preview frames written to {preview_dir}", flush=True)
            page.close()
            return

        total = DURATION * FPS
        for index in range(total):
            page.evaluate("(t) => window.seek(t)", index / FPS)
            page.screenshot(path=str(frame_dir / f"frame-{index:05d}.png"), type="png")
            if index % 60 == 0:
                print(f"[{name}] rendered {index:03d}/{total} frames", flush=True)

        page.evaluate("(t) => window.seek(t)", 18.2)
        page.screenshot(path=str(ROOT / config["poster"]), type="png")
        page.close()

        output = ROOT / config["output"]
        scene_times = [3.0, 6.6, 10.7, 14.6, 17.0]
        frequencies = [520, 620, 720, 620, 880]
        command = [
            "ffmpeg", "-y",
            "-framerate", str(FPS),
            "-i", str(frame_dir / "frame-%05d.png"),
            "-f", "lavfi", "-i", "anoisesrc=color=pink:amplitude=0.02:duration=20:sample_rate=48000",
            "-f", "lavfi", "-i", "sine=frequency=65:duration=20:sample_rate=48000",
        ]
        for frequency in frequencies:
            command += [
                "-f", "lavfi", "-i",
                f"sine=frequency={frequency}:duration=0.20:sample_rate=48000",
            ]

        filters = [
            "[1:a]lowpass=f=420,volume=0.035[bed]",
            "[2:a]lowpass=f=180,volume=0.018[bass]",
        ]
        click_labels = []
        for offset, (timecode, _) in enumerate(zip(scene_times, frequencies), start=3):
            label = f"c{offset}"
            delay = int(timecode * 1000)
            filters.append(
                f"[{offset}:a]afade=t=out:st=0:d=0.20,volume=0.055,"
                f"adelay={delay}|{delay}[{label}]"
            )
            click_labels.append(f"[{label}]")
        audio_inputs = "[bed][bass]" + "".join(click_labels)
        filters.append(
            f"{audio_inputs}amix=inputs={2 + len(click_labels)}:normalize=0,"
            "volume=26dB,alimiter=limit=0.85,"
            "afade=t=in:st=0:d=0.7,afade=t=out:st=19:d=1[a]"
        )

        command += [
            "-filter_complex", ";".join(filters),
            "-map", "0:v:0", "-map", "[a]",
            "-t", str(DURATION),
            "-c:v", "libx264", "-preset", "slow", "-crf", "17",
            "-profile:v", "high", "-level", "4.2", "-pix_fmt", "yuv420p",
            "-r", str(FPS), "-movflags", "+faststart",
            "-c:a", "aac", "-b:a", "160k", "-ar", "48000",
            "-metadata", f"title={config['title']}",
            str(output),
        ]
        run(command)
        print(f"[{name}] wrote {output}", flush=True)
    finally:
        shutil.rmtree(frame_dir, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--preview", action="store_true", help="Render six review frames per variant")
    parser.add_argument("--only", choices=VARIANTS.keys(), help="Render one variant")
    args = parser.parse_args()
    selected = {args.only: VARIANTS[args.only]} if args.only else VARIANTS

    with sync_playwright() as playwright:
        launch_options = {"headless": True}
        if CHROME.exists():
            launch_options["executable_path"] = str(CHROME)
        browser = playwright.chromium.launch(**launch_options)
        try:
            for name, config in selected.items():
                render_variant(browser, name, config, preview_only=args.preview)
        finally:
            browser.close()


if __name__ == "__main__":
    main()
