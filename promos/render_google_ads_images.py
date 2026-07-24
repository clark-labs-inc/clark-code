#!/usr/bin/env python3
"""Render upload-ready Clark Code Google Ads images from the static HTML."""

from pathlib import Path
from urllib.parse import urlencode

from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parent
HTML = ROOT / "google-ads-static.html"
CHROME = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")

CREATIVES = {
    "direct": {
        "width": 1200,
        "height": 1200,
        "output": "clark-code-google-square-01-vs-claude.png",
    },
    "parallel": {
        "width": 1200,
        "height": 1200,
        "output": "clark-code-google-square-02-parallel-agents.png",
    },
    "memory": {
        "width": 1200,
        "height": 1200,
        "output": "clark-code-google-square-03-persistent-context.png",
    },
    "outgrow": {
        "width": 960,
        "height": 1200,
        "output": "clark-code-google-vertical-04-outgrow-terminal.png",
    },
}


def main():
    with sync_playwright() as playwright:
        options = {"headless": True}
        if CHROME.exists():
            options["executable_path"] = str(CHROME)
        browser = playwright.chromium.launch(**options)
        try:
            for variant, config in CREATIVES.items():
                page = browser.new_page(
                    viewport={"width": config["width"], "height": config["height"]},
                    device_scale_factor=1,
                )
                url = HTML.as_uri() + "?" + urlencode({"variant": variant})
                page.goto(url, wait_until="load")
                page.wait_for_function("window.__ready === true")
                output = ROOT / config["output"]
                page.screenshot(path=str(output), type="png")
                page.close()
                print(f"[{variant}] wrote {output}")
        finally:
            browser.close()


if __name__ == "__main__":
    main()
